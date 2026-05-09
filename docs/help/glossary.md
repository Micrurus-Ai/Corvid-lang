# Glossary

**Agent.** A Corvid function that composes prompts and tools and is
the unit `corvid run` and `corvid build` operate on.

**Approve.** A compile-time token that authorizes a call to a tool
whose effect row carries `trust: supervisor_required`. Lexically
scoped.

**Audit log.** The append-only log of `approve` decisions, grounded
unwraps, tool calls, and budget events produced by every agent run.

**Budget.** A compile-time cost ceiling on an agent (`@budget($X)`).
The compiler refuses to emit if worst-case composed cost exceeds X.

**cdylib.** A shared library produced by `corvid build --target=cdylib`
or as the output of a signed build. Carries the embedded ABI
descriptor and signed-claim block.

**Compile-time guarantee.** A property the compiler statically refuses
to violate. Catalogued in `corvid contract list`. Each row has an id,
a class (Static / RuntimeChecked / OutOfScope), and adversarial test
references.

**Connector.** A typed surface to an external service (Gmail, Slack,
MS365, etc.) defined in `std.connectors.*`. Connector calls carry
effect rows and replay-quarantine tags.

**Effect.** A named declaration of cost, latency, confidence, trust,
reversibility, and data dimensions. Composed via `uses` into prompts,
tools, and agents. See **[Effects](/docs/effects)**.

**Effect row.** The set of effects on a function's signature. The
compiler computes the row's closure across the function's call graph
and uses it for budget, approval, and grounding decisions.

**Grounded.** A type wrapper (`Grounded<T>`) that carries provenance.
The compiler refuses silent unwrap.

**Guarantee id.** The string id of a compile-time guarantee in the
registry, e.g. `approval.dangerous_call_requires_token`. Returned by
diagnostic messages so you can `corvid claim --explain <id>` for
context.

**Invention.** A Corvid-specific language/runtime capability we'd
name in the README, the website, or launch material. Catalogued in
[`docs/reference/inventions.md`](https://github.com/Corvid-lang/Corvid-lang/blob/main/docs/reference/inventions.md)
with shipped status, runnable command, test coverage, spec link.

**Job.** A durable agent run managed by the Phase 38 runner, with
retry, idempotency, lease, and replay semantics.

**Prompt.** A function whose body is a string template, whose return
type drives JSON-schema-shaped decode, and whose effect row carries
the LLM call's cost, latency, confidence.

**Provenance.** The (source, timestamp, content-hash) triple a
`Grounded<T>` carries. Survives composition via `Grounded.merge`.

**Receipt.** A DSSE-signed bundle of a run's audit log, prompt
records, tool calls, and trace metadata. `corvid receipt verify`
validates a third-party receipt.

**Registry.** The in-code source of truth for compile-time guarantees
(`crates/corvid-guarantees/src/registry.rs`). Read by `corvid
contract list`, the spec doc generator, the bilateral verifier, and
the signed-claim coverage validator.

**Replay.** Deterministic re-execution of a recorded agent run.
`corvid replay <trace-id>` reproduces it byte-for-byte without
hitting the LLM provider or the side-effect tools.

**Tool.** A side-effecting function whose body is a host call (`@host.x.y`).
Carries an effect row that the compiler uses for approval and
reversibility decisions.

**Trace.** A structured record of an agent run: every prompt, every
tool call, every approve, every grounded unwrap, every budget event.
The input to `corvid replay`, `corvid trace dag`, and the eval
surface.
