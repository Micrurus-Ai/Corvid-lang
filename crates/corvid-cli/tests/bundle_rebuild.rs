mod bundle_support;

use std::fs;

use bundle_support::{create_fixture, run_corvid};

#[test]
fn bundle_verify_rebuild_accepts_happy_path_and_catches_binding_or_platform_drift() {
    let fixture = create_fixture();

    let ok = run_corvid(
        &[
            "bundle",
            "verify",
            fixture.root.to_str().expect("utf8 root"),
            "--rebuild",
        ],
        &fixture.root,
    );
    assert!(
        ok.status.success(),
        "bundle verify --rebuild failed: stdout={} stderr={}",
        String::from_utf8_lossy(&ok.stdout),
        String::from_utf8_lossy(&ok.stderr)
    );

    let rust_readme = fixture.root.join("bindings_rust").join("README.md");
    fs::write(&rust_readme, "tampered binding\n").expect("tamper rust binding");
    let mismatch = run_corvid(
        &[
            "bundle",
            "verify",
            fixture.root.to_str().expect("utf8 root"),
            "--rebuild",
        ],
        &fixture.root,
    );
    assert_eq!(mismatch.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&mismatch.stderr);
    assert!(stderr.contains("BundleHashMismatch"), "stderr was: {stderr}");
}

#[test]
fn bundle_verify_rebuild_rejects_target_mismatch() {
    let fixture = create_fixture();
    let manifest = fixture.manifest_path.clone();
    let original = fs::read_to_string(&manifest).expect("read manifest");
    // The fixture's `target_triple` is the host's triple (set by
    // `corvid build` at fixture-creation time). To trigger
    // `BundlePlatformUnsupported`, rewrite the manifest to claim a
    // NOT-host triple — which differs by platform. Hard-coding the
    // Windows-side rewrite worked on Windows but became a no-op on
    // Linux (where the fixture's triple is already
    // `x86_64-unknown-linux-gnu`), leaving the test silently
    // unable to trigger the rejection.
    let (host_triple, foreign_triple) = if cfg!(target_os = "linux") {
        ("x86_64-unknown-linux-gnu", "x86_64-pc-windows-msvc")
    } else if cfg!(target_os = "windows") {
        ("x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu")
    } else if cfg!(target_os = "macos") && cfg!(target_arch = "aarch64") {
        ("aarch64-apple-darwin", "x86_64-pc-windows-msvc")
    } else if cfg!(target_os = "macos") {
        ("x86_64-apple-darwin", "x86_64-pc-windows-msvc")
    } else {
        eprintln!("skipping bundle_verify_rebuild_rejects_target_mismatch on unrecognised host");
        return;
    };
    fs::write(
        &manifest,
        original.replace(
            &format!("target_triple = \"{host_triple}\""),
            &format!("target_triple = \"{foreign_triple}\""),
        ),
    )
    .expect("rewrite manifest");
    let result = run_corvid(
        &[
            "bundle",
            "verify",
            fixture.root.to_str().expect("utf8 root"),
            "--rebuild",
        ],
        &fixture.root,
    );
    assert_eq!(result.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&result.stderr);
    assert!(stderr.contains("BundlePlatformUnsupported"), "stderr was: {stderr}");
}
