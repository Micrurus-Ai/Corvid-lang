//! Slice `33J6-grammar-drift-gate` — the drift gate that keeps
//! `docs/reference/grammar.md` consistent with the parser.
//!
//! ## What this gate enforces
//!
//! `docs/reference/grammar.md` is a hand-written EBNF derived from the
//! parser. Hand-written derivations drift: a contributor changes the
//! parser, the docs stay frozen, and the published grammar quietly
//! misrepresents what the language accepts. This gate enforces two
//! structural invariants that a drift would violate:
//!
//! 1. **Every RHS reference resolves.** Every lowercase identifier on
//!    the RHS of an EBNF production must either be (a) declared as the
//!    LHS of some other production in the same file or (b) appear on
//!    the explicit terminal-token allow-list below. A reference that
//!    matches neither is a typo or a stale reference — the gate names
//!    it and points the contributor at the file to fix.
//!
//! 2. **Every production is reachable from `program`.** The grammar
//!    root is `program`. Any production whose name never transitively
//!    appears on a reachable RHS is either dead documentation (delete
//!    it) or a missing reference site (add it). The gate names which
//!    productions are orphaned so the contributor can decide.
//!
//! 3. **Parse-evidence correspondence (slice 44c).** Every production
//!    NOT marked `# PLANNED(<slice>)` on its LHS line must be mapped
//!    to a curated Corvid source snippet in `EVIDENCE` below, and that
//!    snippet must parse through the real `parse_file`. This is the
//!    check the original 33J6 gate deliberately skipped (and its doc
//!    header admitted skipping) — the 2026-07-09 language gap audit
//!    found seven-plus productions documented as shipped with no
//!    parser behind them, which is exactly the failure mode this
//!    closes. Adding a production without evidence, leaving a stale
//!    evidence key, or shipping evidence that fails to parse all
//!    fail CI. PLANNED productions are design documentation; when
//!    their slice ships, the marker comes off and this gate demands
//!    the snippet.
//!
//! ## What this gate deliberately does NOT enforce
//!
//! - **Production names matching `parse_<name>` fns in the parser.** The
//!   parser uses Pratt-style precedence climbing, so its expression
//!   fns are named after the operator level rather than the EBNF
//!   production (`parse_add` for `add_expr`, `parse_cmp` for
//!   `cmp_expr`). A naming-substring drift gate would be flaky against
//!   this convention. The evidence table below is the non-flaky
//!   replacement: it proves each production's syntax parses without
//!   constraining parser-fn naming.
//!
//! - **Which productions a snippet exercises.** Evidence snippets are
//!   curated by hand — the gate proves they parse, not that they
//!   touch the exact production they're keyed to. Mechanically
//!   verifying coverage would need parser instrumentation; curation
//!   plus parse-success is the honest achievable bar, and review
//!   catches a snippet keyed to the wrong production.
//!
//! - **Lexical keyword set matching `TokKind::keyword_from`.** The
//!   grammar.md "Lexical tokens" paragraph now documents the
//!   contextual-vs-reserved split prose-side (44c); asserting
//!   set-equality remains future work.
//!
//! ## Failure mode
//!
//! If a contributor adds a production reference without declaring it,
//! this gate fails with a message like:
//!
//! ```text
//! grammar.md references undeclared production `if_expr` on these
//! lines: 214, 219. Either declare `if_expr ::= ...` or fix the typo.
//! ```
//!
//! If a contributor adds a declaration but no use of it, the gate
//! fails with a message naming the orphan:
//!
//! ```text
//! grammar.md declares `unused_thing` but no other production
//! references it transitively from `program`. Add a reference or
//! remove the orphan.
//! ```

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::fs;
use std::path::PathBuf;

/// Terminal tokens the grammar may reference without declaration.
/// These are lexer surfaces (uppercase tokens) or contextually-
/// parsed keywords that the grammar lists with PascalCase / lowercase
/// names but that don't have an EBNF production. The list is small,
/// stable, and curated — anything outside this set must be declared
/// as a production.
const TERMINAL_ALLOW_LIST: &[&str] = &[
    // Lexer-emitted tokens.
    "IDENT",
    "INT",
    "FLOAT",
    "STRING",
    "STRING_LITERAL",
    "NUMBER",
    "INDENT",
    "DEDENT",
    "NEWLINE",
    "EOF",
];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn grammar_md_path() -> PathBuf {
    repo_root().join("docs").join("reference").join("grammar.md")
}

/// Read grammar.md and return:
///   - the ordered set of EBNF code blocks (between fenced `\`\`\`ebnf`
///     blocks), each carrying its starting line number for diagnostics.
fn read_ebnf_blocks(src: &str) -> Vec<(usize, String)> {
    let mut blocks: Vec<(usize, String)> = Vec::new();
    let mut current: Option<(usize, Vec<String>)> = None;
    for (i, line) in src.lines().enumerate() {
        let lineno = i + 1; // 1-indexed for diagnostics
        let trimmed = line.trim_start();
        if trimmed == "```ebnf" {
            current = Some((lineno + 1, Vec::new()));
            continue;
        }
        if trimmed == "```" {
            if let Some((start, buf)) = current.take() {
                blocks.push((start, buf.join("\n")));
            }
            continue;
        }
        if let Some((_, buf)) = current.as_mut() {
            buf.push(line.to_string());
        }
    }
    blocks
}

/// Extract LHS declarations and RHS references from an EBNF block.
/// Returns a Vec of `(name, line_no, kind)` where kind is either
/// `Lhs` (declaration) or `Rhs` (reference).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SiteKind {
    Lhs,
    Rhs,
}

fn extract_sites(blocks: &[(usize, String)]) -> Vec<(String, usize, SiteKind)> {
    let mut sites: Vec<(String, usize, SiteKind)> = Vec::new();
    for (start, block) in blocks {
        for (offset, raw_line) in block.lines().enumerate() {
            let lineno = start + offset;
            // Strip the inline comment introducer `#` (EBNF comment).
            let line = match raw_line.find('#') {
                Some(idx) => &raw_line[..idx],
                None => raw_line,
            };
            // LHS detection: `name<spaces>::=` at the start of the
            // logical line. Continuation lines (with whitespace before
            // the identifier) are NOT declarations.
            if let Some(eq_pos) = line.find("::=") {
                let before = &line[..eq_pos];
                if let Some(name) = leading_ident(before) {
                    if line.starts_with(name) {
                        sites.push((name.to_string(), lineno, SiteKind::Lhs));
                    }
                }
                // The RHS portion of this line still contains references.
                for ident in extract_lowercase_idents(&line[eq_pos + 3..]) {
                    sites.push((ident, lineno, SiteKind::Rhs));
                }
            } else {
                // Continuation line — every lowercase identifier is a
                // reference.
                for ident in extract_lowercase_idents(line) {
                    sites.push((ident, lineno, SiteKind::Rhs));
                }
            }
        }
    }
    sites
}

/// Return the leading identifier of `s` (sequence of `[a-z_]+` at
/// the start, after stripping any leading whitespace).
fn leading_ident(s: &str) -> Option<&str> {
    let trimmed = s.trim_start();
    if trimmed.is_empty() {
        return None;
    }
    let bytes = trimmed.as_bytes();
    if !(bytes[0] == b'_' || bytes[0].is_ascii_lowercase()) {
        return None;
    }
    let mut end = 0;
    while end < bytes.len() {
        let b = bytes[end];
        if b == b'_' || b.is_ascii_lowercase() {
            end += 1;
        } else {
            break;
        }
    }
    if end == 0 {
        None
    } else {
        Some(&trimmed[..end])
    }
}

/// Extract every lowercase EBNF identifier from a line. Strips
/// single-quoted literals (`'foo'` — those are terminal keywords),
/// then collects every `[a-z_]+` token. Skips bare quotes.
fn extract_lowercase_idents(line: &str) -> Vec<String> {
    // Strip single-quoted literals.
    let mut cleaned = String::with_capacity(line.len());
    let mut in_quote = false;
    for ch in line.chars() {
        if ch == '\'' {
            in_quote = !in_quote;
            continue;
        }
        if in_quote {
            continue;
        }
        cleaned.push(ch);
    }

    // EBNF identifiers must START with a lowercase ASCII letter and
    // may CONTINUE with lowercase letters, underscores, or digits.
    // Starting on `_` would falsely match the `_` inside uppercase
    // terminal tokens like `STRING_LITERAL` (the `_` is a tokenizer
    // boundary, not an ident).
    let mut out: Vec<String> = Vec::new();
    let mut cur = String::new();
    for ch in cleaned.chars() {
        let is_start = ch.is_ascii_lowercase();
        let is_continue = ch.is_ascii_lowercase() || ch == '_' || ch.is_ascii_digit();
        if cur.is_empty() {
            if is_start {
                cur.push(ch);
            }
        } else if is_continue {
            cur.push(ch);
        } else {
            out.push(std::mem::take(&mut cur));
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// Productions whose LHS line carries a `# PLANNED(<slice>)` marker.
/// These are designed-but-unimplemented; they're exempt from the
/// evidence requirement.
fn planned_productions(blocks: &[(usize, String)]) -> BTreeSet<String> {
    let mut planned = BTreeSet::new();
    for (_, block) in blocks {
        for raw_line in block.lines() {
            if !raw_line.contains("PLANNED(") {
                continue;
            }
            let code = match raw_line.find('#') {
                Some(idx) => &raw_line[..idx],
                None => raw_line,
            };
            if let Some(eq_pos) = code.find("::=") {
                if let Some(name) = leading_ident(&code[..eq_pos]) {
                    if code.starts_with(name) {
                        planned.insert(name.to_string());
                    }
                }
            }
        }
    }
    planned
}

/// Curated parse-evidence snippets. Each must parse through
/// `corvid_syntax::parse_file`. Keys are snippet names referenced by
/// the `EVIDENCE` table.
const SNIPPETS: &[(&str, &str)] = &[
    (
        "imports",
        r#"import "./util"
import "./util" as u
import "./std/io" use io_read_text, io_write_text as write
import python "mylib" as ml
"#,
    ),
    (
        "record_types_and_stores",
        r#"type User:
    id: Int
    email: String

session cart:
    items: List<String>

memory prefs:
    theme: String
"#,
    ),
    (
        "weak_type_refs",
        r#"effect io_eff:
    reversible: true

type Node:
    label: String
    parent: Weak<Node, {tool_call}>
"#,
    ),
    (
        "tools_and_ownership",
        r#"effect fs_eff:
    reversible: false

tool wipe_dir(path: String @borrowed) -> Nothing dangerous uses fs_eff

tool read_config(path: String) -> String @owned uses fs_eff
"#,
    ),
    (
        "prompts",
        r#"effect llm_eff:
    cost: $0.01
    latency: medium
    confidence: 0.9

prompt summarize(text: String) -> String uses llm_eff:
    "Summarize in one sentence: {text}"
"#,
    ),
    (
        "effects_and_models",
        r#"effect pay_eff:
    cost: $50.00
    trust: supervisor_required
    reversible: false
    confidence: 0.9

model fast_model:
    provider: anthropic
    cost: $0.001
"#,
    ),
    (
        "agents_annotations_extern",
        r#"@replayable
@budget($0.50)
agent main(input: String) -> String:
    return input

@trust(autonomous)
agent stubborn(x: Int) -> Int:
    return x

pub extern "c" agent embed_entry(x: Int) -> Int:
    return x

public agent helper() -> Int:
    return 1
"#,
    ),
    (
        "extend_blocks",
        r#"type Customer:
    email: String

extend Customer:
    agent describe(self: Customer) -> String:
        return self.email
"#,
    ),
    (
        "server_and_schedule",
        r#"type HealthStatus:
    healthy: Bool

server api:
    route GET "/health" -> json HealthStatus:
        return HealthStatus(true)

schedule "0 9 * * *" zone "UTC" -> daily_summary()
"#,
    ),
    (
        "test_surface",
        r#"eval checks:
    assert 1 == 1
    assert_snapshot "golden"

test smoke:
    x = 1
    assert x == 1

test replayed from_trace "traces/golden.trace":
    assert 1 == 1

fixture seed_count() -> Int:
    return 3

mock summarize(text: String) -> String:
    return "mocked summary"
"#,
    ),
    (
        "statements",
        r#"effect pay_eff:
    cost: $10.00
    trust: supervisor_required
    reversible: false

tool refund(amount: Float) -> Nothing uses pay_eff

type Wallet:
    balance: Float

agent map_demo() -> Int:
    m = {"a": 1, "b": 2}
    m["c"] = 3
    return m.length()

agent mutate(w: Wallet, xs: List<Int>) -> Float:
    w.balance = 250.0
    w.balance += 50.0
    xs[0] = 9
    xs[1] *= 2
    n = 5
    n += 37
    return w.balance

agent flow(xs: List<Int>) -> Result<Int, String>:
    total: Int = 0
    for x in xs:
        if x > 3:
            break
        else:
            total = total + x
        continue
    approve Refund(50.0)
    refund(50.0)
    return Ok(total)

agent counter(n: Int) -> Stream<Int>:
    yield n

agent idle() -> Int:
    pass
    return 0
"#,
    ),
    (
        "expressions",
        r#"type Wallet:
    balance: Float

agent calc(a: Int, b: Float, name: String) -> Bool:
    c = -a * 2 + 3 % 2 - 1
    d = (a < 3) and not (b >= 2.0) or (name == "x")
    e = [1, 2, 3]
    f = e[0]
    flag = true
    return d

agent postfixes(w: Wallet) -> Result<Float, String>:
    v = try compute() on error retry 3 times backoff exponential 250
    x = helper()?
    total = w.balance + x
    return Ok(total)

agent compute() -> Int:
    return 1

agent helper() -> Result<Float, String>:
    return Ok(2.5)
"#,
    ),
];

/// production name -> snippet name that exercises it. Every declared,
/// non-PLANNED production MUST appear here exactly once; the gate
/// fails on missing or stale keys.
const EVIDENCE: &[(&str, &str)] = &[
    ("program", "imports"),
    ("decl", "imports"),
    ("visibility", "agents_annotations_extern"),
    ("import_decl", "imports"),
    ("import_target", "imports"),
    ("import_list", "imports"),
    ("import_item", "imports"),
    ("type_decl", "record_types_and_stores"),
    ("type_field", "record_types_and_stores"),
    ("store_decl", "record_types_and_stores"),
    ("type_ref", "record_types_and_stores"),
    ("type_args", "record_types_and_stores"),
    ("type_arg", "weak_type_refs"),
    ("weak_effect_row", "weak_type_refs"),
    ("weak_effect", "weak_type_refs"),
    ("tool_decl", "tools_and_ownership"),
    ("params", "tools_and_ownership"),
    ("param", "tools_and_ownership"),
    ("ownership", "tools_and_ownership"),
    ("uses_clause", "tools_and_ownership"),
    ("effect_name", "tools_and_ownership"),
    ("prompt_decl", "prompts"),
    ("prompt_body", "prompts"),
    ("template_line", "prompts"),
    ("effect_decl", "effects_and_models"),
    ("dimension_assign", "effects_and_models"),
    ("dimension_value", "effects_and_models"),
    ("model_decl", "effects_and_models"),
    ("model_field", "effects_and_models"),
    ("agent_decl", "agents_annotations_extern"),
    ("annotation", "agents_annotations_extern"),
    ("annotation_args", "agents_annotations_extern"),
    ("extern_abi", "agents_annotations_extern"),
    ("block", "agents_annotations_extern"),
    ("arg_list", "statements"),
    ("extend_decl", "extend_blocks"),
    ("extend_method", "extend_blocks"),
    ("server_decl", "server_and_schedule"),
    ("route_decl", "server_and_schedule"),
    ("schedule_decl", "server_and_schedule"),
    ("eval_decl", "test_surface"),
    ("test_decl", "test_surface"),
    ("fixture_decl", "test_surface"),
    ("mock_decl", "test_surface"),
    ("eval_body", "test_surface"),
    ("assertion", "test_surface"),
    ("http_method", "server_and_schedule"),
    ("stmt", "statements"),
    ("return_stmt", "statements"),
    ("yield_stmt", "statements"),
    ("if_stmt", "statements"),
    ("for_stmt", "statements"),
    ("approve_stmt", "statements"),
    ("break_stmt", "statements"),
    ("continue_stmt", "statements"),
    ("pass_stmt", "statements"),
    ("assign_stmt", "statements"),
    ("place", "statements"),
    ("assign_op", "statements"),
    ("expr_stmt", "statements"),
    ("expr", "expressions"),
    ("or_expr", "expressions"),
    ("and_expr", "expressions"),
    ("not_expr", "expressions"),
    ("cmp_expr", "expressions"),
    ("cmp_op", "expressions"),
    ("add_expr", "expressions"),
    ("mul_expr", "expressions"),
    ("unary_expr", "expressions"),
    ("postfix_expr", "expressions"),
    ("postfix_op", "expressions"),
    ("primary_expr", "expressions"),
    ("literal", "expressions"),
    ("list_literal", "expressions"),
    ("map_literal", "statements"),
    ("map_entry", "statements"),
    ("retry_expr", "expressions"),
];

#[test]
fn every_non_planned_production_has_parse_evidence() {
    let path = grammar_md_path();
    let src = fs::read_to_string(&path).expect("read grammar.md");
    let blocks = read_ebnf_blocks(&src);
    let sites = extract_sites(&blocks);
    let planned = planned_productions(&blocks);

    let declared: BTreeSet<String> = sites
        .iter()
        .filter(|(_, _, k)| *k == SiteKind::Lhs)
        .map(|(n, _, _)| n.clone())
        .collect();

    let snippet_map: BTreeMap<&str, &str> = SNIPPETS.iter().copied().collect();
    let evidence_map: BTreeMap<&str, &str> = EVIDENCE.iter().copied().collect();

    let mut problems: Vec<String> = Vec::new();

    // Every non-planned declared production needs an evidence entry.
    for name in &declared {
        if planned.contains(name) {
            if evidence_map.contains_key(name.as_str()) {
                problems.push(format!(
                    "`{name}` is marked PLANNED in grammar.md but has an \
                     EVIDENCE entry — if the feature shipped, remove the \
                     PLANNED marker; if not, remove the stale evidence."
                ));
            }
            continue;
        }
        if !evidence_map.contains_key(name.as_str()) {
            problems.push(format!(
                "`{name}` is declared in grammar.md without a PLANNED marker \
                 but has no EVIDENCE entry — add a parse-evidence snippet, \
                 or mark the production `# PLANNED(<slice>)` if it is not \
                 implemented."
            ));
        }
    }

    // No stale evidence keys for productions that no longer exist.
    for (name, snippet) in &evidence_map {
        if !declared.contains(*name) {
            problems.push(format!(
                "EVIDENCE maps `{name}` -> `{snippet}` but grammar.md no \
                 longer declares `{name}` — remove the stale entry."
            ));
        }
        if !snippet_map.contains_key(snippet) {
            problems.push(format!(
                "EVIDENCE maps `{name}` -> unknown snippet `{snippet}`."
            ));
        }
    }

    // Every referenced snippet must parse through the real parser.
    let referenced: BTreeSet<&str> = evidence_map.values().copied().collect();
    for name in referenced {
        let source = snippet_map
            .get(name)
            .unwrap_or_else(|| panic!("snippet `{name}` missing from SNIPPETS"));
        match corvid_syntax::lex(source) {
            Ok(tokens) => {
                let (_file, errors) = corvid_syntax::parse_file(&tokens);
                if !errors.is_empty() {
                    problems.push(format!(
                        "evidence snippet `{name}` fails to parse: {errors:?}\n--- source ---\n{source}"
                    ));
                }
            }
            Err(e) => problems.push(format!(
                "evidence snippet `{name}` fails to lex: {e:?}\n--- source ---\n{source}"
            )),
        }
    }

    assert!(
        problems.is_empty(),
        "grammar drift gate (slice 44c) — parse-evidence correspondence \
         failures:\n\n{}",
        problems.join("\n\n")
    );
}

#[test]
fn grammar_md_every_rhs_reference_resolves_to_a_declared_production() {
    let path = grammar_md_path();
    let src = fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "grammar drift gate: cannot read `{}`: {e}. The drift gate \
             can't run without grammar.md.",
            path.display()
        )
    });
    let blocks = read_ebnf_blocks(&src);
    let sites = extract_sites(&blocks);

    let declared: BTreeSet<String> = sites
        .iter()
        .filter(|(_, _, k)| *k == SiteKind::Lhs)
        .map(|(n, _, _)| n.clone())
        .collect();

    let allow: BTreeSet<&'static str> = TERMINAL_ALLOW_LIST.iter().copied().collect();

    // (production -> sorted list of lines where it's referenced)
    let mut unresolved: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    for (name, line, kind) in &sites {
        if *kind != SiteKind::Rhs {
            continue;
        }
        if declared.contains(name) {
            continue;
        }
        if allow.contains(name.as_str()) {
            continue;
        }
        unresolved.entry(name.clone()).or_default().push(*line);
    }

    if !unresolved.is_empty() {
        let mut lines = Vec::new();
        lines.push(format!(
            "grammar drift gate (slice 33J6): {} undeclared RHS reference(s) in `docs/reference/grammar.md`:",
            unresolved.len()
        ));
        for (name, refs) in &unresolved {
            lines.push(format!(
                "  - `{name}` referenced on lines {refs:?}. Either declare `{name} ::= ...` in grammar.md, fix the typo, or add `{name}` to TERMINAL_ALLOW_LIST in this test if it's a new lexer-emitted token."
            ));
        }
        panic!("{}", lines.join("\n"));
    }
}

#[test]
fn grammar_md_every_declared_production_is_reachable_from_program() {
    let path = grammar_md_path();
    let src = fs::read_to_string(&path).expect("read grammar.md");
    let blocks = read_ebnf_blocks(&src);
    let sites = extract_sites(&blocks);

    let declared: BTreeSet<String> = sites
        .iter()
        .filter(|(_, _, k)| *k == SiteKind::Lhs)
        .map(|(n, _, _)| n.clone())
        .collect();

    // Build (lhs -> rhs references) map.
    // For each LHS site, gather every RHS site that occurs AFTER it
    // until the next LHS site (or EOF). Simpler: bucket RHS sites by
    // the nearest preceding LHS line.
    let mut lhs_lines: Vec<(String, usize)> = sites
        .iter()
        .filter(|(_, _, k)| *k == SiteKind::Lhs)
        .map(|(n, l, _)| (n.clone(), *l))
        .collect();
    lhs_lines.sort_by_key(|(_, l)| *l);

    let mut productions: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for (n, _) in &lhs_lines {
        productions.entry(n.clone()).or_default();
    }
    for (name, line, kind) in &sites {
        if *kind != SiteKind::Rhs {
            continue;
        }
        // Find the nearest preceding LHS site.
        let owner = lhs_lines
            .iter()
            .rev()
            .find(|(_, l)| *l <= *line)
            .map(|(n, _)| n.clone());
        if let Some(owner) = owner {
            // Only include references to OTHER declared productions
            // (self-recursion is allowed; we just shouldn't crash on
            // it). Terminals and unresolved references are flagged by
            // the previous test, so here we ignore unknowns.
            if declared.contains(name) {
                productions.entry(owner).or_default().insert(name.clone());
            }
        }
    }

    // Reachability from `program` via BFS.
    assert!(
        declared.contains("program"),
        "grammar.md must declare a `program` production as the grammar root"
    );
    let mut reached: BTreeSet<String> = BTreeSet::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    queue.push_back("program".to_string());
    reached.insert("program".to_string());
    while let Some(name) = queue.pop_front() {
        if let Some(refs) = productions.get(&name) {
            for r in refs {
                if reached.insert(r.clone()) {
                    queue.push_back(r.clone());
                }
            }
        }
    }

    let orphans: BTreeSet<&String> = declared.difference(&reached).collect();
    if !orphans.is_empty() {
        let names: Vec<&String> = orphans.into_iter().collect();
        panic!(
            "grammar drift gate (slice 33J6): {} declared production(s) are unreachable from `program`: {:?}. \
             Either reference them transitively from `program` (you probably forgot to mention the new production in an `alt` \
             alternative somewhere) or delete the orphan declaration.",
            names.len(),
            names
        );
    }
}
