use std::{env, fs, path::PathBuf};

fn main() {
    let stdlib = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest directory")).join("../../std");
    println!("cargo:rerun-if-changed={}", stdlib.display());
    let mut modules: Vec<_> = fs::read_dir(&stdlib)
        .expect("read Corvid standard library")
        .map(|entry| entry.expect("read standard library entry").path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "cor"))
        .collect();
    modules.sort();
    let mut contents = String::from("&[\n");
    for path in modules {
        let name = path
            .file_name()
            .unwrap()
            .to_str()
            .expect("UTF-8 module name");
        contents.push_str(&format!(
            "    ({name:?}, include_bytes!({path:?}) as &[u8]),\n"
        ));
    }
    contents.push_str("]\n");
    let output = PathBuf::from(env::var_os("OUT_DIR").unwrap()).join("bundled_std.rs");
    fs::write(output, contents).expect("write embedded standard library");
}
