# First run without an API key (B21, launch track step 1)

**Status:** proposal, 2026-11-28. It adds a project setting and CLI flags, no shared grammar, so it is built after lineage L3 without waiting for frontend review.
**Owner:** backend and AI (Claude Code, Windows session).
**Roadmap:** [launch track](../backend-ai-roadmap.md#launch-track-decided-by-the-owner-2026-11-28) step 1; B49 (evaluations in the build) builds on it.

## The problem

A developer who runs `corvid new` today gets a project with no AI step. Adding one means choosing a provider, making an account, exporting an API key and paying for calls before the first run works. Most people who try a new language give it minutes. Every one of those steps loses some of them, and the AI step is the reason they came.

Corvid already records every model call in a run's trace, and replay serves a whole run back. That is not enough for a first run: a replay only answers the exact run it recorded, so the first edit a developer makes falls off it.

## The design

```text
$ corvid new notes
$ cd notes
$ corvid run
Summary: Three tasks are due this week; the report is the most urgent.
  (answered from answers/summarize.jsonl, a recorded response; `corvid run --live` asks a model)
```

`corvid new` scaffolds a project whose `main` calls a typed prompt, with its answers already recorded:

```corvid
type Summary:
    text: String
    urgent: Bool

prompt summarize(notes: String) -> Summary:
    "Summarize these notes in one sentence and say whether anything is urgent: {notes}"

agent main() -> String:
    summary = summarize("Report due Friday. Call the bank. Buy milk.")
    return "Summary: " + summary.text
```

- **Recorded answers.** `answers/<prompt>.jsonl` holds, for each prompt, the typed reply to each input it has been asked. An answer is keyed by the prompt's name and the text the prompt rendered for the model, so any program that asks the same question gets the same answer, however the code around it changes. That key is why it survives edits where a replay does not.
- **Where answers come from is a written choice.** A model call costs money and sends data to a provider, so the project says which source it uses, in `corvid.toml`:

  ```toml
  [models]
  answers = "recorded"   # or "live"
  ```

  `corvid new` writes `recorded`. `corvid run --live` overrides the setting for one run.

  **Decided when building (2026-11-28):** leaving the setting out keeps today's behavior, where the model the developer configured (an exported provider key, or a local model) answers. Requiring the setting would break every existing program that calls a model, and those programs already made the consequential choice explicitly by exporting a provider's key. Recorded answers are the opt-in; a project chooses them in source, and nothing changes for one that does not.
- **A missing answer is a typed refusal, never a guess.** When the program asks a prompt something with no recorded answer, the call fails with an error naming the prompt and the input, and showing how to record it: `corvid run --live --record`. The checker's types still hold, because a recorded answer is decoded through the prompt's declared return type exactly as a live one is. An answer that no longer fits the type is refused the same way.
- **Recording.** `corvid run --live --record` asks a live model and writes each answer to the prompt's file. Answers are plain JSON lines, so they can be read, edited by hand and reviewed in a diff. **Decided when building:** the answers are taken from the run's own trace, which already records what each prompt sent and what came back, so recording is a projection of the trace, not a second recorder. The trace redacts secrets, so a prompt that sent one is recorded with `<redacted>` in place of it. It will not match the next run and asks to be recorded again, so a secret never reaches an answers file.
- **One line to go live.** Changing `answers = "recorded"` to `"live"` and setting a provider's key is the whole switch. `corvid run` then prints which provider and model answered, so the change is visible.

## Three tiers

- **Backend:** answers are served through the runtime's model adapter, so effects, budgets, approvals and the trace behave as for a live call. A recorded answer records `source: recorded` in the trace, so a timeline never presents a recorded answer as a model's.
- **Frontend:** a served app on recorded answers works offline, so `corvid serve` of a starter shows its AI feature in the browser with no key. Nothing new on the wire.
- **AI:** the same recorded answers are what B49 evaluations run against, so "it works on my machine with no key" and "CI checks it with no key" are one mechanism.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Ship a whole-run replay trace in the template | Falls off on the first edit, since replay answers only the run it recorded. |
| A fake model that returns placeholder values of the right type | A first run that prints `"lorem ipsum"` shows nothing of what Corvid is for. Recorded answers are real model output. |
| Bundle a small local model | Large to download, slow on many laptops, and bundling a model is the owner's decision. `corvid model pull` (below) makes it opt-in. |
| Require `answers =` in every project | Breaks every program that calls a model today, which chose a provider explicitly by exporting its key. Recorded answers are the opt-in. |
| Key answers by the call site | Breaks when code moves. The prompt and its rendered text are what the answer depends on. |

## Slices

- [ ] F1. The answers store and its model adapter: `answers/<prompt>.jsonl`, keyed by prompt name and rendered text, decoded through the prompt's return type, with a typed refusal for a missing or ill-fitting answer and `source: recorded` in the trace.
- [ ] F2. `[models] answers = "recorded" | "live"` in `corvid.toml` (an unknown value refused, naming both), and `corvid run --live` and `--record`, recording from the run's trace.
- [ ] F3. `corvid new` scaffolds the AI step with recorded answers, so `corvid new notes && cd notes && corvid run` prints a real answer on a clean machine with no account. The `--web` starter does the same through `corvid serve`.
- [ ] F4. `corvid model pull`: an optional local model through Ollama. Which model is offered is the owner's decision, asked before this slice.

## Non-scope

- Choosing among several recorded answers for one input (sampling). One input has one answer.
- Recording tool calls or connector responses: those have their own replay and mocks.
