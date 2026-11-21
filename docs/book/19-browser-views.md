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

For an opt-in, read-only search that a compatible browser agent can discover, use `ui.agent_query_form(name, description, path, controls)`. Supply a stable unique tool name, an author-written description, and a trusted read-only GET path. The visible form and the agent use the same named fields. The user submits the form. On browsers without declarative WebMCP, the form works as a normal GET form. The server remains responsible for authorizing the request.

```corvid-fragment
fn calendar_search() -> View:
    day = ui.text_field("Day", "day", "")
    return ui.agent_query_form("find_slots", "Find available calendar slots for a day", "/slots", [day, ui.submit("Find slots")])
```

## Static view functions

Use layout functions to arrange any list of views. `ui.stack(content)` places children vertically. `ui.row(content)` places children horizontally and wraps to another line when needed. `ui.grid(content)` fits as many equal-width columns as the container allows, aiming for a minimum of 16rem per column. A narrower container gets a single column. All three preserve the order of their children and use a 1rem gap.

For example, a project board can use the same column function for each status.

```corvid-fragment
fn column(name: String, tasks: List<View>) -> View:
    return ui.stack([ui.text(name)] + tasks)

fn board() -> View:
    return ui.grid([column("Planned", [ui.text("Design navigation")]), column("Done", [ui.text("Create project")])])
```

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

### Display state in more than one view

`ui.shared_int_state(name, label, initial, controls)` creates an integer control with a page-wide name. Give each owner a unique name. `ui.state_output(name, label)` adds a labeled, read-only display anywhere on that page. It picks up the initial value when the browser handlers load and updates after each successful action. Reloading the page restores the initial value.

For example, place the quantity control in a product card and the item count in a header. Use the `increment` browser handler shown above.

```corvid-fragment
server shop:
    route GET "/" -> html:
        header = ui.state_output("items", "Cart items")
        quantity = ui.shared_int_state("items", "Quantity", 0, [ui.action("Add one", "increment")])
        return ui.interactive_page("Shop", [header, quantity])
```

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

## Package and restart a reading list

The [reading list](../../examples/reading_list/src/main.cor) renders three shelves from SQLite. Select a shelf to open the typed `Reading` form. The submitted shelf number chooses the row to update. The POST handler binds the title and shelf as SQL parameters, then lists the stored rows again.

From the repository root, run these commands in PowerShell with the Corvid CLI on your path. Use new directories for the project and extracted package.

```powershell
corvid new reading-list
Copy-Item examples/reading_list/src/main.cor reading-list/src/main.cor
Copy-Item examples/reading_list/migrations reading-list -Recurse
cd reading-list
corvid check
corvid build src/main.cor --target web
mkdir installed
tar -xf target/web.tar -C installed
cd installed
corvid migrate up --dir migrations --database reading.sqlite
corvid serve src/main.cor --listen 127.0.0.1:8080
```

Open `http://127.0.0.1:8080/`. Tab to a shelf link and press Enter. Tab to the title, replace it, and activate **Save to shelf**. Stop the server with Ctrl+C and repeat the serve command from the same directory. Open the list again to read the saved title.

The scaffold supplies `src/std` and a project-relative I/O root. The archive includes the SQL migration. Running it in the extracted directory creates and seeds the database there. Reuse that directory across restarts. No model setup is required.

The [verification script](../../scripts/verify_reading_list.py) repeats the scaffold, package, migration and restart in fresh directories. With Python 3.12+, Python Playwright and its Chromium browser installed, run `python scripts/verify_reading_list.py path/to/corvid` from the repository root. It reports list and save navigation durations from the browser and separate HTTP save round trips. Use `--channel msedge` for installed Edge or `--http-only` for the server checks alone.

### Try a typed suggestion

Run the verification script above for a credential-free exercise. Its loopback Chat Completions fixture can return a typed action, produce malformed output, or hold a request open until the client cancels. To choose your own titles interactively, start a local OpenAI-compatible model that supports structured output. Set `CORVID_MODEL` to `openai-compat:http://127.0.0.1:8080/v1:local-model` before starting the packaged server, substituting your local port and model name.

Open a shelf and choose **Suggest a title**. The original edit form stays available while the request runs. Cancel a pending request with **Cancel suggestion**. After a malformed response, choose **Suggest a title** again. On success, review the second form, edit the proposed title if desired, and choose **Apply suggestion**. Returning to the list without submitting discards the preview. Follow the decision from the typed prompt to the save handler in the [first-agent exercise](10-agents.md#your-first-agent-in-the-reading-list).

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
