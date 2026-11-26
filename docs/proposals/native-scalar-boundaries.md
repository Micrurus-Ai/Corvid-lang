# Proposal: text-held scalars at native tool and prompt boundaries, and in native maps

Status: **proposed 2026-11-26.** Owner: backend and AI development, macOS session. No grammar changes.

## Problem

Inside a native program a `Decimal`, `Duration`, `Instant`, `TimeZone`, `ZonedTime`, `Currency` or `Locale` is a String holding its canonical text, and every operation calls the type's crate through the runtime. At three places the native tier still sends such a program to the interpreter:

1. **A prompt whose signature holds one.** Native prompts render their arguments with a per-type function (`emit_stringify_arg`) and decode a structured reply with a hand-written decoder generated per record (`struct_decode.rs`). Neither knows the text-held types. Both are also a second copy of the JSON mapping the interpreter uses (`corvid-public-json`), which is how the `%.17g` float text went unnoticed until the number-text fix.
2. **A tool of the program's own whose signature holds one.** A native user tool is called directly through its tools-library wrapper. Its implementation would receive and return the text with nothing checking it is the type's text.
3. **A map keyed by, or holding, one.** Native maps hold `String` keys and a few value types.

(`time_now()` already runs natively: it is one of the runtime's own tools, which go through `corvid_runtime_call` since native dispatch N2.)

## Decision

**One JSON mapping at every native boundary.** Each crossing goes through the public JSON plan the compiler embeds, as runtime calls do (native dispatch N1). The decoder checks a text scalar's text with the type's own parser, so a reply or a tool result that is not the type's text is refused with the field path, on both tiers.

## Slices

1. **B1. Native prompts through the public JSON plan.** *Found while starting it (2026-11-26):* a prompt has three JSON forms today, not one. The grammar says "any typed parameter renders as its JSON form". The interpreter renders an argument with its internal `value_to_json`, describes the reply to the model with `corvid_prompt_format::schema_for` in that same internal form (an `Option` is `{"tag":"some","value":...}`), and decodes the reply with `json_to_value`. Native code inserted a `String` argument raw, without the quotes, and refused lists and records. The raw string was a plain bug against the grammar, and is fixed (B1a below). The rest of B1 is a decision with a visible effect, so it is written here before it is built. **Prompts use the public JSON form**, the one `T.to_json()` writes, for arguments, for the schema the model is shown, and for decoding the reply, on both tiers:
   - The grammar already says "its JSON form", and a model handles a value-or-`null` `Option` better than a tagged envelope.
   - It removes the last second JSON mapping.
   - It needs a public-form JSON Schema generator for IR types, which the contract's OpenAPI generator could share.
   - It changes the prompt text and schema for `Option` and variant values, so a recording made before it, with such an argument, stops replaying with a divergence naming the prompt. Learnings will say so.
   - B1b then sends native prompts through the plan: lists, records and text-held scalars as arguments and replies.

   *B1, done 2026-11-26.* Every JSON crossing between a prompt and a model on the interpreter goes through one module (`corvid-vm/src/interp/prompt/model_json.rs`): the rendered text, the recorded arguments, values shown to a judge or an adjudicator, a continuation's partial output, and a prompt function's arguments and results. Reply decoding uses the public decoder with unknown fields ignored, as `T.from_json` does. `schema_for` now describes the public form, built from shape functions in `corvid_public_json::schema` that OpenAPI calls too (its output is byte for byte what it was). Two prompt rules sit on top of the mapping: a `Grounded<T>` argument shows its `T`, and a `Partial<T>` reply keeps its per-field `complete`/`streaming` envelope with public values inside. Native code needed no change: it accepts scalars and records of scalars, where the two forms agree, and its schema comes from the same `schema_for`. Tests: `corvid-vm/src/tests/prompt_json.rs` (arguments, schema, replies, an old tagged reply refused with the field named) and the schema unit tests.

   *B1a, done 2026-11-26:* a native `String` prompt argument renders as its JSON form, quoted and escaped, at run time (`corvid_string_json_quote`) and when the arguments are constants. A parity test compares both tiers' recorded prompt text, with quotes, a backslash, a newline, a tab and a non-ASCII letter.

   The original plan for B1 follows. A prompt's arguments are encoded with `corvid_json_encode_typed`, and its reply decoded with `corvid_json_decode_typed`. The rendered prompt is then byte for byte the interpreter's, which `value_to_json` writes. This replaces `emit_stringify_arg` and the per-record decoders in `struct_decode.rs`, so the second copy of the mapping goes. Text-held types at a prompt boundary then need nothing of their own. Tests:
   - a parity test of the recorded rendered prompt for each type;
   - a reply that is not the type's text, which fails the same way on both tiers.
2. **B2. User tools through the runtime call.** This is the existing roadmap item, "move a tool of the program's own onto the runtime call". The tools library registers its implementations with the runtime, and native code calls them through `corvid_runtime_call`. Then the arguments are encoded with the public mapping, a `Secret` is redacted and revealed, the result is decoded and checked, and the trace comes from the one path the interpreter uses. It settles text scalars and secrets at user-tool boundaries together. The direct-call wrappers and their separate trace emission (`corvid_trace_tool_call`) go.
3. **B3. Native maps with text-held keys and values.** A key compares and hashes by value through the type's crate (`1.0` and `1.00` are one key), as the interpreter's map does. Values are reference-counted slots, as `String` values are.

## Alternatives considered

- **Check the text at each bridge with the type's parser.** This is smaller per slice, but it would keep two JSON mappings and add a third piece that has to agree with them.
- **Keep the refusal.** It is honest, but it leaves every program that prices or schedules with a prompt or a tool on the interpreter.

## Across the tiers

- **Backend:** a pricing or scheduling program with prompts and tools builds natively.
- **AI:** the model sees exactly the interpreter's prompt text, and the trace records the same text from either tier.
- **Frontend:** nothing changes.

## Not in scope

- Streams, which native code does not have yet.
