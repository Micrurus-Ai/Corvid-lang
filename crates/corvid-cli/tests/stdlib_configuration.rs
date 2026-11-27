use std::{fs, process::Command};

#[test]
fn project_reads_configuration_and_round_trips_a_json_file() {
    let dir = tempfile::tempdir().unwrap();
    let cli = env!("CARGO_BIN_EXE_corvid");
    let created = Command::new(cli)
        .current_dir(dir.path())
        .args(["new", "config"])
        .output()
        .unwrap();
    assert!(created.status.success(), "{created:?}");
    let project = dir.path().join("config");
    fs::write(project.join("src/main.cor"), r#"import "std:env" use env_get
import "std:io" use io_read_text, io_write_text, io_list_dir
import "std:json" use json_parse, json_get_int, json_object_new, json_object_set_int, json_object_finish
import "std:time" use time_parse_iso, time_format_iso
import "std:path" use path_join, path_filename, path_parent, path_extension, path_is_absolute

agent main() -> Result<Bool, String>:
    value = env_get("CORVID_TEST_MODE")?
    missing = env_get("CORVID_TEST_ABSENT")?
    empty = env_get("CORVID_TEST_EMPTY")?
    config_ok = value == Some("local") and missing == None and empty == Some("")
    names_ok = env_get("a=b").is_err() and env_get("a\0b").is_err() and env_get("").is_err()
    text = json_object_finish(json_object_set_int(json_object_new(), "count", 4))
    file = path_join("data", "café.json")
    parts_ok = path_filename(file) == Some("café.json") and path_parent(file) == Some("data") and path_extension(file) == Some("json")
    paths_ok = parts_ok and not path_is_absolute(file) and path_extension(".profile") == None and path_parent("") == None
    written = io_write_text(file, text)?
    loaded = io_read_text(file)?
    entries = io_list_dir(".")?
    data = json_parse(loaded.contents)?
    count = json_get_int(data, "count")?
    zero = time_parse_iso(time_format_iso(0))?
    files_ok = written.bytes == loaded.bytes and entries.length() > 0 and io_read_text("missing").is_err()
    errors_ok = json_parse("{").is_err() and json_get_int(data, "absent").is_err() and time_parse_iso("invalid").is_err()
    return Ok(config_ok and names_ok and paths_ok and files_ok and errors_ok and count == 4 and zero == 0)
"#).unwrap();
    let output = Command::new(cli)
        .current_dir(&project)
        .arg("run")
        .env("CORVID_TEST_MODE", "local")
        .env("CORVID_TEST_EMPTY", "")
        .env_remove("CORVID_TEST_ABSENT")
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), r#"{"ok":true,"tag":"ok"}"#);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(
            &fs::read(project.join("data/café.json")).unwrap()
        )
        .unwrap(),
        serde_json::json!({"count": 4})
    );
}
