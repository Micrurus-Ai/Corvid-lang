//! Compile the C runtime files (`runtime/*.c`) into corvid-runtime's
//! library artifact. This makes corvid-runtime self-contained — every
//! `extern "C"` reference its abi + ffi_bridge modules make to the C
//! helpers (`corvid_alloc`, `corvid_release`, `corvid_string_from_bytes`,
//! `corvid_runtime_overflow`, etc.) resolves at link time of any binary
//! that depends on corvid-runtime, including Rust test binaries that
//! never touch the native-codegen pipeline.
//!
//! Without this, link errors like
//!   `unresolved external symbol corvid_string_from_bytes`
//! surface in cargo-test for any crate that depends on corvid-runtime,
//! because Rust extern "C" declarations don't synthesize implementations.
//!
//! The C files were previously compiled by `corvid-codegen-cl`'s
//! `link.rs` at user-binary link time. The current layout moves the compilation
//! here so corvid-codegen-cl just links against corvid-runtime's
//! staticlib (which already contains the C objects).

fn main() {
    let runtime_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("runtime");

    let mut build = cc::Build::new();
    build
        .file(runtime_dir.join("alloc.c"))
        .file(runtime_dir.join("strings.c"))
        .file(runtime_dir.join("lists.c"))
        .file(runtime_dir.join("entry.c"))
        .file(runtime_dir.join("shim.c"))
        .file(runtime_dir.join("weak.c"))
        .file(runtime_dir.join("stack_maps.c"))
        .file(runtime_dir.join("stack_maps_fallback.c"))
        .file(runtime_dir.join("collector.c"))
        .file(runtime_dir.join("verify.c"))
        .file(runtime_dir.join("json.c"))
        .opt_level(2);

    // C standard: C11 kept for designated initializers in static
    // typeinfo blocks (corvid_typeinfo_String in alloc.c).
    if cc::Build::new().get_compiler().is_like_msvc() {
        build.flag("/std:c11");
    } else {
        build.flag("-std=c11");
    }

    build.compile("corvid_c_runtime");

    // Historically this script wrote `OUT_DIR/c_runtime_path.rs`
    // containing `pub const C_RUNTIME_LIB_PATH: &str = "<absolute
    // OUT_DIR>/libcorvid_c_runtime.a"`. That constant got embedded
    // into the resulting binary's `.rodata` and broke bit-identical
    // rebuilds: two builds with different `CARGO_TARGET_DIR`
    // values (the `reproducible-build.yml` workflow uses
    // `target-build-1` and `target-build-2` on purpose) ended up
    // with two different absolute paths in their compiled output,
    // differing by exactly one byte. `--remap-path-prefix` only
    // remaps rustc's internal path tracking (debuginfo, error
    // messages, `file!()` macro expansion) — it does NOT rewrite
    // string literals in build-script-generated source files.
    //
    // The constant has been retired. Downstream consumers now
    // either (a) hash the C source files via `include_bytes!` at
    // their own compile time for cache-invalidation (e.g.
    // `corvid-driver::native_cache`), or (b) discover the
    // staticlib at runtime via `corvid-codegen-cl`'s
    // `staticlib_discovery` walk-up (e.g.
    // `corvid-codegen-cl/tests/ffi_bridge_smoke.rs`). Both
    // patterns produce byte-identical binaries across
    // `CARGO_TARGET_DIR` choices.

    // Cargo rebuilds when any C source changes.
    println!("cargo:rerun-if-changed=runtime/alloc.c");
    println!("cargo:rerun-if-changed=runtime/strings.c");
    println!("cargo:rerun-if-changed=runtime/lists.c");
    println!("cargo:rerun-if-changed=runtime/entry.c");
    println!("cargo:rerun-if-changed=runtime/shim.c");
    println!("cargo:rerun-if-changed=runtime/weak.c");
    println!("cargo:rerun-if-changed=runtime/stack_maps.c");
    println!("cargo:rerun-if-changed=runtime/stack_maps_fallback.c");
    println!("cargo:rerun-if-changed=runtime/collector.c");
    println!("cargo:rerun-if-changed=runtime/verify.c");
    println!("cargo:rerun-if-changed=runtime/json.c");
    println!("cargo:rerun-if-changed=build.rs");
}
