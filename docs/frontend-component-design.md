# Writing a component in Corvid

**Design draft for developer review.** These examples use proposed syntax. For runnable Corvid today, use the [browser views guide](book/19-browser-views.md). Both candidates below use the same state, events and execution rules. The choice under review is how to express the view.

## A cart that updates when an item is removed

The cart owns its lines. Each line has a unique product ID, a quantity and a price in integer cents. The total is computed from the current lines, so the author only writes one update. Assume the imported ordinary function `total_cents` sums quantity times unit price and `money` formats cents for display.

**Candidate A · Indented elements**

```text
browser component Cart(initial: List<CartLine>):
    state lines = initial

    fn remove(id: String):
        lines = lines.filter(line => line.id != id)

    view:
        heading("Shopping cart")
        for line in lines key line.id:
            row:
                text(line.name)
                button("Remove " + line.name, on_click = () => remove(line.id))
        text(money(total_cents(lines)))
```

**Candidate B · Functions returning views**

Keep the same component declaration, state and handler. Replace the view block with an ordinary return expression.

```text
    return stack([
        heading("Shopping cart"),
        keyed(lines, line => line.id, line => row([
            text(line.name),
            button("Remove " + line.name, on_click = () => remove(line.id))
        ])),
        text(money(total_cents(lines)))
    ])
```

## A search that keeps the latest answer

The field emits a string. `catalog.search` is an imported typed server endpoint taking a string and returning a list of product names. Calling `request` crosses the network. `latest` owns one pending request per mounted component instance. A new call cancels the old one, rejects stale completions and publishes the latest loading, success or error state. Leaving the page disposes the request.

**Candidate A · Indented elements**

```text
browser component ProductSearch():
    state results = latest<List<String>>()

    fn search(term: String):
        results.start(request catalog.search(term))

    view:
        text_field("Find products", on_input = search)
        status(results)
        for name in results.value_or([]):
            text(name)
```

**Candidate B · Functions returning views**

```text
    return stack([
        text_field("Find products", on_input = search),
        status(results),
        stack(results.value_or([]).map(name => text(name)))
    ])
```

In both candidates, an idle search renders an empty list. Starting a request clears the prior value. The proposed `status` view announces loading or an error, and renders no message on success. A canceled request cannot publish an error into its successor.

## Rules to check in the prototype

- A state initializer runs once per mounted instance. Updating state refreshes dependent output and preserves keyed child identity. Duplicate keys produce a development diagnostic.
- A click callback in the cart takes no arguments. The text field supplies a string to its input callback. Report incompatible handlers at the binding. A browser component may call a typed server endpoint via a request. Importing server credentials into a browser component must fail the build.
- Render dynamic text as text. Use native button and input semantics. Keep the existing browser guide's server-rendered notes form working under the same CLI.

## How to choose the simpler design

Give both versions to developers in alternating order. Ask them to predict the total after removing a line, identify the network call, and explain what appears if the first search finishes last. Then ask them to add a quantity button, handle an empty search and locate a deliberately mistyped event handler using its diagnostic. Record wrong predictions, edits that require documentation and time to the first working change. Use the results to revise the syntax before adopting it. Benchmark the browser targets separately.

## Reading-list prototype

The two [indented](../examples/component_trial/indented.view) and [function](../examples/component_trial/functions.view) forms declare the same labeled integer and bind the same [page-count handler](../examples/component_trial/events.cor). The indented form makes the child a block. The expression form passes children in an ordered list, as existing Corvid view functions do. The trial accepts one control and an integer addition. It lowers each view to ordinary Corvid before the normal web build. Both targets use the existing DOM binding code.

| Action | Owner, input and result | Invocation and lifetime |
| --- | --- | --- |
| Read one page | A mounted browser control owns the count. Its callback accepts the current `Int` and returns the next `Int`. | A native button invokes the handler locally. Its enclosing control supplies the value and displays the result. Navigation discards the instance. |
| Save to shelf | The server owns the shelf. The form submits a `Reading` containing shelf and title to `POST /save`. The handler returns a saved record or a failure. | Submitting waits for the HTTP response. The route validates input and renders success or failure. A successful write persists in SQLite. |

For the proposed general model, component parameters have ordinary Corvid types. A container accepts ordered view children. Callbacks declare the event payload and result at the binding. A keyed child belongs to its parent instance and retains local state until its key leaves that parent. Unmounting removes listeners and cancels component-owned requests. Keep server calls explicit in source and give the view pending, success and failure branches. Compute-heavy browser functions can use a typed export from a Wasm module via its generated JavaScript loader. The view remains responsible for DOM updates.

Both prototype actions are UI-only. Proposed assistant exposure requires an explicit author declaration naming the action, its public input and result types, and the context fields an assistant may read. Changing the button label should preserve that identity. A server-owned action validates the public input and caller's current authority on every invocation. The reading-list trial measures a local control and exercises its adjacent ordinary form. Assistant invocation and retry policy have separate implementation milestones.

### What the comparison established

On Windows with the Corvid 0.0.1 debug CLI (4e3ba2b0), all four packages passed `check`, the reading-list application test, web build, extraction, migration, five HTTP saves and rejection of a malformed shelf. Changing the callback to `(String) -> String` produced the required and actual types at indented.view line 3 and functions.view line 2. Both use the same pre-build checker. A Node check invoked the packaged handlers at zero, one and the maximum signed 64-bit integer. Both targets agreed, including wraparound.

| Target | HTML response bytes | Additional handler bytes | Combined body bytes |
| --- | ---: | ---: | ---: |
| JavaScript | 6,379 | 0 | 6,379 |
| WebAssembly | 2,561 | 6,602 loader + 508 module | 9,671 |

After five valid saves and one rejected malformed save, each package served the fifth title and both untouched shelves from a replacement server using the same database.

These are loopback HTTP body sizes for the initial shelf 2 edit screen. The JavaScript prototype embeds a base64 module in the HTML. The Wasm loader and module are separate assets. Both grammars produced the same sizes. A repeat visit with cached external assets can have different costs.

Use `python scripts/verify_component_trial.py target/debug/corvid.exe --packages-only` for the package and HTTP checks. With Python Playwright and a launchable Chromium browser, omit `--packages-only` and pass e.g. `--channel msedge` for five fresh contexts per combination, alternating run order. The harness records navigation-to-enabled time, first synchronous click-to-output time and a batch of 1,000 updates. It also checks keyboard activation and reload. The initial background launch failed at Windows IPC. A direct run with Edge 158.0.4370.33 passed keyboard activation and reload in 20 fresh contexts.

| Grammar and target | Navigation to enabled (ms), five runs | Median (ms) | Median first click (ms) | Median 1,000 clicks (ms) | Browser body bytes |
| --- | --- | ---: | ---: | ---: | ---: |
| indented-js | 54.5, 24.6, 29.2, 32.1, 28.6 | 29.2 | 0.7 | 7.6 | 6,413 |
| indented-wasm | 87.1, 64.2, 88.8, 99.6, 94.2 | 88.8 | 1.1 | 10.5 | 9,737 |
| functions-js | 29.9, 31.0, 43.3, 25.7, 29.2 | 29.9 | 0.7 | 7.4 | 6,413 |
| functions-wasm | 86.3, 94.5, 75.5, 88.0, 93.4 | 88.0 | 1.1 | 12.5 | 9,737 |

Click times cover synchronous handler execution and the output update, excluding paint. Browser body bytes sum same-origin navigation and resource entries on the edit page after the five saves. The direct run logged 7,142 Wasm asset bytes, compared with 7,110 in the package recheck above.

The function form demonstrated ordinary ordered-child composition. The selected authoring direction uses indented components, named state and inline click handlers. See the [tested prototype and learning guide](https://github.com/Micrurus-Ai/Corvid-lang/blob/1022d50b/docs/book/19-browser-views.md#a-component-with-named-state-prototype). Both grammars gave the same binding diagnostic at the action. JavaScript is the provisional default for this small control because it transferred fewer bytes and had lower median startup and click times in both comparisons. Before adopting the grammar, apply the reader protocol above. For this exercise, show both fixtures and the save route, record each reader's predictions about two clicks, reload and save before running anything, then time a change to the increment and ask the reader to repair the mismatched callback. Record the source line they use and any help they need. No independent predictions or edit times have been collected.
