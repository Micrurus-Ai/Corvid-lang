# Debugging

## When a Corvid program won't compile

The diagnostic carries a stable error code and a fix hint:

```text
[E0101] error: dangerous tool `refund` called without a prior `approve`
    │ Help: add `approve Refund(arg1, arg2)` on the line before this call
```

The guarantee behind an approval-rule diagnostic is
`approval.dangerous_call_requires_token`. Look it up:

```sh
corvid claim --explain approval.dangerous_call_requires_token
```

The output explains the rule, the registry row, the positive +
adversarial test references, and the canonical fix shape.

## When `corvid audit` reports something

`corvid audit` emits a structured report. Each finding includes:
- The agent or tool involved.
- The guarantee class (Static / RuntimeChecked / OutOfScope).
- The recommended fix.
- The CLI command to re-check after the fix.

## When an agent fails at runtime

```sh
corvid trace list
corvid replay <trace-id>
```

The replay reproduces the failure deterministically. Inspect the
trace DAG to see which step diverged.

## When a model upgrade broke something

```sh
corvid eval --swap-model gpt-5 --source src/main.cor target/traces
```

The diff shows: prompts that changed answer, budgets that exceeded,
tool sequences that differed. Each diff entry links to the original
trace's id.

## When a connector test fails

```sh
corvid connectors test <name> --verbose
```

Shows mock vs. replay vs. real-mode results side by side. A drift
between modes is a connector contract bug.

## When `corvid doctor` is red

Each check has a one-line "how to fix" hint. Common failures:

- C toolchain missing → install `cc` (`xcode-select --install` /
  `apt install build-essential` / Visual Studio Build Tools).
- Provider key missing → `export OPENAI_API_KEY=...` (or whichever
  provider you configured).
- Replay storage permissions → `chmod -R u+w ~/.corvid/replay`.

## Common pitfalls

| Symptom | Likely cause |
|---|---|
| "expected `String`, got `Grounded<String>`" | Forgot to `unwrap_with_citation()` on a retrieval result. |
| "dangerous tool called without `approve`" | Approve is in the wrong scope (lexical) or missing. |
| "budget exceeded" at compile time | Composed cost across the call graph exceeds `@budget`. Either raise the budget or refactor. |
| "replay quarantine fired" | Replay tried to escape into real-mode through a connector. |
| Agent loops forever in tests | Missing `@max_steps`. |
| Migration drift | Edited a previously-applied SQL file. Roll the change forward in a new migration. |
