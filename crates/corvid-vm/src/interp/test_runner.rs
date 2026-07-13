use super::{Flow, Interpreter};
use super::test_trace::{evaluate_trace_assertion, load_trace_fixture, TraceFixture};
use crate::conv::value_to_json;
use crate::errors::{InterpError, InterpErrorKind};
use crate::value::Value;
use corvid_ast::Span;
use corvid_ir::{IrEvalAssert, IrExpr, IrFile, IrTest};
use corvid_runtime::Runtime;
use std::path::{Path, PathBuf};

/// Execute one lowered `test` declaration and evaluate its assertions.
///
/// This intentionally lives beside the interpreter rather than in the driver:
/// setup bodies and assertion expressions must use the same evaluator as
/// agents, including tool/prompt dispatch and runtime errors.
pub async fn run_test(
    ir: &IrFile,
    test_name: &str,
    runtime: &Runtime,
) -> Result<TestExecution, InterpError> {
    let test = ir
        .tests
        .iter()
        .find(|test| test.name == test_name)
        .ok_or_else(|| {
            InterpError::new(
                InterpErrorKind::DispatchFailed(format!("no test named `{test_name}`")),
                Span::new(0, 0),
            )
        })?;
    run_test_decl(ir, test, runtime, &TestRunOptions::default()).await
}

pub async fn run_all_tests(ir: &IrFile, runtime: &Runtime) -> Vec<TestExecution> {
    run_all_tests_with_options(ir, runtime, TestRunOptions::default()).await
}

pub async fn run_all_tests_with_options(
    ir: &IrFile,
    runtime: &Runtime,
    options: TestRunOptions,
) -> Vec<TestExecution> {
    let mut results = Vec::with_capacity(ir.tests.len());
    for test in &ir.tests {
        match run_test_decl(ir, test, runtime, &options).await {
            Ok(result) => results.push(result),
            Err(error) => results.push(TestExecution {
                name: test.name.clone(),
                assertions: Vec::new(),
                setup_error: Some(error.to_string()),
            }),
        }
    }
    results
}

async fn run_test_decl(
    ir: &IrFile,
    test: &IrTest,
    runtime: &Runtime,
    options: &TestRunOptions,
) -> Result<TestExecution, InterpError> {
    let mut interp = Interpreter::new(ir, runtime).with_mocks();
    run_test_setup(&mut interp, test).await?;
    let trace_fixture = load_test_trace_fixture(test, options);

    let mut assertions = Vec::with_capacity(test.assertions.len());
    for (index, assertion) in test.assertions.iter().enumerate() {
        assertions.push(
            eval_test_assertion(
                ir,
                test,
                runtime,
                &mut interp,
                assertion,
                index,
                options,
                trace_fixture.as_ref(),
            )
            .await,
        );
    }
    Ok(TestExecution {
        name: test.name.clone(),
        assertions,
        setup_error: None,
    })
}

async fn run_test_setup<'ir>(
    interp: &mut Interpreter<'ir>,
    test: &'ir IrTest,
) -> Result<(), InterpError> {
    match interp.eval_block(&test.body).await? {
        Flow::Normal => Ok(()),
        Flow::Return(_) => Err(InterpError::new(
            InterpErrorKind::Other("test setup returned before assertions".into()),
            test.span,
        )),
        Flow::Break | Flow::Continue => Err(InterpError::new(
            InterpErrorKind::Other("loop control flow escaped test setup".into()),
            test.span,
        )),
    }
}

async fn eval_test_assertion<'ir>(
    ir: &'ir IrFile,
    test: &'ir IrTest,
    runtime: &'ir Runtime,
    interp: &mut Interpreter<'ir>,
    assertion: &'ir IrEvalAssert,
    assertion_index: usize,
    options: &TestRunOptions,
    trace_fixture: Option<&Result<TraceFixture, String>>,
) -> TestAssertionExecution {
    match assertion {
        IrEvalAssert::Value {
            expr,
            confidence,
            runs,
            ..
        } => {
            if confidence.is_some() || runs.is_some() {
                eval_statistical_value_assertion(ir, test, runtime, assertion).await
            } else {
                eval_value_assertion(interp, expr, assertion_label(assertion)).await
            }
        }
        IrEvalAssert::Snapshot { expr, .. } => {
            eval_snapshot_assertion(interp, expr, assertion_label(assertion), &test.name, assertion_index, options).await
        }
        IrEvalAssert::Similar {
            expr,
            expected,
            min,
            ..
        } => eval_similar_assertion(interp, expr, expected, *min, assertion_label(assertion)).await,
        IrEvalAssert::Judged {
            expr,
            criteria,
            min,
            ..
        } => {
            eval_judged_assertion(interp, runtime, expr, criteria, *min, assertion_label(assertion))
                .await
        }
        IrEvalAssert::Called { .. }
        | IrEvalAssert::Approved { .. }
        | IrEvalAssert::Cost { .. }
        | IrEvalAssert::Ordering { .. } => eval_trace_assertion(assertion, trace_fixture),
    }
}

/// Slice 46h: deterministic word-set similarity. Both values render
/// to text; lowercase alphanumeric word sets compare by Jaccard
/// index. No LLM cost — the cheap regression gate.
async fn eval_similar_assertion<'ir>(
    interp: &mut Interpreter<'ir>,
    expr: &'ir IrExpr,
    expected: &'ir IrExpr,
    min: f64,
    label: String,
) -> TestAssertionExecution {
    let render = |v: &Value| match v {
        Value::String(s) => s.to_string(),
        other => crate::conv::value_to_json(other).to_string(),
    };
    let actual = match interp.eval_expr(expr).await.map(|f| f.into_value()) {
        Ok(Ok(v)) | Ok(Err(v)) => render(&v),
        Err(e) => {
            return TestAssertionExecution {
                label,
                status: TestAssertionStatus::Error,
                message: Some(e.to_string()),
            }
        }
    };
    let wanted = match interp.eval_expr(expected).await.map(|f| f.into_value()) {
        Ok(Ok(v)) | Ok(Err(v)) => render(&v),
        Err(e) => {
            return TestAssertionExecution {
                label,
                status: TestAssertionStatus::Error,
                message: Some(e.to_string()),
            }
        }
    };
    let score = word_jaccard(&actual, &wanted);
    if score >= min {
        TestAssertionExecution {
            label,
            status: TestAssertionStatus::Passed,
            message: None,
        }
    } else {
        TestAssertionExecution {
            label,
            status: TestAssertionStatus::Failed,
            message: Some(format!(
                "similarity {score:.3} is below min {min} (actual: {actual:?}, expected: {wanted:?})"
            )),
        }
    }
}

fn word_jaccard(a: &str, b: &str) -> f64 {
    let words = |s: &str| -> std::collections::HashSet<String> {
        s.split(|c: char| !c.is_alphanumeric())
            .filter(|w| !w.is_empty())
            .map(|w| w.to_lowercase())
            .collect()
    };
    let (wa, wb) = (words(a), words(b));
    if wa.is_empty() && wb.is_empty() {
        return 1.0;
    }
    let intersection = wa.intersection(&wb).count() as f64;
    let union = wa.union(&wb).count() as f64;
    intersection / union
}

/// Slice 46h: LLM-judge assertion. The judge call flows through
/// the NORMAL LLM path — traced, cost-accounted, mockable — so
/// eval `--max-spend` accounting and replay both see it.
async fn eval_judged_assertion<'ir>(
    interp: &mut Interpreter<'ir>,
    runtime: &'ir Runtime,
    expr: &'ir IrExpr,
    criteria: &str,
    min: f64,
    label: String,
) -> TestAssertionExecution {
    let value = match interp.eval_expr(expr).await.map(|f| f.into_value()) {
        Ok(Ok(v)) | Ok(Err(v)) => v,
        Err(e) => {
            return TestAssertionExecution {
                label,
                status: TestAssertionStatus::Error,
                message: Some(e.to_string()),
            }
        }
    };
    let rendered_value = match &value {
        Value::String(s) => s.to_string(),
        other => crate::conv::value_to_json(other).to_string(),
    };
    let req = corvid_runtime::llm::LlmRequest {
        prompt: "__corvid_eval_judge".to_string(),
        model: String::new(),
        rendered: format!(
            "You are an evaluation judge. Score how well the VALUE satisfies the CRITERIA on a scale from 0.0 (not at all) to 1.0 (perfectly).\n\nCRITERIA: {criteria}\n\nVALUE:\n{rendered_value}\n\nRespond with the score."
        ),
        args: vec![serde_json::json!(criteria), serde_json::json!(rendered_value)],
        output_schema: Some(serde_json::json!({
            "type": "object",
            "properties": {"score": {"type": "number"}},
            "required": ["score"],
        })),
        sampling: Default::default(),
        messages: Vec::new(),
    };
    let resp = match runtime.call_llm(req).await {
        Ok(r) => r,
        Err(e) => {
            return TestAssertionExecution {
                label,
                status: TestAssertionStatus::Error,
                message: Some(format!("judge call failed: {e}")),
            }
        }
    };
    let score = resp.value["score"]
        .as_f64()
        .or_else(|| resp.value.as_f64());
    let Some(score) = score else {
        return TestAssertionExecution {
            label,
            status: TestAssertionStatus::Error,
            message: Some(format!(
                "judge returned no numeric score: {}",
                resp.value
            )),
        };
    };
    if score >= min {
        TestAssertionExecution {
            label,
            status: TestAssertionStatus::Passed,
            message: None,
        }
    } else {
        TestAssertionExecution {
            label,
            status: TestAssertionStatus::Failed,
            message: Some(format!("judge score {score:.3} is below min {min}")),
        }
    }
}

fn load_test_trace_fixture(
    test: &IrTest,
    options: &TestRunOptions,
) -> Option<Result<TraceFixture, String>> {
    let spec = test.trace_fixture.as_deref()?;
    let root = options
        .trace_fixtures
        .as_ref()
        .map(|options| options.root.as_path())
        .unwrap_or_else(|| Path::new("."));
    Some(load_trace_fixture(spec, root))
}

fn eval_trace_assertion(
    assertion: &IrEvalAssert,
    trace_fixture: Option<&Result<TraceFixture, String>>,
) -> TestAssertionExecution {
    let label = assertion_label(assertion);
    let Some(trace_fixture) = trace_fixture else {
        return TestAssertionExecution {
            label,
            status: TestAssertionStatus::Unsupported,
            message: Some(
                "trace assertions require `test name from_trace \"trace.jsonl\":`".into(),
            ),
        };
    };
    let fixture = match trace_fixture {
        Ok(fixture) => fixture,
        Err(error) => {
            return TestAssertionExecution {
                label,
                status: TestAssertionStatus::Error,
                message: Some(error.clone()),
            };
        }
    };
    match evaluate_trace_assertion(assertion, fixture) {
        Ok(message) => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Passed,
            message,
        },
        Err(message) => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Failed,
            message: Some(message),
        },
    }
}

async fn eval_snapshot_assertion<'ir>(
    interp: &mut Interpreter<'ir>,
    expr: &'ir IrExpr,
    label: String,
    test_name: &str,
    assertion_index: usize,
    options: &TestRunOptions,
) -> TestAssertionExecution {
    let Some(snapshot_options) = &options.snapshots else {
        return TestAssertionExecution {
            label,
            status: TestAssertionStatus::Unsupported,
            message: Some("snapshot assertions require file-backed test options".into()),
        };
    };
    let value = match interp.eval_expr(expr).await {
        Ok(flow) => match flow.into_value() {
            Ok(value) | Err(value) => value,
        },
        Err(error) => {
            return TestAssertionExecution {
                label,
                status: TestAssertionStatus::Error,
                message: Some(error.to_string()),
            };
        }
    };
    let actual = snapshot_text(&value);
    let path = snapshot_path(&snapshot_options.root, test_name, assertion_index);
    if let Err(error) = std::fs::create_dir_all(&snapshot_options.root) {
        return TestAssertionExecution {
            label,
            status: TestAssertionStatus::Error,
            message: Some(format!(
                "failed to create snapshot directory `{}`: {error}",
                snapshot_options.root.display()
            )),
        };
    }
    match std::fs::read_to_string(&path) {
        Ok(expected) if expected == actual => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Passed,
            message: Some(format!("matched `{}`", path.display())),
        },
        Ok(_) if snapshot_options.update => match std::fs::write(&path, actual.as_bytes()) {
            Ok(()) => TestAssertionExecution {
                label,
                status: TestAssertionStatus::Updated,
                message: Some(format!("updated `{}`", path.display())),
            },
            Err(error) => TestAssertionExecution {
                label,
                status: TestAssertionStatus::Error,
                message: Some(format!("failed to update `{}`: {error}", path.display())),
            },
        },
        Ok(expected) => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Failed,
            message: Some(format!(
                "snapshot mismatch at `{}`\n{}",
                path.display(),
                snapshot_diff(&expected, &actual)
            )),
        },
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            match std::fs::write(&path, actual.as_bytes()) {
                Ok(()) => TestAssertionExecution {
                    label,
                    status: TestAssertionStatus::Updated,
                    message: Some(format!("created `{}`", path.display())),
                },
                Err(error) => TestAssertionExecution {
                    label,
                    status: TestAssertionStatus::Error,
                    message: Some(format!("failed to write `{}`: {error}", path.display())),
                },
            }
        }
        Err(error) => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Error,
            message: Some(format!("failed to read `{}`: {error}", path.display())),
        },
    }
}

async fn eval_statistical_value_assertion<'ir>(
    ir: &'ir IrFile,
    test: &'ir IrTest,
    runtime: &'ir Runtime,
    assertion: &'ir IrEvalAssert,
) -> TestAssertionExecution {
    let IrEvalAssert::Value {
        expr,
        confidence,
        runs,
        ..
    } = assertion else {
        unreachable!("caller only passes value assertions");
    };
    let runs = runs.unwrap_or(1);
    let confidence = confidence.unwrap_or(1.0);
    let mut passed = 0_u64;
    for _ in 0..runs {
        let mut fresh = Interpreter::new(ir, runtime).with_mocks();
        if let Err(error) = run_test_setup(&mut fresh, test).await {
            return TestAssertionExecution {
                label: assertion_label(assertion),
                status: TestAssertionStatus::Error,
                message: Some(format!("statistical setup failed: {error}")),
            };
        }
        match eval_bool_assertion(&mut fresh, expr).await {
            Ok(true) => passed += 1,
            Ok(false) => {}
            Err(error) => {
                return TestAssertionExecution {
                    label: assertion_label(assertion),
                    status: TestAssertionStatus::Error,
                    message: Some(error.to_string()),
                };
            }
        }
    }
    let observed = passed as f64 / runs as f64;
    if observed >= confidence {
        TestAssertionExecution {
            label: assertion_label(assertion),
            status: TestAssertionStatus::Passed,
            message: Some(format!(
                "{passed}/{runs} runs passed; observed confidence {observed:.3} >= required {confidence:.3}"
            )),
        }
    } else {
        TestAssertionExecution {
            label: assertion_label(assertion),
            status: TestAssertionStatus::Failed,
            message: Some(format!(
                "{passed}/{runs} runs passed; observed confidence {observed:.3} < required {confidence:.3}"
            )),
        }
    }
}

async fn eval_value_assertion<'ir>(
    interp: &mut Interpreter<'ir>,
    expr: &'ir IrExpr,
    label: String,
) -> TestAssertionExecution {
    match eval_bool_assertion(interp, expr).await {
        Ok(true) => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Passed,
            message: None,
        },
        Ok(false) => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Failed,
            message: Some("assertion evaluated to false".into()),
        },
        Err(error) => TestAssertionExecution {
            label,
            status: TestAssertionStatus::Error,
            message: Some(error.to_string()),
        },
    }
}

async fn eval_bool_assertion<'ir>(
    interp: &mut Interpreter<'ir>,
    expr: &'ir IrExpr,
) -> Result<bool, InterpError> {
    let value = match interp.eval_expr(expr).await?.into_value() {
        Ok(value) | Err(value) => value,
    };
    match value {
        Value::Bool(value) => Ok(value),
        other => Err(InterpError::new(
            InterpErrorKind::TypeMismatch {
                expected: "Bool".into(),
                got: other.type_name(),
            },
            expr.span,
        )),
    }
}

fn assertion_label(assertion: &IrEvalAssert) -> String {
    match assertion {
        IrEvalAssert::Value { .. } => "assert <expr>".into(),
        IrEvalAssert::Similar { min, .. } => format!("assert similar ... min {min}"),
        IrEvalAssert::Judged { criteria, min, .. } => {
            format!("assert judged {criteria:?} min {min}")
        }
        IrEvalAssert::Snapshot { .. } => "assert_snapshot <expr>".into(),
        IrEvalAssert::Called { name, .. } => format!("assert called {name}"),
        IrEvalAssert::Approved { label, .. } => format!("assert approved {label}"),
        IrEvalAssert::Cost { bound, .. } => format!("assert cost < {bound}"),
        IrEvalAssert::Ordering {
            before_name,
            after_name,
            ..
        } => format!("assert called {before_name} before {after_name}"),
    }
}

fn snapshot_text(value: &Value) -> String {
    let json = value_to_json(value);
    let mut text = serde_json::to_string_pretty(&json).unwrap_or_else(|_| "null".into());
    text.push('\n');
    text
}

fn snapshot_path(root: &Path, test_name: &str, assertion_index: usize) -> PathBuf {
    root.join(format!(
        "{}__{:03}.snap",
        sanitize_snapshot_segment(test_name),
        assertion_index + 1
    ))
}

fn sanitize_snapshot_segment(raw: &str) -> String {
    let sanitized: String = raw
        .chars()
        .map(|ch| if ch.is_ascii_alphanumeric() || ch == '_' || ch == '-' { ch } else { '_' })
        .collect();
    if sanitized.is_empty() {
        "snapshot".into()
    } else {
        sanitized
    }
}

fn snapshot_diff(expected: &str, actual: &str) -> String {
    let expected_lines: Vec<&str> = expected.lines().collect();
    let actual_lines: Vec<&str> = actual.lines().collect();
    let max = expected_lines.len().max(actual_lines.len());
    let mut out = String::from("--- expected\n+++ actual\n");
    for index in 0..max {
        match (expected_lines.get(index), actual_lines.get(index)) {
            (Some(left), Some(right)) if left == right => {}
            (Some(left), Some(right)) => {
                out.push_str(&format!("- {left}\n+ {right}\n"));
            }
            (Some(left), None) => out.push_str(&format!("- {left}\n")),
            (None, Some(right)) => out.push_str(&format!("+ {right}\n")),
            (None, None) => {}
        }
    }
    out
}

#[derive(Debug, Clone, Default)]
pub struct TestRunOptions {
    pub snapshots: Option<SnapshotOptions>,
    pub trace_fixtures: Option<TraceFixtureOptions>,
}

#[derive(Debug, Clone)]
pub struct SnapshotOptions {
    pub root: PathBuf,
    pub update: bool,
}

#[derive(Debug, Clone)]
pub struct TraceFixtureOptions {
    pub root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestExecution {
    pub name: String,
    pub assertions: Vec<TestAssertionExecution>,
    pub setup_error: Option<String>,
}

impl TestExecution {
    pub fn passed(&self) -> bool {
        self.setup_error.is_none()
            && self
                .assertions
                .iter()
                .all(|assertion| {
                    matches!(
                        assertion.status,
                        TestAssertionStatus::Passed | TestAssertionStatus::Updated
                    )
                })
    }

    pub fn updated_snapshot_count(&self) -> usize {
        self.assertions
            .iter()
            .filter(|assertion| assertion.status == TestAssertionStatus::Updated)
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TestAssertionExecution {
    pub label: String,
    pub status: TestAssertionStatus,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TestAssertionStatus {
    Passed,
    Updated,
    Failed,
    Error,
    Unsupported,
}

#[cfg(test)]
mod tests {
    use super::*;
    use corvid_ir::{lower, IrFile};
    use corvid_resolve::resolve;
    use corvid_runtime::Runtime;
    use corvid_syntax::{lex, parse_file};
    use corvid_types::typecheck;

    fn lower_src(src: &str) -> IrFile {
        let tokens = lex(src).expect("lex");
        let (file, parse_errors) = parse_file(&tokens);
        assert!(parse_errors.is_empty(), "parse errors: {parse_errors:?}");
        let resolved = resolve(&file);
        assert!(
            resolved.errors.is_empty(),
            "resolve errors: {:?}",
            resolved.errors
        );
        let checked = typecheck(&file, &resolved);
        assert!(
            checked.errors.is_empty(),
            "type errors: {:?}",
            checked.errors
        );
        lower(&file, &resolved, &checked)
    }

    #[tokio::test]
    async fn fixtures_and_mocks_execute_inside_test_runner() {
        let ir = lower_src(
            r#"
tool lookup_score(id: String) -> Int

fixture order_id() -> String:
    return "ord_42"

mock lookup_score(id: String) -> Int:
    if id == "ord_42":
        return 42
    return 0

test mocked_tool_contract:
    score = lookup_score(order_id())
    assert score == 42
"#,
        );
        let runtime = Runtime::builder().build();
        let results = run_all_tests(&ir, &runtime).await;
        assert_eq!(results.len(), 1);
        assert!(results[0].passed(), "result: {:?}", results[0]);
    }
}
