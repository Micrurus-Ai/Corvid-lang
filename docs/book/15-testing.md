# Testing

Every `corvid`-tagged block compiles through the real driver in CI.

## Test surfaces

Corvid ships four declaration forms:

- **`test`** — unit tests. Run via `corvid test`.
- **`eval`** — model-quality checks; trace-aware assertion support.
  Run via `corvid eval`.
- **`fixture`** — typed, reusable test helpers (params + return type).
- **`mock`** — tool/prompt replacement for tests (see the known
  defect below).

## Writing a unit test

A test body is statements plus `assert` lines:

```corvid
agent refund_allowed(amount: Float) -> Bool:
    if amount > 100.0:
        return false
    return true

test refund_within_policy:
    result = refund_allowed(50.0)
    assert result == true

test refund_over_policy_limit:
    result = refund_allowed(5000.0)
    assert result == false
```

Run:

```sh
corvid test
corvid test --filter refund        # only tests whose name matches
```

## Fixtures

Fixtures are typed: a parameter list and a return type, called like
a function from test bodies:

```corvid
fixture sample_amount() -> Float:
    return 50.0

agent refund_allowed(amount: Float) -> Bool:
    if amount > 100.0:
        return false
    return true

test handles_sample_amount:
    amount = sample_amount()
    assert refund_allowed(amount) == true
```

## Trace-bound tests

A test can bind a recorded trace with `from_trace` — the runner
replays the recorded LLM responses and tool results instead of
hitting providers:

```corvid
agent always_true() -> Bool:
    return true

test replayed_regression from_trace "traces/golden.trace":
    assert always_true() == true
```

## Eval tests

An `eval` is statements plus assertions; `corvid eval` runs them with
model-quality tooling on top (`--golden-traces`, `--swap-model`,
`--max-spend` are CLI flags, not language syntax):

```corvid
agent always_refund() -> Bool:
    return true

eval refund_accuracy:
    result = always_refund()
    assert result == true
```

```sh
corvid eval
corvid eval --swap-model <model>   # diff behavior against the baseline
```

## Quality assertions

Two assertion forms for prompt-quality regressions (slice 46h):

```corvid-fragment
eval description_quality:
    d = describe()
    assert similar d, "an AI-native language with typed effects" min 0.7
    assert judged d, "is accurate and mentions the language name" min 0.8
```

`assert similar` is a deterministic word-set similarity — zero LLM
cost, the cheap regression gate. `assert judged` sends the value
and your criteria to an LLM judge scoring 0..1; the judge call
flows through the normal traced, cost-accounted LLM path, so eval
`--max-spend` sees it and failures print the score. Both take a
`min` threshold in 0..=1, compile-checked at parse.

## Snapshots

```corvid
agent double_it(x: Int) -> Int:
    return x * 2

test output_stable:
    r = double_it(21)
    assert_snapshot "double_21.snap"
```

The snapshot file is committed; subsequent runs diff against it.

## Mocks

A mock names the tool or prompt it replaces and provides a body with
the same signature:

```corvid-fragment
mock summarize(text: String) -> String:
    return "mocked summary"
```

A mock's target may be a tool or a prompt; the checker verifies
the mock's signature matches the target's declaration (fixed in
slice 45q — prompt targets used to be rejected with E0203).

## CI integration

```yaml
# .github/workflows/ci.yml
- run: corvid check
- run: corvid test
- run: corvid eval
- run: corvid contract list --check-against=committed
```

The last command is the drift-gate that catches an unannounced
guarantee registry change.
