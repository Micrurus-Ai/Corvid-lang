use anyhow::{Context, Result};
use corvid_driver::{render_all_pretty, Diagnostic};
use corvid_syntax::{lex, parse_file, TokKind, Token};
use std::io::Write;
use std::path::{Path, PathBuf};

pub(crate) fn run(mut files: Vec<PathBuf>, check: bool) -> Result<u8> {
    if files.is_empty() {
        crate::project_source::resolve_project_source(None)?;
        collect_sources(Path::new("src"), &mut files)?;
        if Path::new("tests").is_dir() {
            collect_sources(Path::new("tests"), &mut files)?;
        }
    }
    files.sort();
    files.dedup();
    let mut changes = Vec::new();
    for path in files {
        let source =
            std::fs::read_to_string(&path).with_context(|| format!("read {}", path.display()))?;
        let tokens = lex(&source).map_err(|errors| diagnostics(&path, &source, errors))?;
        let (_, errors) = parse_file(&tokens);
        if !errors.is_empty() {
            return Err(diagnostics(&path, &source, errors));
        }
        let formatted = format_source(&source, &tokens);
        if formatted != source {
            anyhow::ensure!(
                lex(&formatted).is_ok_and(|after| after
                    .iter()
                    .map(|t| &t.kind)
                    .eq(tokens.iter().map(|t| &t.kind))),
                "formatting would change the token sequence in {}",
                path.display()
            );
            changes.push((path, formatted));
        }
    }
    for (path, formatted) in &changes {
        if check {
            println!("would format {}", path.display());
            continue;
        }
        // Resolve explicit symlinks before atomically replacing their targets.
        let target = path.canonicalize()?;
        let mut tmp = tempfile::NamedTempFile::new_in(target.parent().unwrap())?;
        tmp.write_all(formatted.as_bytes())?;
        tmp.as_file()
            .set_permissions(std::fs::metadata(&target)?.permissions())?;
        tmp.persist(&target)
            .with_context(|| format!("format {}", path.display()))?;
        println!("formatted {}", path.display());
    }
    Ok(u8::from(check && !changes.is_empty()))
}

fn collect_sources(dir: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in dir
        .read_dir()
        .with_context(|| format!("read {}", dir.display()))?
    {
        let entry = entry?;
        let ty = entry.file_type()?;
        let path = entry.path();
        if ty.is_dir() && path != Path::new("src/std") {
            collect_sources(&path, files)?;
        } else if ty.is_file() && path.extension().is_some_and(|ext| ext == "cor") {
            files.push(path);
        }
    }
    Ok(())
}

fn diagnostics<E: Into<Diagnostic>>(path: &Path, source: &str, errors: Vec<E>) -> anyhow::Error {
    let errors = errors.into_iter().map(Into::into).collect::<Vec<_>>();
    anyhow::anyhow!("{}", render_all_pretty(&errors, path, source))
}

// Only replace whitespace between tokens. Literal spelling, comments, line
// endings and continuation lines stay in the original source slices.
fn format_source(source: &str, tokens: &[Token]) -> String {
    use TokKind::*;
    let mut out = std::string::String::with_capacity(source.len());
    let mut depth = 0;
    let mut statement_start = true;
    let mut end = 0;
    let mut previous = None;
    let mut previous_unary = false;
    for token in tokens {
        match token.kind {
            Indent => {
                depth += 1;
                continue;
            }
            Dedent => {
                depth -= 1;
                continue;
            }
            Newline => {
                statement_start = true;
                previous = None;
                continue;
            }
            Eof => break,
            _ => {}
        }
        let gap = &source[end..token.span.start];
        if statement_start {
            let leading = gap.trim_end_matches([' ', '\t']);
            out.push_str(leading);
            out.extend(std::iter::repeat_n(' ', depth * 4));
        } else if gap.bytes().all(|b| matches!(b, b' ' | b'\t')) {
            out.push_str(spacing(previous.unwrap(), &token.kind, previous_unary, gap));
        } else {
            out.push_str(gap);
        }
        out.push_str(&source[token.span.start..token.span.end]);
        previous_unary = matches!(token.kind, Minus | Plus) && !previous.is_some_and(ends_operand);
        previous = Some(&token.kind);
        end = token.span.end;
        statement_start = false;
    }
    out.push_str(&source[end..]);
    if !out.is_empty() && !out.ends_with('\n') {
        out.push_str(if source.contains("\r\n") {
            "\r\n"
        } else {
            "\n"
        });
    }
    out
}

fn ends_operand(kind: &TokKind) -> bool {
    use TokKind::*;
    matches!(
        kind,
        Ident(_)
            | Int(_)
            | Float(_)
            | StringLit(_)
            | KwTrue
            | KwFalse
            | KwNothing
            | RParen
            | RBracket
            | RBrace
            | Question
    )
}

fn spacing<'a>(left: &TokKind, right: &TokKind, unary: bool, original: &'a str) -> &'a str {
    use TokKind::*;
    let operator = |t: &TokKind| {
        matches!(
            t,
            Assign
                | Eq
                | NotEq
                | LtEq
                | GtEq
                | PlusEq
                | MinusEq
                | StarEq
                | SlashEq
                | PercentEq
                | Arrow
                | Star
                | Slash
        )
    };
    if matches!(right, Comma | Colon | RParen | RBracket | RBrace | Question)
        || matches!(left, LParen | LBracket | LBrace)
        || matches!((left, right), (Ident(_), LParen | Dot) | (Dot, Ident(_)))
    {
        ""
    } else if matches!(left, Comma | Colon) || operator(left) || operator(right) {
        " "
    } else if matches!(left, Minus | Plus) {
        if unary {
            ""
        } else {
            " "
        }
    } else if matches!(right, Minus | Plus)
        && (ends_operand(left) || matches!(left, KwReturn | KwYield))
    {
        " "
    } else if original.is_empty() {
        ""
    } else {
        " "
    }
}
