//! Corvid proc-macros.
//!
//! Only surface today is `#[tool("name")]`. Applied to an `async fn` in
//! a user Rust crate, it generates the typed C-ABI bridge that
//! Cranelift-compiled Corvid code links against.
//!
//! # What the macro does
//!
//! Given:
//!
//! ```ignore
//! #[tool("get_order")]
//! async fn get_order(id: String) -> Order { ... }
//! ```
//!
//! it emits:
//!
//! 1. The user's `async fn` unchanged — other Rust code keeps calling
//!    it the normal way (used by the interpreter tier, by tests, and
//!    by any Rust code that happens to live in the same crate).
//! 2. A `#[no_mangle] pub extern "C" fn __corvid_tool_get_order(...)`
//!    whose signature uses `#[repr(C)]` Corvid ABI types (e.g.
//!    `CorvidString` in place of `String`, plain `i64` in place of an
//!    `i64` Corvid Int). The wrapper converts args into native Rust
//!    types, calls the user's `async fn` through `block_on` on the
//!    runtime's tokio handle, and converts the result back out.
//! 3. An `inventory::submit!` block registering a `ToolMetadata` entry
//!    so the runtime can build its effect-policy table and tracer
//!    registry at startup.
//!
//! # What the macro does NOT do
//!
//! - It doesn't decide which Rust types map to which Corvid ABI types
//!   — that mapping lives in `corvid-runtime::abi` and is shared with
//!   the codegen side. The macro just calls the conversion traits.
//! - It doesn't do error handling beyond what the user's `async fn`
//!   already does. Tools whose `async fn` returns `T` cannot fail at
//!   the Corvid level today — the macro does not support a
//!   `Result<T, E>` return path yet.
//! - It doesn't support sync `fn` (only `async fn`). Users wrap a
//!   sync body in `async { ... }` trivially; keeping the macro
//!   async-only means one codepath to test and maintain.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as TokenStream2;
use quote::{format_ident, quote};
use syn::{parse_macro_input, FnArg, ItemFn, LitStr, Pat, ReturnType, Type};

/// `#[tool("name")]` — mark an async fn as a Corvid tool implementation.
///
/// The string argument is the name the Corvid declaration references.
/// It does NOT have to match the Rust fn name — users are free to call
/// the Rust fn `get_order_impl` and register it for Corvid's
/// `get_order` — but keeping them aligned is recommended.
#[proc_macro_attribute]
pub fn tool(attr: TokenStream, item: TokenStream) -> TokenStream {
    let name_lit = parse_macro_input!(attr as LitStr);
    let fn_item = parse_macro_input!(item as ItemFn);
    match expand_tool(name_lit, fn_item) {
        Ok(tokens) => tokens.into(),
        Err(err) => err.to_compile_error().into(),
    }
}

fn expand_tool(name_lit: LitStr, f: ItemFn) -> syn::Result<TokenStream2> {
    // `#[tool]` only accepts `async fn`. Sync fns
    // are wrappable in `async { ... }` trivially — rejecting them here
    // prevents accidental "my tool isn't async so it can't await the
    // LLM" foot-guns down the line.
    if f.sig.asyncness.is_none() {
        return Err(syn::Error::new_spanned(
            &f.sig.fn_token,
            "#[tool] requires `async fn` — wrap synchronous bodies in `async { ... }` if you don't need to await anything",
        ));
    }

    let tool_name = name_lit.value();
    let fn_ident = f.sig.ident.clone();
    // Wrapper symbol = `__corvid_tool_<tool-name>`. Using the declared
    // tool name (not the Rust fn name) keeps the linker lookup aligned
    // with Corvid source — codegen emits `call "__corvid_tool_<name>"`.
    let wrapper_ident = format_ident!(
        "__corvid_tool_{}",
        mangle_tool_name(&tool_name)
    );
    let wrapper_symbol_string = wrapper_ident.to_string();

    // Decide up front whether the typed C-ABI wrapper is emittable.
    //
    // The typed wrapper takes / returns one Corvid ABI primitive per
    // arg / return — i64 / f64 / bool / `CorvidString` — and codegen
    // direct-calls `__corvid_tool_<name>` for native-binary targets.
    // Struct (and list) arguments cannot be expressed at the scalar
    // C ABI — their layout is per-type and the wrapper would have to
    // encode/decode a JSON or refcount envelope, which is exactly
    // what the JSON wrapper below already does. So when ANY arg or
    // the return type is non-scalar, the typed wrapper is omitted
    // and the tool is reachable only through the JSON wrapper /
    // runtime registry — the cdylib dispatch path that
    // `35V2-P42-G0-tools-2b` made target-conditional. Native-binary
    // targets that try to direct-call a struct-signature tool will
    // get a clean linker error (the symbol isn't emitted) rather
    // than the wrong-ABI miscompilation that would result from
    // forcing a scalar wrapper around a struct value.
    let emit_typed_wrapper = signature_is_all_scalar(&f.sig)?;

    // Collect (native_type, abi_type, arg_name) for each parameter.
    let mut wrapper_params: Vec<TokenStream2> = Vec::new();
    let mut arg_conversions: Vec<TokenStream2> = Vec::new();
    let mut call_args: Vec<TokenStream2> = Vec::new();
    // (arg name, native type) pairs for the JSON-dispatch wrapper, which
    // deserializes each arg from the JSON args array via serde rather
    // than the typed C ABI.
    let mut json_args: Vec<(syn::Ident, Type)> = Vec::new();

    for (idx, input) in f.sig.inputs.iter().enumerate() {
        let (arg_name, native_ty) = match input {
            FnArg::Receiver(r) => {
                return Err(syn::Error::new_spanned(
                    r,
                    "#[tool] fns can't take `self` — they're free functions, not methods",
                ));
            }
            FnArg::Typed(pt) => {
                let name = match &*pt.pat {
                    Pat::Ident(i) => i.ident.clone(),
                    other => {
                        return Err(syn::Error::new_spanned(
                            other,
                            "#[tool] fn parameters must be plain identifiers (not patterns)",
                        ));
                    }
                };
                let ty = (*pt.ty).clone();
                (name, ty)
            }
        };

        if emit_typed_wrapper {
            // Scalar/String path — typed C-ABI wrapper compiles, so set
            // up its arg encoding + body conversion.
            let abi_ty = abi_type_for(&native_ty)?;
            let native_ty_tokens = quote! { #native_ty };
            wrapper_params.push(quote! { #arg_name: #abi_ty });

            // Conversion: every Corvid ABI type implements
            // `FromCorvidAbi` (defined in `corvid_runtime::abi`). The
            // conversion may copy (e.g. CorvidString -> String copies
            // bytes) and may release refcounts (CorvidString into-
            // conversion releases the caller's +0 reference — per the
            // +0 ABI the wrapper retains on entry and releases here).
            arg_conversions.push(quote! {
                let #arg_name: #native_ty_tokens =
                    ::corvid_runtime::abi::FromCorvidAbi::from_corvid_abi(#arg_name);
            });
            call_args.push(quote! { #arg_name });
        }
        json_args.push((arg_name.clone(), native_ty.clone()));

        // `idx` unused today; kept to make future arity-diagnostics
        // straightforward.
        let _ = idx;
    }

    // Return type: `async fn` expands to a Future whose Output is the
    // declared return. Extract the Output type (or `()` for no return).
    let (return_is_unit, native_ret_ty) = match &f.sig.output {
        ReturnType::Default => (true, None),
        ReturnType::Type(_, ty) => (false, Some((**ty).clone())),
    };

    let wrapper_ret_ty = if emit_typed_wrapper {
        match &native_ret_ty {
            None => quote! { () },
            Some(ty) => {
                let abi = abi_type_for(ty)?;
                quote! { #abi }
            }
        }
    } else {
        // Unused when the typed wrapper is omitted; placeholder.
        quote! { () }
    };

    let arity = f.sig.inputs.len();

    let call_expr = if return_is_unit {
        quote! {
            __handle.block_on(async { #fn_ident(#(#call_args),*).await });
        }
    } else {
        quote! {
            let __result = __handle.block_on(async { #fn_ident(#(#call_args),*).await });
            ::corvid_runtime::abi::IntoCorvidAbi::into_corvid_abi(__result)
        }
    };

    // JSON-dispatch wrapper: the call args arrive as a JSON array, each
    // is deserialized to its native type via serde, and the result is
    // serialized back to JSON. This is the callable the runtime
    // registers into the tool registry at init (G0-tools-3) so codegen
    // can dispatch a tool call through `corvid_invoke_tool_*` instead of
    // the link-time `__corvid_tool_<name>` symbol — the same mechanism a
    // host uses to provide tools to an embedded cdylib.
    let json_wrapper_ident = format_ident!("__corvid_tool_json_{}", mangle_tool_name(&tool_name));
    let json_arg_decodes: Vec<TokenStream2> = json_args
        .iter()
        .enumerate()
        .map(|(idx, (arg_name, native_ty))| {
            quote! {
                let #arg_name: #native_ty = ::corvid_runtime::serde_json::from_value(
                    __args_arr
                        .get(#idx)
                        .cloned()
                        .unwrap_or(::corvid_runtime::serde_json::Value::Null),
                )
                .expect(concat!(
                    "#[tool] `", #tool_name,
                    "`: failed to deserialize argument `", stringify!(#arg_name), "` from JSON"
                ));
            }
        })
        .collect();
    let json_call_args: Vec<TokenStream2> = json_args.iter().map(|(n, _)| quote! { #n }).collect();
    let json_result = if return_is_unit {
        quote! {
            __handle.block_on(async { #fn_ident(#(#json_call_args),*).await });
            ::corvid_runtime::serde_json::Value::Null
        }
    } else {
        quote! {
            let __result = __handle.block_on(async { #fn_ident(#(#json_call_args),*).await });
            ::corvid_runtime::serde_json::to_value(&__result).expect(concat!(
                "#[tool] `", #tool_name, "`: failed to serialize result to JSON"
            ))
        }
    };

    // Typed C-ABI wrapper. Linker-visible symbol name is the literal
    // `__corvid_tool_<mangled-name>`; codegen emits a direct call to
    // this symbol on native-binary targets. Omitted entirely when the
    // signature has any non-scalar (struct / list / custom) arg or
    // return type — those tools dispatch only through the JSON
    // wrapper / runtime registry under the cdylib path.
    let typed_wrapper_tokens: TokenStream2 = if emit_typed_wrapper {
        quote! {
            #[no_mangle]
            pub extern "C" fn #wrapper_ident(#(#wrapper_params),*) -> #wrapper_ret_ty {
                // Grab the tokio handle. Panics if `corvid_runtime_init`
                // hasn't run — contract is that main calls it first
                // whenever `ir_uses_runtime(ir)` returned true.
                let __handle = ::corvid_runtime::ffi_bridge::tokio_handle();
                #(#arg_conversions)*
                #call_expr
            }
        }
    } else {
        TokenStream2::new()
    };

    // `symbol` in the inventory entry names the typed wrapper for
    // direct dispatch. When the typed wrapper is omitted (struct
    // signatures), the empty-string marker signals "no typed
    // wrapper exists; route only through `json_dispatch`."
    let inventory_symbol_lit: TokenStream2 = if emit_typed_wrapper {
        let s = wrapper_symbol_string.clone();
        quote! { #s }
    } else {
        quote! { "" }
    };

    let expanded = quote! {
        // 1. The user's async fn, unchanged.
        #f

        // 2. Typed C-ABI wrapper (only when every arg + return is
        //    scalar/String — see `emit_typed_wrapper` for the rule).
        #typed_wrapper_tokens

        // 2b. JSON-dispatch wrapper (registered into the tool registry
        //     at init; see `ToolMetadata.json_dispatch`).
        #[no_mangle]
        pub unsafe extern "C" fn #json_wrapper_ident(
            __args_ptr: *const ::std::ffi::c_char,
            __args_len: usize,
            _user_data: *mut ::std::ffi::c_void,
        ) -> *mut ::std::ffi::c_char {
            let __handle = ::corvid_runtime::ffi_bridge::tokio_handle();
            let __bytes = ::std::slice::from_raw_parts(__args_ptr as *const u8, __args_len);
            let __args_arr: ::std::vec::Vec<::corvid_runtime::serde_json::Value> =
                ::corvid_runtime::serde_json::from_slice::<::corvid_runtime::serde_json::Value>(__bytes)
                    .ok()
                    .and_then(|v| match v {
                        ::corvid_runtime::serde_json::Value::Array(items) => ::std::option::Option::Some(items),
                        _ => ::std::option::Option::None,
                    })
                    .unwrap_or_default();
            #(#json_arg_decodes)*
            let __result_value = { #json_result };
            let __result_json = ::corvid_runtime::serde_json::to_string(&__result_value)
                .unwrap_or_else(|_| ::std::string::String::from("null"));
            ::std::ffi::CString::new(__result_json)
                .expect("#[tool] result JSON contained an interior NUL")
                .into_raw()
        }

        // 3. Metadata registration. `corvid_runtime_init` collects
        //    every entry at startup to build the effect-policy table
        //    and to self-register each tool's `json_dispatch` into the
        //    runtime tool registry. Never on the dispatch hot path.
        ::corvid_runtime::inventory::submit! {
            ::corvid_runtime::ToolMetadata {
                name: #tool_name,
                symbol: #inventory_symbol_lit,
                arity: #arity,
                json_dispatch: #json_wrapper_ident,
            }
        }
    };

    Ok(expanded)
}

/// Return `true` when every parameter and the return type fits the
/// scalar set the typed C-ABI wrapper can express (`i64` / `f64` /
/// `bool` / `String`), `false` otherwise. The boundary is structural,
/// not user-facing: a `false` result tells `expand_tool` to omit the
/// typed wrapper and emit only the JSON wrapper + inventory entry
/// (the cdylib registry path). Returns `Err` only on a malformed
/// signature shape `expand_tool` would have rejected anyway (`self`
/// receiver / non-identifier pattern); the type-vocabulary check is
/// pure inspection and cannot fail.
fn signature_is_all_scalar(sig: &syn::Signature) -> syn::Result<bool> {
    for input in sig.inputs.iter() {
        match input {
            FnArg::Receiver(_) | FnArg::Typed(_) => {}
        }
        let ty = match input {
            FnArg::Typed(pt) => &*pt.ty,
            FnArg::Receiver(_) => continue,
        };
        if !is_scalar_abi_type(ty) {
            return Ok(false);
        }
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        if !is_scalar_abi_type(ty) {
            return Ok(false);
        }
    }
    Ok(true)
}

/// True iff `ty` is one of the four types the typed C-ABI wrapper can
/// represent: `i64` (Corvid Int), `f64` (Corvid Float), `bool` (Corvid
/// Bool), `String` (Corvid String via `CorvidString` ABI). Anything
/// else — `Vec<_>`, `Option<_>`, user structs, fully-qualified paths,
/// references — falls back to the JSON wrapper. Same vocabulary that
/// `abi_type_for` accepts; kept as a separate predicate so callers can
/// branch without provoking a syn::Error.
fn is_scalar_abi_type(ty: &Type) -> bool {
    let ts = quote! { #ty }.to_string().replace(' ', "");
    matches!(ts.as_str(), "i64" | "f64" | "bool" | "String")
}

/// Map a Rust type appearing in a `#[tool]` signature to its Corvid
/// **typed C-ABI** type. Only ever called on signatures
/// `signature_is_all_scalar` already approved, so the `Err` branch
/// would represent a genuine macro bug rather than a user-error
/// surface. Kept as an explicit error rather than `unreachable!()`
/// so future contributors who add an arm to `is_scalar_abi_type`
/// without updating this function get a compile error pointing at
/// the missed arm, not a runtime panic.
///
/// Struct/list/custom signatures are not unreachable through `#[tool]`
/// — they're supported, but route only through the JSON wrapper. See
/// `signature_is_all_scalar` for the dispatch-shape rule.
fn abi_type_for(ty: &Type) -> syn::Result<TokenStream2> {
    let ts = quote! { #ty }.to_string().replace(' ', "");
    match ts.as_str() {
        "i64" => Ok(quote! { i64 }),
        "f64" => Ok(quote! { f64 }),
        "bool" => Ok(quote! { bool }),
        "String" => Ok(quote! { ::corvid_runtime::abi::CorvidString }),
        other => Err(syn::Error::new_spanned(
            ty,
            format!(
                "internal #[tool] macro error: `abi_type_for` reached `{other}` even though `signature_is_all_scalar` filtered it out. Add the matching arm to `is_scalar_abi_type` (and this function) or report this as a `#[tool]` bug."
            ),
        )),
    }
}

/// The wrapper symbol name embeds the tool name. Tool names that aren't
/// valid C identifiers get their non-alphanumeric chars replaced with
/// underscores. Tool names are typically snake_case identifiers
/// anyway; mangling exists for robustness, not because anyone writes
/// a tool named `"with spaces!"`.
fn mangle_tool_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect()
}
