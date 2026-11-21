# Model Provider Routing Benchmark Notes

This demo is performance-relevant at the routing boundary. It records which
provider route is selected and preserves the runtime cost estimate in trace
metadata so routing-report and cost-frontier tooling can inspect provider
decisions later.

Local smoke measurements should focus on:

- `corvid run` with `CORVID_TEST_MOCK_LLM=1`
- `corvid test examples/model_provider_routing/tests/unit.cor`
- `corvid eval examples/model_provider_routing/evals/model_provider_routing.cor`
- `corvid replay` for each provider trace fixture

The committed mock fixtures do not measure provider latency or quality. Real
provider measurements should record model, provider, token counts, latency, and
cost separately from CI fixtures so benchmark changes do not make CI
nondeterministic.
