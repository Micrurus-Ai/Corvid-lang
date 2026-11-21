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

// These are also inputs to the native executable cache in the driver.
const C_SOURCES: &[&str] = &[
    "alloc.c",
    "strings.c",
    "lists.c",
    "ordered_map.c",
    "entry.c",
    "shim.c",
    "weak.c",
    "stack_maps.c",
    "stack_maps_fallback.c",
    "collector.c",
    "verify.c",
    "json.c",
];
const C_HEADERS: &[&str] = &["list_layout.h"];

fn main() {
    let mut build = cc::Build::new();
    for name in C_SOURCES {
        build.file(std::path::Path::new("runtime").join(name));
    }
    build.opt_level(2);

    // C standard: C11 kept for designated initializers in static
    // typeinfo blocks (corvid_typeinfo_String in alloc.c).
    if cc::Build::new().get_compiler().is_like_msvc() {
        build.flag("/std:c11");
    } else {
        build.flag("-std=c11");
    }

    build.compile("corvid_c_runtime");

    // The generated expression embeds file contents only. The final
    // executable must be reproducible across checkout and target directories.
    let mut inputs = String::from("&[\n");
    for name in C_SOURCES.iter().chain(C_HEADERS) {
        println!("cargo:rerun-if-changed=runtime/{name}");
        inputs.push_str(&format!(
            "include_bytes!(concat!(env!(\"CARGO_MANIFEST_DIR\"), \"/runtime/{name}\")),\n"
        ));
    }
    inputs.push(']');
    let out = std::path::PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo OUT_DIR"));
    std::fs::write(out.join("native_runtime_inputs.rs"), inputs)
        .expect("write native cache inputs");
    println!("cargo:rerun-if-changed=build.rs");
}
