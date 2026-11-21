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
