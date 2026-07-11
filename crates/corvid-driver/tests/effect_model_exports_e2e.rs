//! Slice 45o — effect + model exports across `.cor` module
//! boundaries.
//!
//! Pinned behavior:
//! 1. `public effect` / `public model` are importable via `use`,
//!    and a tool in the importing file can write `uses
//!    <imported_effect>` — the imported effect joins the importing
//!    file's effect registry (composition sees its dimensions).
//! 2. A PRIVATE effect in the same module stays unimportable
//!    (UnknownImportMember).
//! 3. An imported model name is a valid route target.

use corvid_driver::compile_to_ir_with_config_at_path;
use std::fs;

fn write_project(shared: &str, main: &str) -> (tempfile::TempDir, std::path::PathBuf, String) {
    let project = tempfile::tempdir().expect("tempdir");
    fs::create_dir_all(project.path().join("src")).unwrap();
    fs::write(project.path().join("src").join("shared.cor"), shared).unwrap();
    let main_path = project.path().join("src").join("main.cor");
    fs::write(&main_path, main).unwrap();
    (project, main_path, main.to_string())
}

const SHARED: &str = r#"
public effect team_llm:
    cost: $0.01
    reversible: true

public model team_model:
    capability: expert

effect private_eff:
    reversible: true
"#;

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imported_public_effect_and_model_resolve() {
    let main = r#"
import "./shared" use team_llm, team_model

public tool fetch_summary(id: String) -> String uses team_llm

prompt summarize(text: String) -> String:
    route:
        true -> team_model
    "Summarize {text}"

agent main() -> String:
    return "EFFECT MODEL EXPORTS WORK"
"#;
    let (_project, main_path, source) = write_project(SHARED, main);
    let result = compile_to_ir_with_config_at_path(&source, &main_path, None);
    assert!(
        result.is_ok(),
        "importing a public effect + model must compile: {:?}",
        result.err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn private_effect_stays_unimportable() {
    let main = r#"
import "./shared" use private_eff

agent main() -> Int:
    return 0
"#;
    let (_project, main_path, source) = write_project(SHARED, main);
    let result = compile_to_ir_with_config_at_path(&source, &main_path, None);
    assert!(
        result.is_err(),
        "a private effect must NOT be importable"
    );
}
