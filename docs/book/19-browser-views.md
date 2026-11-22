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

For a browser-local reading counter, place this control inside `ui.int_state` on an interactive page:

```corvid
record = ui.assistant_action("Read one page", "read_page", "record_page_read", "Add one page to this session's reading count.")
```

The visible label can change independently of the tool name. The binding takes the current count from its owning control and invokes the compiled `read_page(Int) -> Int` handler. An assistant supplies an empty object. The returned decimal string preserves the full Corvid integer. A second button created with `ui.action` stays available to keyboard and pointer users. Tool registration uses the browser's optional WebMCP interface. [Reading counter](../../examples/reading_list/src/progress.cor) · [WebMCP imperative API](https://developer.chrome.com/docs/ai/webmcp/imperative-api)

To let an assistant edit a single-line reading note, put this handler in `src/events.cor`:

```corvid
public agent edit_note(note: String) -> String:
    return note
```

Place `ui.assistant_text_state("Reading note", "", "edit_note", "update_reading_note", "Edit the reading note.")` on an interactive page. Typing into the field or calling `update_reading_note` with `{"note":"Chapter two"}` passes the text to the same handler. The compiled signature supplies the required field name and string result schema. The counter instead owns its input and publishes a decimal-string result schema.

With Node 22.7 or newer on the path, run `python scripts/verify_shared_actions.py target/debug/corvid.exe --package-only` to check the CLI, durable worker and extracted archive. From the extracted project, `corvid jobs enqueue --task read_page --payload '[40]'` queues the call. `corvid jobs run --source src/events.cor --max-runtime-ms 1500` executes it. Inspect the returned job ID with `corvid jobs inspect --job <id>` to see the result type and fingerprint. The full trial requires Playwright and a browser with WebMCP discovery and execution enabled.

The shelf-review example exposes `apply_reviewed_titles` on the proposal form. The user confirms the three edited titles and the form posts to `/proposals/apply`. The server checks the session, reviewer permission and proposal owner on submission. Retrying the same edits with the same proposal key returns the recorded outcome. Run `python scripts/verify_reading_list.py target/debug/corvid.exe --http-only` for the packaged permission and retry checks. On a WebMCP-enabled browser, the trial option `--webmcp --channel chrome` exercises discovery, fills the form through the discovered tool and confirms submission through the visible button. It checks the reply and shelf state after revocation and after access is restored. This browser path still needs a successful run.

The archive includes application sources and compiled browser assets. Static applications can omit `src/events.cor`. A build error preserves the previous successful archive.

### Edit a keyed cart and reuse a color picker

The [storefront](../../examples/storefront/src/main.cor) renders its catalog links, quantity inputs and checkout form on the server. The input name identifies the product in the submitted record. Its row key identifies the same product in the browser. Checkout computes the receipt from the two submitted quantities and the server's prices.

After the event module attaches, each line total reads the current input value, including edits made while the module was loading. Use the move buttons, or press Alt+Up or Alt+Down in a quantity field. The move handler shifts the adjacent row so the active input stays attached. A status message gives the row's new position.

The notebook and pencil screens each import `ColorPicker` and `Color` from [the same local module](../../examples/storefront/src/color_picker.cor). Each supplies an initial value and a form destination. For example, the notebook starts with `Color(0)` and calls `ColorPicker(value, "/notebook")`. With scripting enabled, the select's change event submits the form. **Apply color** submits it before enhancement. Each POST route decodes the selected shade into a `Color` and passes it to its screen handler. This example uses server-owned color previews and browser-local cart edits.

From the repository root, run `python scripts/verify_storefront.py target/debug/corvid.exe --http-only` to scaffold, check, test, package and serve the example on loopback. Omit `--http-only` to run the keyboard and focus tests with Python Playwright and Edge. The browser test uses 320 px and 960 px viewports.

### Compare a local action with the save form

The step 3 experiment adds **Read one page** beside the reading-list edit form. Its handler takes the displayed integer and returns the next value. **Save to shelf** submits a typed record to the server and updates SQLite. To build the experiment from the repository root, pass the path to your CLI executable and a new directory whose parent exists.

```powershell
python scripts/build_component_trial.py target/debug/corvid.exe target/pages-trial functions wasm
```

Extract the resulting web archive and migrate and serve it as in the reading-list example below. Open shelf 2, activate **Read one page**, and reload to see which value lasts. Use `indented` in place of `functions` to try the block grammar, or `js` in place of `wasm` to try the experimental JavaScript lowering. Each build needs its own output directory. The two small view files are inputs to the trial script. The ordinary CLI checks, tests and packages the generated Corvid project. [Source examples and measured sizes](../frontend-component-design.md#reading-list-prototype)

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
Copy-Item examples/reading_list/src/*.cor reading-list/src
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

### Search saved shelves

Choose **Search shelves**, enter part of a title and press Enter. The resulting URL, such as `/search?term=ody`, opens directly with the saved matches. The native GET form works with scripting disabled. With scripting enabled, typing previews matches and submitting adds the query to browser history. Back and Forward restore the submitted term.

Both routes call `find_titles`. The HTML route renders links on the server. The live preview calls `GET /search/results` and reads the Corvid `SearchResult` variant. `Matches` carries shelf numbers and titles. `Unavailable` lets the page offer another search after a database failure. Each edit aborts the preceding fetch. The callback checks its own signal after decoding the response so an obsolete search cannot replace newer links. Leaving the page aborts the pending request.

The verification script changes the required save field from `title` to `book_title` in a disposable installation and restarts the server on the same port. It compares the OpenAPI schemas and submits the old fields. The replacement service returns HTTP 400 with `invalid_form` and identifies the missing field before the save handler runs. The full trial keeps the old form open in a browser. The HTTP-only trial sends its encoded fields directly.

### Review edits to three shelves

The archive includes a second entry point, `src/reviews.cor`, for a shared reading list. Run the verification script for a local exercise with seeded sessions and a loopback model. For a configured service, start `corvid serve src/reviews.cor` from the migrated package, with the identity provider and durable session store configured. Use this entry point alone for a shared database. The starter in `src/main.cor` deliberately permits anonymous manual saves for the local exercise.

Choose **Suggest three titles** in a reviewer session. The review page shows each original title beside its editable replacement. Keep that page open, save a manual edit to shelf 2 in an editor session, then choose **Apply reviewed titles**. The receipt identifies each saved shelf and each shelf whose title changed during review.

The server reads all three revisions in one query before calling the model. Migration `002_proposals.sql` adds revisions to existing shelves and stores the review key with each shelf's original revision. Applying a review checks the revisions, updates eligible titles and records the submitted titles and per-shelf outcomes in one SQLite write transaction. Resubmitting the same review returns the recorded outcomes. Starting another review generates a new key. The HTML receipt and typed JSON result both call `apply_result`.

The packaged HTTP exercise discarded the committed response body, restarted the server and retried. Shelf 2 kept the manual edit, shelves 1 and 3 advanced once, and changing a submitted title on retry returned `KeyConflict`. The review and both apply routes require `shelves.review` at execution. Manual save requires `shelves.edit`. Migration `003_proposal_owner.sql` binds new previews to the verified actor ID. Older anonymous previews require a new review. The page sends the session's CSRF token with each form POST. The HTTP trial revoked the reviewer's role between preview and apply, observed HTTP 403 on both apply routes, and verified unchanged titles and revisions. A different reviewer presenting the key received `InvalidProposal`. Revoking the original reviewer's role also denied access to the saved receipt.

### Pause and revise a shelf extraction

From **Shelf extraction progress** in a reviewer session, enter a field to extract from each title and a maximum number of model operations. Starting the job saves all three source titles and their revisions in one transaction. The progress page shows the latest job for the signed-in reviewer. Use **Refresh progress** to read the next checkpoint.

From the same migrated package directory, configure a loopback model and run the worker in a second terminal. The schedule gives the worker one extraction opportunity per minute. To process one shelf immediately, run the CLI entry point once.

```sh
corvid schedule run --source src/extract.cor --workers 1
# One shelf through the CLI
corvid run --target interp src/extract.cor
```

Choose **Pause** while a shelf is in flight. A reserved extraction can finish, and each new worker checks the pause before reserving a shelf. Refresh after the current extraction finishes. Enter a correction for the completed shelf and a revised instruction, then choose **Save revision**. Restart the worker from the same directory and choose **Resume**. The page keeps the original model field, instruction and source revision beside the human correction. Remaining shelves use the revised instruction. Closing the progress page leaves the saved job available to its worker.

Each attempt reserves one operation before the prompt runs. Concurrent workers and retries use the same counter. A worker that loses its 45-second reservation cannot overwrite a later result. Recovery spends another operation if one remains. An exhausted budget marks remaining shelves as skipped. The optional per-operation price displays an estimate in millionths of a US dollar. Provider billing is unavailable for this prompt.

The packaged HTTP trial used two operations across three shelves. It verified a paused worker restart, a source edit after extraction, the saved correction and a skipped shelf. A separate job retried an empty result through the CLI and exhausted the same job budget. Browser verification of the close-and-reopen workflow is pending.

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
