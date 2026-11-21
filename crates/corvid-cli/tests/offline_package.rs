use sha2::{Digest, Sha256};
use std::{fs, process::Command};

#[test]
fn locked_local_package_rebuilds_offline_and_checks_cached_bytes() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("app");
    let registry = tmp.path().join("registry");
    fs::create_dir_all(root.join("src")).unwrap();
    fs::create_dir(&registry).unwrap();
    fs::write(
        root.join("corvid.toml"),
        "name = \"app\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    let source = "public fn triple(n: Int) -> Int:\n    return n * 3\n";
    let digest = format!("{:x}", Sha256::digest(source.as_bytes()));
    let package = "@example/math";
    let version = "1.0.0";
    fs::write(registry.join("source.cor"), source).unwrap();
    let index = serde_json::json!({"packages": { (package): {"versions": {
        (version): {"name": package, "version": version, "url": "source.cor", "sha256": digest}
    }}}});
    fs::write(registry.join("index.json"), index.to_string()).unwrap();
    fs::write(root.join("src/main.cor"), "import \"corvid://@example/math/v1.0.0\" use triple\nagent main() -> Int:\n    return triple(4)\n").unwrap();
    let cli = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(&root)
            .env("CORVID_OFFLINE", "1")
            .args(args)
            .output()
            .unwrap()
    };
    let added = cli(&[
        "add",
        "package",
        "@example/math@1",
        "--registry",
        "../registry",
    ]);
    assert!(added.status.success(), "{added:?}");
    let verified = cli(&["package", "verify-lock"]);
    assert!(verified.status.success(), "{verified:?}");
    let cached = root.join(format!("target/cache/packages/{digest}.cor"));
    assert_eq!(fs::read_to_string(&cached).unwrap(), source);
    fs::remove_file(&cached).unwrap();
    let checked = cli(&["check"]);
    assert!(checked.status.success(), "{checked:?}");
    assert_eq!(fs::read_to_string(&cached).unwrap(), source);
    fs::rename(&registry, tmp.path().join("disconnected")).unwrap();
    let run = cli(&["run", "--target", "native"]);
    assert!(run.status.success(), "{run:?}");
    assert_eq!(String::from_utf8_lossy(&run.stdout).trim(), "12");
    fs::write(&cached, source.replace("* 3", "* 5")).unwrap();
    let rejected = cli(&["run"]);
    assert!(!rejected.status.success(), "{rejected:?}");
    assert!(
        String::from_utf8_lossy(&rejected.stderr).contains(&digest),
        "{rejected:?}"
    );

    fs::rename(tmp.path().join("disconnected"), &registry).unwrap();
    let first_lock = fs::read_to_string(root.join("Corvid.lock")).unwrap();
    let mut updated_index = index;
    let versions = &mut updated_index["packages"][package]["versions"];
    versions["1.1.0"] = versions[version].clone();
    versions["1.1.0"]["version"] = "1.1.0".into();
    fs::write(registry.join("index.json"), updated_index.to_string()).unwrap();
    let updated = cli(&["update", package]);
    assert!(updated.status.success(), "{updated:?}");
    let verified = cli(&["package", "verify-lock"]);
    assert!(verified.status.success(), "{verified:?}");
    let lock = root.join("Corvid.lock");
    let updated_lock = fs::read_to_string(&lock).unwrap();
    assert!(updated_lock.contains("corvid://@example/math/v1.1.0"));
    assert!(!updated_lock.contains("corvid://@example/math/v1.0.0"));
    fs::write(&lock, format!("{updated_lock}\n{first_lock}")).unwrap();
    let conflict = cli(&["package", "verify-lock", "--json"]);
    assert!(!conflict.status.success(), "{conflict:?}");
    assert!(
        String::from_utf8_lossy(&conflict.stdout).contains("multiple versions"),
        "{conflict:?}"
    );
}
