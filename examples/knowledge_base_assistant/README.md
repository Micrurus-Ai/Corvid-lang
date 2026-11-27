# Knowledge Base Assistant

Grounded retrieval-augmented Q&A over a small committed knowledge base.

## Setup

From this directory:

```sh
set CORVID_TEST_MOCK_TOOLS={"retrieve_context":["Refunds over one hundred dollars require approval before money moves.","Refunds over one hundred dollars require approval before money moves.","Refunds over one hundred dollars require approval before money moves."]}
set CORVID_TEST_MOCK_LLM=1
set CORVID_TEST_MOCK_LLM_RESPONSE=Refunds over one hundred dollars require approval before money moves.
cargo run -q -p corvid-cli -- check
cargo run -q -p corvid-cli -- run
```

On macOS or Linux, use `export` instead of `set`.

Expected output:

```text
"Refunds over one hundred dollars require approval before money moves."
```

## What It Shows

- The retrieval tool returns a `Grounded<String>` value from the committed
  knowledge-base seed data.
- The answer prompt accepts grounded context and preserves the source metadata
  in the returned `RagAnswer`.
- Unit, integration, eval, and replay coverage all use the same typed Corvid
  program surface.
- The adversarial checks prove a prompt-only answer cannot be laundered into a
  grounded return.
- The hardening docs under `docs/` spell out real-provider opt-in, the
  app-specific security model, and the operator runbook.

## Verify

From the repository root:

```sh
set CORVID_TEST_MOCK_TOOLS={"retrieve_context":["Refunds over one hundred dollars require approval before money moves.","Refunds over one hundred dollars require approval before money moves.","Refunds over one hundred dollars require approval before money moves."]}
set CORVID_TEST_MOCK_LLM=1
set CORVID_TEST_MOCK_LLM_RESPONSE=Refunds over one hundred dollars require approval before money moves.
cargo run -q -p corvid-cli -- test examples/knowledge_base_assistant/tests/unit.cor
cargo run -q -p corvid-cli -- test examples/knowledge_base_assistant/tests/integration.cor
cargo run -q -p corvid-cli -- test examples/knowledge_base_assistant/tests/replay_invariant.cor
cargo run -q -p corvid-cli -- eval examples/knowledge_base_assistant/evals/rag_qa_bot.cor
cargo run -q -p corvid-cli -- replay examples/knowledge_base_assistant/traces/rag_qa_bot_refund_policy.jsonl
cargo run -q -p corvid-cli -- replay examples/knowledge_base_assistant/seed/traces/rag_qa_bot_refund_policy.jsonl
```

## How To Modify

Add or edit documents under `seed/kb/`, update `seed/embeddings.json` and
`seed/mock_tools.json`, then update the tests, eval assertions, and replay
trace together. Keep mock retrieval, replay substitution, and real embedder
mode on the same `RagAnswer` and `RagChunkEnvelope` surface.
