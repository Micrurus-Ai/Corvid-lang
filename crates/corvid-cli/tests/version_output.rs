//! Lock the `corvid --version` output shape introduced by slice
//! `35V2-P33-version-output-sha`. The reviewer-facing prompt at
//! `docs/external-trials/33m-friends-and-family-followup-prompt.md`
//! tells reviewers to verify their binary is at the right commit
//! by reading the SHA out of `--version`; if the output regresses
//! to the bare crate-version, that verification path silently
//! breaks. This test fails-loud on either regression.

use std::path::PathBuf;
use std::process::Command;

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

#[test]
fn version_output_includes_short_sha_and_commit_date() {
    let out = Command::new(corvid_bin())
        .arg("--version")
        .output()
        .expect("run corvid --version");
    assert!(
        out.status.success(),
        "corvid --version exited non-zero: stdout=`{}` stderr=`{}`",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let trimmed = stdout.trim();

    // Expected shape: `corvid <crate-version> (<sha>, <date>)`.
    // The crate version comes from `CARGO_PKG_VERSION` (currently
    // `0.0.1`); the SHA + date come from `build.rs`'s git probe
    // (with `unknown` fallback). Use shape-checks rather than
    // value-checks so the test doesn't have to update on every
    // commit.
    assert!(
        trimmed.starts_with("corvid "),
        "version output must start with `corvid `: got `{trimmed}`"
    );
    assert!(
        trimmed.contains(env!("CARGO_PKG_VERSION")),
        "version output must contain the crate version `{}`: got `{trimmed}`",
        env!("CARGO_PKG_VERSION")
    );
    assert!(
        trimmed.contains('(') && trimmed.contains(')'),
        "version output must contain the `(sha, date)` parenthetical (slice \
         `35V2-P33-version-output-sha` shape): got `{trimmed}`. \
         If you intentionally removed it, the reviewer-facing prompt at \
         `docs/external-trials/33m-friends-and-family-followup-prompt.md` \
         needs to drop the `git log --oneline -1` check it tells the \
         reviewer to run, and the install-honesty audit it walks through \
         needs an update."
    );
    // The parenthetical should contain a comma (separator between sha
    // and date) — distinguishes the shape from a single-element
    // `(unknown)` collapse.
    let paren = trimmed
        .split_once('(')
        .and_then(|(_, after)| after.split_once(')'))
        .map(|(inside, _)| inside.to_string())
        .expect("version output had `(` but no closing `)`");
    assert!(
        paren.contains(", "),
        "version parenthetical must be `<sha>, <date>` separated by `, `: got `({paren})`"
    );
}
