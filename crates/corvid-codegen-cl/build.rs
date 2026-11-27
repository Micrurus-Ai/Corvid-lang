//! Identify the native backend that produced a cached executable: a stamp
//! of the code generator's sources and of every workspace crate it builds
//! from, the runtime included (see `corvid-source-stamp`). An edit to any
//! of them changes the id, so the native cache never reuses a binary
//! built by other code.

use std::path::PathBuf;

fn main() -> std::io::Result<()> {
    println!("cargo:rerun-if-changed=build.rs");
    let out_dir = PathBuf::from(std::env::var_os("OUT_DIR").expect("Cargo output directory"));
    std::fs::write(out_dir.join("native-backend-id.txt"), corvid_source_stamp::closure_stamp())
}
