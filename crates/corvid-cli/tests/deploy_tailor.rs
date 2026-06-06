//! 33Q13c end-to-end acceptance — `corvid deploy tailor` against
//! real reference apps and a bare `corvid new`-shape app.
//!
//! These tests pin three properties:
//!
//! 1. **Per-app signal accuracy**: the deterministic IR walk
//!    surfaces the right counts for a known app (e.g. the
//!    personal-executive-agent reference app has > 0 server blocks,
//!    > 0 agents, > 0 dangerous tools). The exact numbers can shift
//!    when the reference apps are edited, so the assertions use
//!    inequalities rather than equalities.
//! 2. **Recommendation matching**: when a known signal is present
//!    (dangerous tools, server block, migrations dir, etc.), the
//!    corresponding recommendation MUST appear in the output —
//!    when the signal is absent, the recommendation MUST NOT appear.
//!    This is the load-bearing groundedness contract: the tailor
//!    cannot fabricate recommendations for patterns the app doesn't
//!    have.
//! 3. **Bare-app coverage**: a freshly-scaffolded `corvid new`-shape
//!    app surfaces the no-server-block WARN recommendation (the
//!    Dockerfile's CMD would otherwise fail). This is the
//!    operator-facing safety net for the friends-and-family round.

use std::path::PathBuf;
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn corvid_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_corvid"))
}

fn tailor_json(app: &PathBuf) -> serde_json::Value {
    let output = Command::new(corvid_bin())
        .arg("deploy")
        .arg("tailor")
        .arg(app)
        .arg("--json")
        .current_dir(repo_root())
        .output()
        .expect("spawn corvid deploy tailor");
    assert!(
        output.status.success(),
        "deploy tailor exited non-zero. stderr={}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("tailor JSON utf-8");
    serde_json::from_str(&stdout).unwrap_or_else(|err| {
        panic!("tailor JSON parse failed: {err}\n--- stdout ---\n{stdout}")
    })
}

/// Coverage: the personal_executive_agent reference app surfaces the
/// expected canonical signals (server block, dangerous tools,
/// migrations dir, agents, tools). When PEA's source changes the
/// exact counts can shift, so assertions use `> 0` not equality —
/// the load-bearing property is "the analyzer detects these signal
/// kinds at all," not "the counts are exactly N."
#[test]
fn deploy_tailor_surfaces_canonical_signals_for_reference_app() {
    let pea = repo_root()
        .join("examples")
        .join("backend")
        .join("personal_executive_agent");
    assert!(
        pea.is_dir(),
        "personal_executive_agent reference app missing: {}",
        pea.display()
    );

    let report = tailor_json(&pea);

    let signals = &report["signals"];
    assert!(
        signals["server_blocks"].as_u64().expect("u64") > 0,
        "PEA must declare at least one server block: {signals:?}"
    );
    assert!(
        signals["agents"].as_u64().expect("u64") > 0,
        "PEA must declare at least one agent: {signals:?}"
    );
    assert!(
        signals["dangerous_tools"].as_u64().expect("u64") > 0,
        "PEA must declare at least one `dangerous` tool — that's the \
         load-bearing 5-agent surface the trial prompt's Surface 3 \
         exercises: {signals:?}"
    );
    assert!(
        signals["has_migrations"].as_bool().expect("bool"),
        "PEA must have a migrations/ directory — it's part of the \
         33Q4 presence-conditional Dockerfile shape: {signals:?}"
    );

    // The matched-recommendation property: PEA has dangerous tools,
    // so the critical-severity approval-queue recommendation MUST
    // appear in the output.
    let recs = report["recommendations"].as_array().expect("rec array");
    let has_critical_approval = recs.iter().any(|r| {
        r["severity"].as_str() == Some("critical")
            && r["title"]
                .as_str()
                .map(|t| t.contains("approval-queue"))
                .unwrap_or(false)
    });
    assert!(
        has_critical_approval,
        "PEA has dangerous tools → tailor MUST surface the critical \
         approval-queue recommendation. recs={recs:?}"
    );

    // Migrations dir present → must surface the migrate-up
    // recommendation.
    let has_migrate_warn = recs.iter().any(|r| {
        r["severity"].as_str() == Some("warn")
            && r["title"]
                .as_str()
                .map(|t| t.contains("migrate up"))
                .unwrap_or(false)
    });
    assert!(
        has_migrate_warn,
        "PEA has migrations/ → tailor MUST surface the migrate-up \
         WARN recommendation. recs={recs:?}"
    );
}

/// 33Q13c load-bearing groundedness: when a signal is ABSENT, the
/// corresponding recommendation MUST NOT appear. This is the no-
/// fabrication property — the tailor's deterministic core cannot
/// invent a recommendation that has no source in the analyzed IR.
///
/// Constructs a bare app with no migrations/ dir and asserts the
/// migrate-up recommendation is NOT emitted, AND asserts the no-
/// server-block WARN appears (the analyzer's safety net).
#[test]
fn deploy_tailor_is_grounded_recommendations_match_present_signals() {
    let app_dir = tempfile::tempdir().expect("tempdir");
    let src_dir = app_dir.path().join("src");
    std::fs::create_dir_all(&src_dir).expect("create src/");
    // Minimal source with NO server block + NO tools — the bare
    // `corvid new` shape. This exercises the no-server-block WARN
    // recommendation path.
    let source = "agent dummy(x: Int) -> Int:\n    return x\n";
    std::fs::write(src_dir.join("main.cor"), source).expect("write main.cor");

    let app_dir_path = app_dir.path().to_path_buf();
    let report = tailor_json(&app_dir_path);

    let signals = &report["signals"];
    assert_eq!(
        signals["server_blocks"].as_u64().expect("u64"),
        0,
        "bare app has NO server block: {signals:?}"
    );
    assert_eq!(
        signals["dangerous_tools"].as_u64().expect("u64"),
        0,
        "bare app has NO dangerous tools: {signals:?}"
    );
    assert!(
        !signals["has_migrations"].as_bool().expect("bool"),
        "bare app has NO migrations/: {signals:?}"
    );

    let recs = report["recommendations"].as_array().expect("rec array");

    // GROUNDEDNESS — when the signal is absent, the matching
    // recommendation MUST NOT appear:

    let has_migrate_warn = recs.iter().any(|r| {
        r["title"]
            .as_str()
            .map(|t| t.contains("migrate up"))
            .unwrap_or(false)
    });
    assert!(
        !has_migrate_warn,
        "bare app has NO migrations/ — tailor MUST NOT surface a \
         migrate-up recommendation. That would be a fabrication, the \
         exact property 33Q13c pins so a future LLM layer can't \
         hallucinate. recs={recs:?}"
    );

    let has_critical_approval = recs.iter().any(|r| {
        r["severity"].as_str() == Some("critical")
            && r["title"]
                .as_str()
                .map(|t| t.contains("approval-queue"))
                .unwrap_or(false)
    });
    assert!(
        !has_critical_approval,
        "bare app has NO dangerous tools — tailor MUST NOT surface \
         the approval-queue critical recommendation. recs={recs:?}"
    );

    // SAFETY NET: a bare app with NO server block + a generated
    // Dockerfile that CMDs `corvid serve` WILL fail at container
    // startup. The tailor MUST surface this as a WARN so operators
    // don't deploy a known-broken image.
    let has_no_server_warn = recs.iter().any(|r| {
        r["severity"].as_str() == Some("warn")
            && r["title"]
                .as_str()
                .map(|t| t.contains("No server block"))
                .unwrap_or(false)
    });
    assert!(
        has_no_server_warn,
        "bare app with NO server block MUST surface a WARN — the \
         generated Dockerfile's CMD `corvid serve` would fail at \
         container startup. recs={recs:?}"
    );
}
