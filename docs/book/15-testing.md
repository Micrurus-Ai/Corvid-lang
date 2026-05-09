# Testing

## Test types

Corvid ships four test surfaces:

- **`test`** — unit tests. Run via `corvid test`.
- **`eval`** — model-quality tests against saved traces. Run via
  `corvid eval`.
- **`fixture`** — reusable test data and setup helpers.
- **`mock`** — host-call mocks for tools and connectors.

## Writing a unit test

```corvid
# tests/refund_test.cor
import refund

test refund_within_policy:
    let result = refund.refund_logic(50.0, "cust_123")
    assert result == Ok(Unit)

test refund_over_policy_limit:
    let result = refund.refund_logic(5000.0, "cust_123")
    assert result == Err(RefundError::OverDailyLimit)
```

Run:

```sh
corvid test
corvid test --filter refund        # only tests whose name matches
corvid test --watch                # rerun on file change
```

## Mocking tools

```corvid
mock payment_mock:
    on @host.payment.refund(_, _) -> "refund_ok_mock"

test refund_calls_payment:
    use payment_mock
    let result = refund(50.0, "cust_123")
    assert result == "refund_ok_mock"
    assert calls(@host.payment.refund) == 1
```

The `calls()` helper asserts the number of times a host call was
invoked. The mock surface integrates with the source-bypass corpus —
attempting to bypass `approve` via a mock fails compile-time the same
way it fails in production code.

## Fixtures

```corvid
fixture sample_ticket:
    Ticket {
        id: "tkt_42",
        text: "I never received my order",
        customer_id: "cust_123",
    }

test handles_sample_ticket:
    use sample_ticket as ticket
    let decision = process(ticket)
    assert decision.refund == true
```

## Eval tests

Eval tests rerun a saved trace against the current code and assert on
output stability:

```corvid
eval refund_quality:
    source app.cor
    trace_dir target/refund_traces
    swap_model gpt-5
    assert_no_regression on outcome
    assert_cost_within $0.10 per_run
```

Run:

```sh
corvid eval refund_quality
corvid eval refund_quality --swap-model gpt-5
```

## Snapshots

```corvid
test prompt_output_stable:
    let result = summarize("hello world")
    assert_snapshot "summarize_hello.snap"
```

The snapshot file is committed; subsequent runs diff against it. Use
`corvid test --update-snapshots` to refresh after intentional changes.

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
