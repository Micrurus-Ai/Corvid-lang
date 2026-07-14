use std::process::Command;

#[test]
fn package_help_states_hosted_registry_boundary() {
    let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["package", "--help"])
        .output()
        .expect("run corvid package --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("format-and-tooling only")
            && stdout.contains("no Corvid-hosted registry service runs yet"),
        "{stdout}"
    );
}

#[test]
fn add_package_help_requires_explicit_registry_source() {
    // Slice 49a moved package adds under the unified capability verb:
    // `corvid add package <spec>` (skills/mcp/connectors share `add`).
    let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["add", "package", "--help"])
        .output()
        .expect("run corvid add package --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("No Corvid-hosted package registry runs yet")
            && stdout.contains("CORVID_PACKAGE_REGISTRY"),
        "{stdout}"
    );
}

#[test]
fn add_help_names_the_capability_kinds() {
    let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
        .args(["add", "--help"])
        .output()
        .expect("run corvid add --help");

    assert!(output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("skill") && stdout.contains("package"),
        "`corvid add --help` must list the capability kinds; got: {stdout}"
    );
}
