use std::{fs, process::Command};

#[test]
fn interpreter_dispatches_typed_python_imports() {
    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    let library = root.join("numbers.cor");
    fs::write(
        root.join("main.cor"),
        r#"import "./numbers" as numbers

agent main() -> Int:
    return numbers.compute()
"#,
    )
    .unwrap();
    let source = r#"import python "math" as m effects: pure:
    fn gcd(left: Int, right: Int) -> Int

public agent compute() -> Int:
    return m.gcd(18, 12)
"#;
    for (code, success, expected) in [
        (source.to_owned(), true, "6"),
        (
            source
                .replace("gcd(left: Int, right: Int)", "isqrt(n: Int)")
                .replace("gcd(18, 12)", "isqrt(-1)"),
            false,
            "ValueError",
        ),
        (
            source
                .replace("gcd(left: Int, right: Int)", "sqrt(n: Int)")
                .replace("gcd(18, 12)", "sqrt(2)"),
            false,
            // Read with the public decoder, as every tool result is.
            "expected Int, found the fractional number",
        ),
        (
            source
                .replace(r#""math""#, r#""builtins""#)
                .replace("gcd(left: Int, right: Int) -> Int", "object() -> String")
                .replace("return m.gcd(18, 12)", "value = m.object()\n    return 1"),
            false,
            "is not JSON serializable",
        ),
        (
            source.replace("gcd(18, 12)", "missing(18, 12)"),
            false,
            "no declared signature",
        ),
    ] {
        fs::write(&library, code).unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .current_dir(root)
            .args(["run", "main.cor"])
            .output()
            .unwrap();
        assert_eq!(output.status.success(), success, "{output:?}");
        if success {
            assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), expected);
        } else {
            assert!(
                String::from_utf8_lossy(&output.stderr).contains(expected),
                "{output:?}"
            );
        }
    }
}

#[test]
fn qualified_approval_reviews_one_python_call_with_matching_arguments() {
    use std::io::Write;
    use std::process::Stdio;

    let project = tempfile::tempdir().unwrap();
    let root = project.path();
    fs::write(root.join("reviewed.py"), "def echo(value):\n    with open('calls', 'a') as log:\n        log.write(value)\n    return value\n").unwrap();
    let source = r#"effect reviewed_call:
    cost: $0.30
    trust: human_required

import python "reviewed" as host effects: reviewed_call:
    fn echo(value: String) -> String

@budget($0.30)
agent main() -> String:
    approve host.echo("hello")
    return host.echo("hello")
"#;
    let run = |source: &str, answer: &str| {
        fs::write(root.join("main.cor"), source).unwrap();
        let mut child = Command::new(env!("CARGO_BIN_EXE_corvid"))
            .args(["run", "main.cor"])
            .current_dir(root)
            .env("PYTHONPATH", root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(answer.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    };
    let ok = run(source, "yes\n");
    assert!(ok.status.success(), "{ok:?}");
    assert_eq!(fs::read_to_string(root.join("calls")).unwrap(), "hello");
    for (program, answer) in [
        (source.to_owned(), "no\n"),
        (
            source.replace(
                "approve host.echo(\"hello\")",
                "approve host.echo(\"other\")",
            ),
            "yes\n",
        ),
        (source.replace("    approve host.echo(\"hello\")\n", ""), ""),
        (source.replace("approve host.echo", "approve HostEcho"), ""),
        (
            source.replace("approve host.echo", "approve host.missing"),
            "",
        ),
    ] {
        let rejected = run(&program, answer);
        assert!(!rejected.status.success(), "{rejected:?}");
        assert_eq!(fs::read_to_string(root.join("calls")).unwrap(), "hello");
    }
    let twice = source.replace("$0.30)", "$0.60)").replace(
        "    return host.echo",
        "    first = host.echo(\"hello\")\n    return host.echo",
    );
    let reused = run(&twice, "yes\n");
    assert!(!reused.status.success(), "{reused:?}");
    assert_eq!(
        fs::read_to_string(root.join("calls")).unwrap(),
        "hellohello"
    );
}
