# Migrating from Python

## Who this is for

You have a Python codebase that uses LangChain, DSPy, LlamaIndex,
Pydantic-AI, or your own home-grown LLM glue. You want to know what
Corvid changes — concretely — for that team.

## The five concrete changes

### 1. Effect rows replace ad-hoc tagging

In Python you might tag tools with decorators or runtime config:

```python
@dangerous_tool
def refund(amount, customer_id):
    ...
```

In Corvid the tag is a typed effect row that the compiler reads:

```corvid
tool refund(amount: Float, customer_id: String) -> String uses refund_effect:
    @host.payment.refund(customer_id, amount)
```

The decorator pattern works at runtime, after the call. The effect
row works at compile time, before the binary exists.

### 2. `approve` replaces approval middleware

Python pattern:

```python
def main():
    if not approval_service.has_pending(user_id, "Refund"):
        raise PermissionError()
    refund(50.0, customer_id)
```

Corvid pattern:

```corvid
agent main():
    approve Refund(50.0, customer_id)
    return refund(50.0, customer_id)
```

If you remove the `approve` line in Python, the program runs and
crashes at runtime (if you remembered to write the check). If you
remove the `approve` line in Corvid, the program does not compile.

### 3. `Grounded<T>` replaces "be careful with strings"

Python pattern:

```python
policy_text = retrieve("policy.txt")    # str
model_summary = summarize(article)       # str
# nothing in the type system distinguishes these
decide(policy_text, model_summary)
```

Corvid pattern:

```corvid
policy = fetch_policy()                     # Grounded<String>
summary = summarize(article)                # String
decide(policy, summary)                     # decide takes (Grounded<String>, String)
```

A function that should take a grounded value cannot be called with a
non-grounded one. The compiler refuses.

### 4. Replay replaces "reproduce the bug locally"

Python pattern: log everything, hope the logs are enough, reconstruct
the failing call by hand, hope the model gives the same answer.

Corvid pattern:

```sh
corvid trace list
corvid replay <id>
```

Byte-identical reproduction. Cached model responses. Same tool calls.

### 5. `corvid eval --swap-model` replaces "manually rerun the eval suite"

Python pattern: run an eval harness against a model, manually compare
to the baseline, hope you remembered to lock the seed.

Corvid pattern:

```sh
corvid eval --swap-model gpt-5 --source app.cor target/trace
```

Diffs the new model's behavior against the recorded baseline trace.
Tells you exactly which prompts changed answer, by how much, and at
what cost.

## What you give up

- Python's vast ecosystem. Corvid's stdlib + connectors cover the AI
  surface; for everything else (numpy, scikit, custom ML infra), you
  call out to Python through the FFI (Phase 30).
- The flexibility of monkey-patching. Effect rows are immutable types,
  not runtime-mutable dicts.
- The freedom to ship code with no type system. You write effect rows.

## What you get

- The bugs LangChain users hit in production (silent approval bypass,
  ungrounded fact claims, runaway loops, model-upgrade regressions)
  caught at compile time.
- A test suite that exercises real safety properties.
- An audit log a security team can read.
- A migration path for model upgrades that is a diff, not an outage.

## Practical migration pattern

1. Pick one agent in your Python code that does something dangerous
   (calls a payment API, sends an email, writes to a database).
2. Rewrite it in Corvid with explicit effect rows, `approve`, and
   `Grounded<T>`.
3. Wire it into your existing service via the FFI or as a separate
   binary spoken to over HTTP.
4. Run `corvid audit` and verify the static report matches your
   expectation.
5. Move the next agent.

You do not have to rewrite your whole codebase. Move the AI-critical
slices.
