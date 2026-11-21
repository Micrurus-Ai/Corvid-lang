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

Use `ui.query_form(path, controls)` to submit named fields to an application route with GET, and `ui.link(label, path)` to navigate there. Supply a trusted application path for each.

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

## Serve a page and an API

In a project created by `corvid new notes`, use this as `src/main.cor`.

```corvid-fragment
import "./std/ui" as ui

server notes:
    route GET "/" -> html:
        return ui.page("Notes", [ui.text("Ready & listening")])

    route GET "/api/status" -> json String:
        return "ready"
```

From the project directory, run `corvid serve src/main.cor --listen 127.0.0.1:8080`. Open `http://127.0.0.1:8080/` to see the page. Request `/api/status` to read the JSON string `"ready"`. An `html` handler returns a `String` and the server sends it as HTML. Use view helpers to escape text inserted into the page.

## Package an interactive page

Run `corvid new counter`. In the new project, put the server in `src/main.cor`.

```corvid-fragment
import "./std/ui" as ui

server counter:
    route GET "/" -> html:
        count = ui.int_state("Count", 0, [ui.action("Add one", "increment")])
        echo = ui.text_state("Message", "", "echo")
        return ui.interactive_page("Counter", [count, echo])

    route GET "/api/status" -> json String:
        return "ready"
```

Define the browser handlers in `src/events.cor`.

```corvid-fragment
agent increment(current: Int) -> Int:
    return current + 1

agent echo(message: String) -> String:
    return message
```

Run `corvid build src/main.cor --target web`. Extract `target/web.tar` into an empty directory. From that directory, run `corvid serve src/main.cor --listen 127.0.0.1:8080` and open `http://127.0.0.1:8080/`. Click **Add one** or focus it and press Enter. Type into **Message** to see the text handler's result. Both handlers run in the browser. The server handles `/api/status`.

| Function | Behavior |
| --- | --- |
| `action(label: String, handler: String) -> View` | Creates a button bound by name to an `(Int) -> Int` handler in `src/events.cor`. |
| `int_state(label: String, initial: Int, controls: List<View>) -> View` | Groups controls around one integer. Each action receives its current value and returns the next one. |
| `text_state(label: String, initial: String, handler: String) -> View` | Calls a `(String) -> String` handler on initial load and on each input event. |
| `interactive_page(title: String, content: List<View>) -> String` | Renders the page and loads its compiled browser handlers. |

The archive includes application sources and compiled browser assets. Static applications can omit `src/events.cor`. A build error preserves the previous successful archive.

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
