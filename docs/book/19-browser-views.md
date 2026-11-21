# Browser views

A view can be an ordinary Corvid function. Import `std/ui` in a scaffolded project and return a `View` to reuse a control. Pass a list of views to a container to set their order.

```corvid-fragment
# src/main.cor
import "./std/ui" as ui
import "./std/ui" use View
import "./std/io" use io_write_text

fn search_box(initial: String) -> View:
    query = ui.text_field("Search notes", "query", initial)
    return ui.search_form([query, ui.submit("Search")])

agent main() -> Result<String, String>:
    tips = ui.disclosure("Search tips", [ui.text("Use a word from the note.")])
    html = ui.page("Notes", [search_box(""), tips])
    io_write_text("index.html", html)?
    return Ok("index.html")
```

From the project root, set `CORVID_IO_ROOT` to the directory where you want the page written, then run `corvid run --target interp src/main.cor`. Open the resulting `index.html` in a browser.

The field keeps its current text as you type. Press Enter in the field or click Search to submit a GET request to the current page with a `query` parameter. A static page reloads its initial values. The disclosure keeps its own open or closed state until navigation. Click its summary or focus it and press Space to toggle it.

`ui.text`, `ui.text_field`, `ui.submit`, `ui.disclosure`, and `ui.page` escape their text arguments for HTML. Pass user input through those functions. Constructing a `View` directly treats its `html` field as trusted markup.

## Static view functions

Import `"./std/ui" as ui` in a scaffolded project. A `View` contains an HTML fragment in its `html: String` field.

| Function | Result |
| --- | --- |
| `text(value: String) -> View` | A paragraph containing the escaped value. |
| `text_field(label: String, name: String, initial: String) -> View` | A labeled text input with a form name and initial value. |
| `submit(label: String) -> View` | A submit button for its enclosing form. |
| `search_form(controls: List<View>) -> View` | A GET form targeting the current page. |
| `query_form(path: String, controls: List<View>) -> View` | A GET form targeting the supplied application path. |
| `disclosure(label: String, content: List<View>) -> View` | Expandable content with a summary. |
| `link(label: String, path: String) -> View` | A link to the supplied application path. |
| `page(title: String, content: List<View>) -> String` | An HTML document that renders the views in order. |

## Updating state with a Corvid handler

Define browser event handlers in `src/events.cor`. Each button passes its component's current integer to the handler and displays the integer it returns.

```corvid
agent add_pair(current: Int) -> Int:
    return current + 2

agent preview(draft: String) -> String:
    if draft == "":
        return "Write a note"
    if draft == "done":
        return "Ready to save"
    return draft
```

Build the handlers with `corvid build src/events.cor --target wasm`. In the same project, render an interactive page from `src/main.cor`.

```corvid-fragment
import "./std/ui" as ui
import "./std/ui" use View
import "./std/io" use io_write_text

fn tally(label: String, initial: Int) -> View:
    add = ui.action("Add two", "add_pair")
    return ui.int_state(label, initial, [add])

agent main() -> Result<String, String>:
    draft = ui.text_state("Note", "", "preview")
    html = ui.interactive_page("Tallies", [tally("Apples", 0), tally("Pears", 10), draft])
    io_write_text("index.html", html)?
    return Ok("index.html")
```

Set `CORVID_IO_ROOT` to the project root and run `corvid run --target interp src/main.cor`. Serve the project root over local HTTP and open `index.html`. The page loads `target/wasm/events.js` and its adjacent WebAssembly module. Each tally keeps its own value until the page reloads. Activating a button updates its tally in place and preserves keyboard focus.

The second argument of `ui.action` names an exported agent. The generated loader checks for a single `Int` parameter and an `Int` return type before enabling the button. A missing or incompatible handler leaves its button disabled and displays an error in the component.

`ui.text_state(label, initial, handler)` calls a `(String) -> String` agent on the initial value and each input event. The browser displays its return value as text and keeps the editable draft and caret in the input.

## Serving a form and its result

A route declared with `-> html` returns a Corvid `String`. The server sends it as an HTML document. Use a typed query record to validate submitted values before the result page renders.

```corvid-fragment
# src/search.cor
public type Search:
    term: String
    limit: Int where between(1, 50)
```

```corvid-fragment
# src/main.cor
import "./std/ui" as ui
import "./search" use Search
import "./std/ui" use View
import "./std/io" use io_write_text

fn search_page(request: Search) -> String:
    search = View(form_for("GET /search", request, "Search"))
    save = View(form_for("POST /save", request, "Save search"))
    return ui.page("Notes", [search, save, ui.text(request.term)])

server notes:
    route GET "/" -> html:
        return search_page(Search("", 5))

    route GET "/search" query Search -> html:
        return search_page(query)

    route POST "/save" form Search -> html:
        return match io_write_text("saved-search.txt", body.term):
            Ok(_) -> search_page(body)
            Err(message) -> ui.page("Save failed", [ui.text(message)])

    route POST "/preview" body Search -> html:
        return search_page(body)
```

Set `CORVID_IO_ROOT` to the project directory for the saved search. Run `corvid serve src/main.cor --listen 127.0.0.1:8000` and open the root URL. Pressing Search navigates to the result page. The browser URL carries the submitted values, so reloading preserves them. An invalid integer returns HTTP 400. Save search submits the fields in the POST body and writes the term to `saved-search.txt`.

`form_for(target, initial, submit_label)` returns HTML for a fixed-path route declared in the same module. Use a literal such as `"GET /search"` or `"POST /save"`. Corvid checks the route and the initial record at compile time. It derives field names, labels and controls from the record in declaration order. Supported field types are String, Int, Float and Bool. GET uses a `query` record. POST uses a `form` record and the browser's standard URL-encoded request body. The route reads the submitted record as `body`.

The save example uses a public route. Cookie-authenticated mutations also require the session CSRF token in the `x-csrf-token` header. A plain HTML form cannot set that header.

The separate preview route accepts the same record as JSON. Posting `{"term":"meeting","limit":2}` to `/preview` returns its rendered page.

Open `/openapi.json` on the running service to inspect the form's contract. In this example, the query and POST body both reference the `Search` schema, whose properties are `term` (string) and `limit` (integer). Use `corvid contract openapi src/main.cor --out -` to inspect the same schema before starting the server. Imported records keep their nested field definitions and field constraints. Follow the emitted schema reference when different modules declare the same type name.

## Displaying a streamed answer

```corvid-fragment
# src/main.cor
import "./std/ui" as ui

prompt answer(topic: String) -> Stream<String>:
    "{topic}"

server help:
    route GET "/" -> html:
        return ui.page("Help", [ui.stream_text("Answer", "/answer")])

    route GET "/answer" -> json Stream<String>:
        return answer("Explain the search feature")
```

Set `CORVID_MODEL` to your streaming provider and run `corvid serve src/main.cor`. Open the root page and press Start. The answer appears as chunks arrive. Cancel closes the connection and keeps the text already received. If the connection fails, Start again makes a fresh request and clears the old answer. Leaving the page closes an active stream. The path argument is an application-relative GET URL on the same origin. Each message must carry a JSON string. The Corvid server sends the completion and error events and serves the browser module.

## Packaging an interactive application

Place the server in `src/main.cor`. Use `ui.link` with a trusted application path to navigate between pages.

```corvid-fragment
import "./std/ui" as ui

prompt answer(topic: String) -> Stream<String>:
    "{topic}"

server notes:
    route GET "/" -> html:
        return ui.page("Notes", [ui.link("Open workspace", "/notes/workspace")])

    route GET "/notes/workspace" -> html:
        count = ui.int_state("Notes reviewed", 0, [ui.action("Review", "increment")])
        answer_view = ui.stream_text("Answer", "/answer")
        return ui.interactive_page("Workspace", [count, answer_view, ui.link("Home", "/")])

    route GET "/answer" -> json Stream<String>:
        return answer("Explain the search feature")
```

Define the button handler in `src/events.cor`.

```corvid
agent increment(current: Int) -> Int:
    return current + 1
```

Run `corvid build src/main.cor --target web` from the project root. The build checks the staged application, compiles the browser handler, and writes `target/web.tar`. Extract the archive into an empty directory. From that directory, set the model configuration and run `corvid serve src/main.cor` with the matching Corvid CLI. Open the root page and follow **Open workspace** to use the counter and the streamed answer.

The archive includes Corvid and Python source files under `src/`, the project manifest and lockfile when present, root `tools.py` when present, SQL files under `migrations/`, and the compiled JavaScript and WebAssembly pair when `src/events.cor` exists. The CLI supplies the streaming module. Supply external providers, runtime credentials, host dependencies and application data in the destination environment.
