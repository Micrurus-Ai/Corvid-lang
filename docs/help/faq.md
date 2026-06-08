# FAQ

## Is Corvid a Python wrapper?

No. Corvid is its own language with its own compiler (Cranelift
backend), runtime, type system, and stdlib. You can call into Python
via the Phase 30 FFI when you need numpy or sklearn, but the language
core is independent.

## What's the runtime overhead vs. Python LangChain?

On orchestration-only benches (no model call), Corvid is roughly
25–36× faster than Python and 1.7–2.6× slower than TypeScript on the
canonical reference apps. The numbers were published in Phase 17
deliberately as "honest slower" — Corvid is not optimized for raw
throughput; it is optimized for the AI-correctness surface. See the
[benchmarks](https://corvid.dev/benchmarks) page for the full numbers.

## Does the compiler need network access?

No. Compilation is local. Network access is only for runtime LLM calls
(when you actually run an agent), and even then `corvid replay`
reproduces a recorded run with no network.

## Which LLM providers are supported?

The model substrate (Phase 31) ships adapters for OpenAI, Anthropic,
Google, AWS Bedrock, Cohere, and local models via Ollama. Provider
selection happens at deploy time, not in source code. New providers
are pluggable via the typed adapter trait.

## Can I use Corvid for non-AI code?

You can, but it's not the point. Corvid's value is the AI-orchestration
surface. For tight numeric loops, systems code, or general-purpose
backend code, use the language that fits best — and call Corvid
through the cdylib FFI when you cross into the AI surface.

## What happens if my LLM provider changes its API?

The model substrate is the boundary. Adapter changes go in the
substrate; your source code does not change. The eval surface
(`corvid eval --swap-model`) verifies whether the change altered
behavior on your saved traces.

## Is `approve` enforceable across module boundaries?

Yes. The compiler resolves call graphs across modules. An imported
tool whose effect row carries `trust: supervisor_required` requires
an `approve` at every call site, regardless of where the tool is
defined.

## Can I bypass `approve` with metaprogramming?

The source-fuzz corpus
([`crates/corvid-types/tests/source_bypass_corpus.rs`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/crates/corvid-types/tests/source_bypass_corpus.rs))
exercises four classes of attempted bypass. Each fails to compile
with the matching `guarantee_id`. New bypass attempts are welcome;
file them as adversarial test cases.

## What about formal verification?

Corvid v1.0 ships engineering-grade compile-time guarantees: a
registry of properties, adversarial tests for each, a separate-binary
ABI descriptor verifier, and a CI gate. Formal mechanized proof of
the type system is a post-v1.0 research agenda. The
[security model](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/docs/security/model.md)
is explicit about what's verified by tests vs. what's verified by
proof.

## How does this relate to DSPy / LangChain / Pydantic-AI?

Those are libraries. Corvid is a language. They patch over the
problems Corvid solves at the type-system level. If you are already
hitting the limits of Python's type system on your AI code, you are
the audience.

## Is the runtime open source?

Yes. MIT-licensed.
[`Micrurus-Ai/Corvid-lang`](https://github.com/Micrurus-Ai/Corvid-lang)
on GitHub.

## What's the road from here to v2?

v1.0 is the launch. The post-v1.0 research agenda includes formal
proof of the type system, true second-implementation TCB shrinkage,
multi-process worker pools for the rendered backend server, custom
middleware injection from Corvid source, and additional connector
families. The roadmap lives in
[`ROADMAP.md`](https://github.com/Micrurus-Ai/Corvid-lang/blob/main/ROADMAP.md)
in the repo.
