//! A temporary project whose `src/std/` vendors standard-library
//! modules, so a test program's `import "./std/..."` resolves through
//! the normal module pipeline.

#![allow(dead_code)]

use corvid_driver::compile_to_ir_with_config_at_path;
use std::path::Path;

/// Compiles `source` as `src/main.cor` of a project that vendors the
/// named `std/` files.
pub fn compile_with_std(source: &str, std_files: &[&str]) -> corvid_ir::IrFile {
    let repo = Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).expect("repo root");
    let project = tempfile::tempdir().expect("tempdir");
    let std_dir = project.path().join("src").join("std");
    std::fs::create_dir_all(&std_dir).unwrap();
    for file in std_files {
        std::fs::copy(repo.join("std").join(file), std_dir.join(file)).unwrap();
    }
    let main_path = project.path().join("src").join("main.cor");
    std::fs::write(&main_path, source).unwrap();
    compile_to_ir_with_config_at_path(source, &main_path, None).expect("the program compiles")
}
