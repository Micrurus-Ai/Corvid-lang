//! Pretty diagnostic rendering via `ariadne`.
//!
//! Every diagnostic carries a span (byte offsets) and a rich message.
//! This module turns them into the Rust-style multi-line output that
//! makes first impressions count.

use crate::Diagnostic;
use ariadne::{Color, Config, Label, Report, ReportKind, Source};
use std::io::IsTerminal;
use std::path::Path;

/// Render a diagnostic to a string suitable for stderr.
///
/// Uses `ariadne` to produce multi-line output with the offending span
/// highlighted under the source code, plus the help hint as a footer.
/// ANSI color is emitted only when `NO_COLOR` is unset *and* stderr is
/// a real terminal — captured / piped / redirected output stays plain
/// text so PowerShell conhost and CI logs render readably.
pub fn render_pretty(diag: &Diagnostic, source_path: &Path, source: &str) -> String {
    render_pretty_with_severity(diag, source_path, source, Severity::Error)
}

/// Self-trial round 4 Gap A: severity-aware variant so warnings
/// (like `W0280` schedule-not-executable) render with the correct
/// "warning:" header + yellow accent instead of being shown as
/// "error: W0280 ...". The original `render_pretty` keeps the
/// error-only path callers depend on.
pub fn render_pretty_with_severity(
    diag: &Diagnostic,
    source_path: &Path,
    source: &str,
    severity: Severity,
) -> String {
    let (source_path, source) = diag.location(source_path, source);
    let filename = source_path.display().to_string();
    // The lexer reports bytes, while ariadne positions labels by character.
    let char_offset = |byte| source.char_indices().take_while(|(i, _)| *i < byte).count();
    let start = char_offset(diag.span.start);
    let span = start..char_offset(diag.span.end).max(start);

    let code = diag.code;
    let with_color = std::env::var_os("NO_COLOR").is_none() && std::io::stderr().is_terminal();
    let (header, accent) = match severity {
        Severity::Error => ("error", Color::Red),
        Severity::Warning => ("warning", Color::Yellow),
    };
    let kind = ReportKind::Custom(header, accent);

    let mut builder = Report::build(kind, filename.as_str(), span.start)
        .with_config(Config::default().with_color(with_color))
        .with_code(code)
        .with_message(short_headline(&diag.message))
        .with_label(
            Label::new((filename.as_str(), span))
                .with_message(label_for(&diag.message))
                .with_color(accent),
        );

    if let Some(hint) = &diag.hint {
        builder = builder.with_help(hint.as_str());
    }

    let mut buf = Vec::new();
    let _ = builder
        .finish()
        .write((filename.as_str(), Source::from(source)), &mut buf);
    let rendered = String::from_utf8_lossy(&buf).to_string();
    if with_color {
        rendered
    } else {
        // ariadne 0.4 mostly respects `Config::with_color(false)`, but
        // the `ReportKind::Custom` colour and a few terminal-default
        // codes (e.g. `\x1b[39m`) leak through. Strip every CSI
        // sequence (`ESC '[' parameters letter`) so PowerShell conhost
        // and CI logs render plain text the way piped output should.
        strip_ansi(&rendered)
    }
}

/// Strip ANSI CSI sequences (`ESC '[' parameters final-byte`) from `s`.
///
/// Used when the renderer has decided not to emit color but the
/// underlying renderer leaks a few escapes anyway.
fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\x1b' && matches!(chars.peek(), Some(&'[')) {
            chars.next(); // consume '['
            // Consume parameter bytes (digits and ';'), then one final
            // byte (any letter, typically 'm', 'K', 'H', ...).
            for next in chars.by_ref() {
                if next.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(ch);
        }
    }
    out
}

/// Render every diagnostic in sequence, followed by a summary line.
pub fn render_all_pretty(
    diags: &[Diagnostic],
    source_path: &Path,
    source: &str,
) -> String {
    let mut out = String::new();
    for d in diags {
        out.push_str(&render_pretty(d, source_path, source));
    }
    out.push_str(&format!("\n{} error(s) found.\n", diags.len()));
    out
}

/// Self-trial round 4 Gap A — same as `render_all_pretty` but with
/// a warning-shaped header + "N warning(s)" summary instead of
/// "N error(s) found." Used by `corvid check` to surface
/// non-blocking warnings (e.g. `ScheduleNotExecutable`).
pub fn render_all_pretty_warnings(
    diags: &[Diagnostic],
    source_path: &Path,
    source: &str,
) -> String {
    let mut out = String::new();
    for d in diags {
        out.push_str(&render_pretty_with_severity(d, source_path, source, Severity::Warning));
    }
    out.push_str(&format!("\n{} warning(s).\n", diags.len()));
    out
}

#[derive(Debug, Clone, Copy)]
pub enum Severity {
    Error,
    Warning,
}

/// Condensed one-line headline for the report's top message.
/// ariadne duplicates the message if we pass the full text, so we keep
/// the headline short and put detail on the label and help lines.
fn short_headline(msg: &str) -> String {
    if msg.contains("type mismatch") { return msg.to_owned(); }
    // Strip anything after a colon so the headline stays tight.
    if let Some(idx) = msg.find(':') {
        if idx < 80 {
            return msg[..idx].to_string();
        }
    }
    msg.to_string()
}

fn label_for(msg: &str) -> String {
    // A per-error hint for the underline caret. These are human-readable
    // phrasings that complement the top headline.
    if msg.contains("dangerous tool") {
        "this call needs prior approval".into()
    } else if msg.contains("ungrounded return") {
        "return value lacks a proven grounded source".into()
    } else if msg.contains("effect constraint violated") && msg.contains("budget") {
        "static worst-case cost exceeds the declared budget".into()
    } else if msg.contains("cost analysis warning") {
        "static cost bound could not be proven".into()
    } else if msg.contains("undefined name") {
        "not declared in this scope".into()
    } else if msg.contains("duplicate declaration") {
        "conflicts with an earlier declaration".into()
    } else if msg.contains("no field named") {
        "field does not exist".into()
    } else if msg.contains("wrong number of arguments") {
        "wrong argument count".into()
    } else if msg.contains("return type mismatch") {
        "wrong return type".into()
    } else if msg.contains("is a type, not a value") {
        "types cannot be used as values".into()
    } else if msg.contains("is a function; call it with") {
        "missing `()` for call".into()
    } else if msg.contains("type mismatch") {
        "wrong type here".into()
    } else {
        msg.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::{label_for, render_pretty};
    use crate::Diagnostic;
    use corvid_ast::Span;
    use std::path::Path;

    #[test]
    fn ungrounded_return_has_its_label() {
        let msg = "ungrounded return in agent `answer`: no retrieval source feeds into the return value";
        assert_eq!(
            label_for(msg),
            "return value lacks a proven grounded source"
        );
    }

    #[test]
    fn budget_violation_has_its_label() {
        let msg = "effect constraint violated in agent `planner`: cost: $1.031 > $1.00 budget (path: search -> generate_plan)";
        assert_eq!(
            label_for(msg),
            "static worst-case cost exceeds the declared budget"
        );
    }

    #[test]
    fn unbounded_cost_warning_has_its_label() {
        let msg = "cost analysis warning in agent `planner`: static iteration count unknown";
        assert_eq!(label_for(msg), "static cost bound could not be proven");
    }

    #[test]
    fn strip_ansi_removes_csi_sequences_and_preserves_other_text() {
        let input = "\x1b[31m[E0001] error:\x1b[0m unexpected character";
        assert_eq!(super::strip_ansi(input), "[E0001] error: unexpected character");
        // No-escape input is preserved exactly.
        let plain = "no escapes here";
        assert_eq!(super::strip_ansi(plain), plain);
    }

    #[test]
    fn render_pretty_omits_ansi_when_stderr_is_not_a_terminal() {
        // `cargo test` runs each test with stderr captured by libtest's
        // harness, which is a pipe rather than a TTY. The renderer must
        // therefore emit plain text — no `\x1b[` escape sequences — even
        // though `Color::Red` is configured for the report kind.
        let diag = Diagnostic {
            origin: None,
            span: Span::new(0, 5),
            message: "type mismatch in agent `main`: Int vs String".to_string(),
            hint: Some("rebind one operand to match the other".to_string()),
            code: "E0208",
        };
        let source = "agent main() -> Int:\n    return 0\n";
        let rendered = render_pretty(&diag, Path::new("main.cor"), source);
        assert!(
            !rendered.contains('\x1b'),
            "expected plain output when stderr is not a TTY, got escape sequences:\n{rendered}"
        );
    }
}
