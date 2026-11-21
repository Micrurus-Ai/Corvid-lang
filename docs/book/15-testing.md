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

Use a mock to give a prompt a predictable result in an ordinary application
test. The checker verifies the mock against the prompt's signature.

```corvid
type Decision:
    label: String

prompt classify(text: String) -> Decision:
    "Classify {text}."

mock classify(text: String) -> Decision:
    return Decision("review " + text)

agent label(text: String) -> String:
    decision = classify(text)
    return decision.label

agent main() -> String:
    return label("ticket")

test classify_ticket:
    assert label("ticket") == "review ticket"
```

Run `corvid test main.cor` with no provider credentials. The agent executes
its own code, and its call to `classify` executes the mock body.

### Test a recorded provider conversation

A test declared with `from_trace` reads a JSONL fixture relative to the
source file. Its setup and assertions consume the recorded provider
responses, including multi-turn function calls. The recording takes
precedence over source mocks in that test. Each repetition of a statistical
assertion starts with a fresh replay cursor.

To record without credentials, set `CORVID_MODEL=offline` and
`CORVID_TEST_MOCK_LLM=1` for the recording process. Set
`CORVID_TEST_MOCK_LLM_REPLIES` to a JSON object keyed by prompt name.
For example, `{"classify":[{"label":"urgent"}]}` supplies one structured
reply. An array of replies for the same prompt is consumed in order.
Run `corvid run --target interp main.cor`. Copy the resulting JSONL
file from `target/trace` next to the test source as `recording.jsonl`.
For the example above, add this test:

```corvid-fragment
test recorded_ticket from_trace "recording.jsonl":
    result = label("ticket")
    assert result == "urgent"
    assert called classify
```

Run `corvid test main.cor` in a fresh process
without the recording environment. A missing fixture or changed model
input fails the test.

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
