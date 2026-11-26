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

   *B1b, done 2026-11-26.* Native prompts take and return any type with a native JSON form: records, lists, `Option`, `Result`, sum types and text-held scalars. Each argument is encoded once through the plan the compiler embeds (`lower_json_encode`); the text fills the template and is the recorded argument, tagged as JSON for the trace. A non-scalar reply goes through a new bridge, `corvid_prompt_call_typed`, which sends what the interpreter sends (the rendered prompt and `schema_for`'s schema as structured output, no text instructions) and reads the reply with the public decoder, ignoring undeclared keys, through the same plan. Record replies move onto it, so `struct_decode.rs` now serves only the C ABI, and the old record bridge, the per-type argument renderer for prompts and the C string quoter are gone. Scalar replies keep their bridges, whose answers already agree. Found while building: the interpreter's citation check compared a string context in its JSON quotes, so a short context never matched a longer reply that contained it; it now compares a string's own words, as native code does. Tests: `crates/corvid-driver/tests/native_prompt_json.rs` runs one program on both tiers and compares the value, the rendered text, the recorded arguments and the recorded reply, and refuses a reply missing a field on both.

   *B1, done 2026-11-26.* Every JSON crossing between a prompt and a model on the interpreter goes through one module (`corvid-vm/src/interp/prompt/model_json.rs`): the rendered text, the recorded arguments, values shown to a judge or an adjudicator, a continuation's partial output, and a prompt function's arguments and results. Reply decoding uses the public decoder with unknown fields ignored, as `T.from_json` does. `schema_for` now describes the public form, built from shape functions in `corvid_public_json::schema` that OpenAPI calls too (its output is byte for byte what it was). Two prompt rules sit on top of the mapping: a `Grounded<T>` argument shows its `T`, and a `Partial<T>` reply keeps its per-field `complete`/`streaming` envelope with public values inside. Native code needed no change: it accepts scalars and records of scalars, where the two forms agree, and its schema comes from the same `schema_for`. Tests: `corvid-vm/src/tests/prompt_json.rs` (arguments, schema, replies, an old tagged reply refused with the field named) and the schema unit tests.

   *B1a, done 2026-11-26:* a native `String` prompt argument renders as its JSON form, quoted and escaped, at run time (`corvid_string_json_quote`) and when the arguments are constants. A parity test compares both tiers' recorded prompt text, with quotes, a backslash, a newline, a tab and a non-ASCII letter.

   The original plan for B1 follows. A prompt's arguments are encoded with `corvid_json_encode_typed`, and its reply decoded with `corvid_json_decode_typed`. The rendered prompt is then byte for byte the interpreter's, which `value_to_json` writes. This replaces `emit_stringify_arg` and the per-record decoders in `struct_decode.rs`, so the second copy of the mapping goes. Text-held types at a prompt boundary then need nothing of their own. Tests:
   - a parity test of the recorded rendered prompt for each type;
   - a reply that is not the type's text, which fails the same way on both tiers.
2. **B2. User tools through the runtime call.** This is the existing roadmap item, "move a tool of the program's own onto the runtime call". The tools library registers its implementations with the runtime, and native code calls them through `corvid_runtime_call`. Then the arguments are encoded with the public mapping, a `Secret` is redacted and revealed, the result is decoded and checked, and the trace comes from the one path the interpreter uses. It settles text scalars and secrets at user-tool boundaries together. The direct-call wrappers and their separate trace emission (`corvid_trace_tool_call`) go.
   *B2 as decided before building (2026-11-26).* Found while starting it: the two tiers send a tool different JSON. The interpreter encodes a tool's arguments with its internal form (`value_to_json`: an `Option` is `{"tag": "some", ...}`, a sum type has no `type`) and decodes the result with `json_to_value`, while native runtime calls (connectors, the runtime's own tools) already send the public form. Moving native user tools onto the runtime call alone would make the tiers disagree for any tool taking an `Option` or a variant. So B2 is two slices:

   - **B2a. The interpreter's tool calls exchange the public form.** Each argument is encoded by its parameter's declared type with the public mapping, the form a frontend, `to_json` and (since B1) a model see. A value whose type has no public form (`DbHandle`, `JsonBuilder`) crosses as the runtime's handle, which only the runtime's own tools accept; that is decided by the declared type, never by an encoding failure. The result is decoded with the public decoder by the declared return type, ignoring undeclared keys as `T.from_json` does, so a tool that answers with the wrong shape is refused with the field path instead of building a wrong value. The same arguments are what the trace records, what an approver is shown, and what Python and host tools receive. Visible effects: a Python or host tool receives `None` as `null` and a variant as `{"tag": "variant", "type": ..., "variant": ..., "fields": [...]}`; a recording whose tool arguments hold an `Option` or a variant stops replaying with a divergence naming the tool.
     *B2a, done 2026-11-26.* `interp/tool_json.rs` holds the two rules; the tool path, revealed secret arguments, the step debugger's view and its override all use them. `Grounded<T>` given where a plain type is declared crosses as its `T`, as for a prompt. The interpreter's internal encoder lost its revealed-secret form, now the public mapping's (`SecretArgument::Revealed`). Tests: `corvid-vm/src/tests/tool_json.rs`.
   - **B2b. Native user tools through the runtime call.** At startup a native binary registers each linked `#[tool]`'s JSON entry point with the runtime's own tool registry, so `Runtime::call_tool` reaches it. Native code then calls a program's tool through `corvid_runtime_call`, the path connectors and the runtime's tools take: public-form arguments, `Secret` redaction and reveal, the result decoded by the embedded plan, and one trace written by the runtime. The direct `__corvid_tool_<name>` calls, the per-return-type replay branches and the separate tool trace encoding (`emit_json_stringify_arg`) go.

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
