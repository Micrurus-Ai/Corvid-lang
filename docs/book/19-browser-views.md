# Browser views

A view can be an ordinary Corvid function. Import `std/ui` in a scaffolded project and return a `View` to reuse a control. Pass a list of views to a container to set their order.

```corvid-fragment
# src/main.cor
import "std:ui" as ui
import "std:ui" use View
import "std:io" use io_write_text

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

To show the tips when the page first loads, use `ui.disclosure_with_initial_state("Search tips", true, [ui.text("Use a word from the note.")])`. Passing `false` starts it collapsed, like `ui.disclosure`. Readers can toggle either version with scripts disabled.

`ui.text`, `ui.text_field`, `ui.submit`, `ui.disclosure`, and `ui.page` escape their text arguments for HTML. Pass user input through those functions.

Use `ui.trusted_markup` when you supply the HTML yourself. For example, keep a fixed heading in a trusted fragment and pass a reader's note as a separate text view. Corvid copies the fragment verbatim, so keep user input out of it. Constructing a `View` directly also treats its `html` field as trusted markup.

```corvid-fragment
fn notes_page(note: String) -> String:
    heading = ui.trusted_markup("<h1 data-topic='notes'>Field notes</h1>")
    return ui.page("Notes", [heading, ui.text(note)])
```

Use `ui.paragraph` to mix inline views in a single paragraph. Its children appear in the order you supply them. Include any spaces between words in the text itself.

Wrap important words in `ui.strong`. Pass them as `ui.inline_text` to escape the text, or include other inline views such as links.

For a multiline preview, use `ui.preformatted_text(note)`. The trail guide uses it below the **Current note** heading so that blank lines and indentation survive the trip from the text area to the preview. The browser determines the block’s writing direction from the note itself.

When a reader's note appears inline with its label, pass the note to `ui.isolated_text`. It escapes the text and wraps it in a native `bdi` element so the browser determines the note's direction independently.

```corvid-fragment
fn note_preview(note: String) -> View:
    label = ui.strong([ui.inline_text("Current note")])
    return ui.paragraph([label, ui.inline_text(": "), ui.isolated_text(note)])
```

Use `ui.query_form(path, controls)` to submit named fields to an application route with GET, and `ui.link(label, path)` to navigate there. Supply a trusted application path for each.

To give the form a landmark name, use `ui.named_query_form("Packing note preview", "/guide", [note_field, ui.submit("Preview")])`. The first argument names the whole form. Keep the field’s own label so readers can also identify the individual control.

To show a hint when scripting is disabled, place `ui.no_script("Select Preview to reload the guide with your note.")` in the form's list of views. The server includes the escaped message in a `noscript` paragraph. The browser decides whether to display it.

When the link label includes markup, pass inline views in reading order. For example, `ui.link_with_content("#route", [ui.inline_text("Route, "), ui.abbreviation("3 km", "3 kilometres")])` makes the whole label clickable. Include any spaces between children in the text.

For an opt-in, read-only search that a compatible browser agent can discover, use `ui.agent_query_form(name, description, path, controls)`. Supply a stable unique tool name, an author-written description, and a trusted read-only GET path. The visible form and the agent use the same named fields. The user submits the form. On browsers without declarative WebMCP, the form works as a normal GET form. The server remains responsible for authorizing the request.

```corvid-fragment
fn calendar_search() -> View:
    day = ui.text_field("Day", "day", "")
    return ui.agent_query_form("find_slots", "Find available calendar slots for a day", "/slots", [day, ui.submit("Find slots")])
```

## Static view functions

A layout can give each slot a name and a `View` parameter. To fill one slot with several siblings, pass `ui.fragment(content)`. In the example below, both packing items become direct children of the list. Pass `ui.fragment([])` when there are no items.

```corvid-fragment
fn packing_list(heading: View, items: View) -> View:
    return ui.fragment([heading, ui.unordered_list([items])])

fn weekend_packing() -> View:
    items = ui.fragment([
        ui.list_item([ui.inline_text("Water")]),
        ui.list_item([ui.inline_text("Map")])
    ])
    return packing_list(ui.h2("Weekend packing"), items)
```

### Pass context across a layout

In the trail guide, the route supplies `GuideContext("/guide")` to the page. The page passes that record to the packing panel, which passes it to the note editor and the suggestions form. Both forms read `context.preview_path` for their GET action. To render the guide for a different destination, supply another record at the page call. The context belongs to that render and follows ordinary typed function arguments.

### Let the caller render a preview

The trail guide's packing panel takes a callback named `render_preview` with type `(String) -> ui.View`. The panel calls `render_preview(note)` while building the page and places the returned view below **Current note**. The guide supplies a preformatted preview to preserve line breaks. Another caller can supply a paragraph using the same panel.

```corvid-fragment
packing = packing_panel(note, context, fn (text: String) -> ui.text(text))
```

### Recover inside a component slot

The packing component returns a `Result<PackingPanel, String>`. On success, the record carries two views. The layout renders `content` inside the packing section and `footer_action` in the page footer, after the main landmark. This lets the component send its suggestions form across a layout boundary during server rendering. The form still uses the preview path from the component's context.

On failure, the layout keeps the route details and the footer's navigation link. It fills the packing section with escaped error text and supplies an empty footer action.

```corvid-fragment
panel = match packing:
    Ok(details) -> details
    Err(reason) -> PackingPanel(ui.text(reason), ui.fragment([]))
```

Run `corvid serve examples/trail_guide/src/main.cor --listen 127.0.0.1:8080` from the repository root and open `http://127.0.0.1:8080/`. The [trail guide](../../examples/trail_guide/src/main.cor) combines a navigation landmark, route marker, ordered stops, distance table and packing form. Turn off JavaScript, enter a note such as `Rain & <wind>`, and select **Preview**. The GET route renders the note as text and fills the field with the same value.

### Give the page an outline

Use `ui.h1` for the page heading, `ui.h2` for its sections, and `ui.h3`, `ui.h4`, `ui.h5`, or `ui.h6` for successively deeper subsections. Each accepts a string and escapes it, so a title such as `Maps & <trails>` appears as literal text.

```corvid-fragment
fn trail_page(title: String) -> String:
    return ui.page_in_language("en", title, [ui.h1(title), ui.h2("Equipment"), ui.text("Bring a compass.")])
```

Pass the document's primary language to `ui.page_in_language`. For example, `"en"` sets `lang="en"` on the root HTML element so readers and speech tools can identify the language of the page.

Use `ui.page_with_landmarks` when navigation or footer links belong to the whole document. It places the three lists of views in a page-level header, main region and footer, in that order.

```corvid-fragment
fn trail_guide_page() -> String:
    header = [ui.h1("Trail guide"), ui.navigation("Guide sections", [ui.link("Route", "#route")])]
    route_section = ui.section("route", "Route", [ui.text("Follow the green markers.")])
    return ui.page_with_landmarks("en", "Trail guide", header, [route_section], [ui.link("Back to route", "#route")])
```

### Group navigation links

To link to a named section, pair `ui.link("Packing note", "#packing")` with `ui.section("packing", "Packing note", [form])`. The section starts with an `h2`. Its ID gives the link a destination, and the heading supplies the region's name through `aria-labelledby`. Choose a unique, nonempty ID without whitespace. Child views follow the heading in list order.

Use `ui.aside("Suggested supplies", [checklist])` to group supplementary material in a named complementary landmark. The trail guide uses this for the packing checklist alongside the note editor.

Use `ui.navigation(label, content)` to group links in a native `nav` landmark. Give the group a name that describes its destinations. Corvid escapes the label for `aria-label` and renders the children in list order.

```corvid-fragment
fn trail_navigation() -> View:
    links = [ui.link("Map", "/map"), ui.link("Trails", "/trails")]
    return ui.navigation("Trail guide", links)
```

### Include an image

Use `ui.image("/images/trail-map.png", "The trail forks left at the bridge")` to place an image in a list of views. Choose a trusted source URL and describe what the image contributes to the page. For a purely decorative image, pass an empty string as the second argument. Corvid escapes both attribute values.

When you know the image size, pass its width and height in CSS pixels. The trail guide uses `ui.image_with_dimensions(marker_url, "Green circle trail marker", 24, 24)` so the browser can reserve space before the marker loads.

To show a visible caption below an image, use `ui.figure("Follow this marker to the lookout.", [marker])`, where `marker` is the image view. The image's alternative text describes the marker. The caption tells readers where it leads.

### Attach application data to a group

`ui.data_group` wraps child views in a `div` with a custom data attribute. Supply the name after `data-`. For example, `trail-id` produces `data-trail-id`. The name must contain at least one character and may use lowercase ASCII letters, digits and hyphens. An invalid name returns `Err`. The value can contain quotes or other text because Corvid escapes it.

```corvid-fragment
fn trail_card(id: String, title: String) -> Result<View, String>:
    return ui.data_group("trail-id", id, [ui.h2(title), ui.link("Map", "/map")])
```

### Write a list with text and links

If a route lists only selected stops, use `ui.numbered_list_item(3, [ui.inline_text("Lookout")])` inside an ordered list to keep the lookout's original stop number. The next ordinary item continues at 4.

Use `ui.inline_text` for text alongside a link. It escapes the string and preserves surrounding spaces. Put each item's children in reading order with `ui.list_item`. Use `ui.unordered_list` for bullets or `ui.ordered_list` for numbering. The first argument to `ui.ordered_list` sets the first item's number, so a second page can continue an earlier list.

```corvid-fragment
fn reading_plan(topic: String) -> View:
    introduction = ui.list_item([ui.inline_text("Read "), ui.link(topic, "/guide")])
    supplies = ui.unordered_list([ui.list_item([ui.inline_text("Notebook")])])
    exercise = ui.list_item([ui.inline_text("Bring these supplies"), supplies])
    return ui.ordered_list(3, [introduction, exercise])
```

### Build a data table

Pass a caption, column names, and a list of rows to `ui.table`. The caption and column names are escaped text. Each column name becomes a header with `scope="col"`. Supply one view per cell in column order. Use `ui.inline_text` for a plain value or a view such as `ui.link` for a navigable cell.

When the first cell names the row, use `ui.table_with_row_headers` with the same arguments. In the trail guide, “Bridge” becomes a `th` with `scope="row"`, identifying the stop that the distance describes.

For a distance cell, `ui.abbreviation("3 km", "3 kilometres")` displays “3 km” and puts the expanded wording in the native `abbr` element’s `title` attribute. Both arguments accept ordinary text.

To give the distance a machine-readable value in metres, use `ui.data_value("3000", [ui.abbreviation("3 km", "3 kilometres")])`. Readers see “3 km” and the HTML carries `<data value="3000">` around the abbreviation.

For a walking time, use `ui.time_value("PT45M", [ui.inline_text("45 minutes")])`. Readers see “45 minutes”. The native `time` element carries the same duration in its `datetime` attribute. You can also supply a date such as `"2026-11-23"` and choose the date wording your readers will see.

```corvid-fragment
fn reading_table(topic: String) -> View:
    guide = [ui.link(topic, "/guide"), ui.inline_text("Ready")]
    notes = [ui.inline_text("Field notes"), ui.inline_text("Draft")]
    return ui.table("Reading queue", ["Title", "Status"], [guide, notes])
```

### Arrange views

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
| `text_area(label: String, name: String, initial: String) -> View` | A labeled multiline field. Use `ui.text_area("Your note", "note", note)` to show a submitted packing note for further editing, including any leading blank line. |
| `checkbox_field(label: String, name: String, checked: Bool) -> View` | Use `ui.checkbox_field("Trail map", "map", false)` for a box the reader can tick with scripts disabled. Pass `true` to start with the box checked. |
| `checkbox_field_with_value(label: String, name: String, value: String, checked: Bool) -> View` | Put `ui.checkbox_field_with_value("Trail map", "supply", "map", false)` inside a form to submit `supply=map` when checked. An unchecked box leaves the field out of the submission. |
| `text_area_with_requirement(label: String, name: String, initial: String, required: Bool) -> View` | Use `ui.text_area_with_requirement("Your note", "note", note, true)` when the browser should require a value before submitting the form. Pass `false` for an optional field. |
| `text_area_with_rows(label: String, name: String, initial: String, rows: Int, required: Bool) -> View` | The trail guide uses `ui.text_area_with_rows("Your note", "note", note, 4, true)` to show four lines while editing. Longer notes can scroll. Pass zero or a negative row count for the browser default. |
| `submit(label: String) -> View` | A submit button for its enclosing form. |
| `option_group(label: String, options: List<View>) -> View` | Group related choices inside a select with `ui.option_group("Navigation", [ui.option("Pack a map", "Trail map", false)])`. The group label describes the choices. Selecting “Trail map” still submits “Pack a map”. |
| `select_field(label: String, name: String, options: List<View>) -> View` | Put `ui.select_field("Suggested note", "note", [ui.option("Pack a map", "Trail map", true)])` beside a submit button. The reader sees “Trail map”. Submitting the form sends “Pack a map” as the note. |
| `reset_button(label: String) -> View` | Put `ui.reset_button("Restore note")` beside Preview. After editing the note, select Restore note to return the form controls to the values rendered on the current page. This works with scripts disabled. |
| `search_form(controls: List<View>) -> View` | A GET form targeting the current page. |
| `query_form(path: String, controls: List<View>) -> View` | A GET form targeting the supplied application path. |
| `disclosure(label: String, content: List<View>) -> View` | Expandable content with a summary. |
| `fieldset(legend: String, controls: List<View>) -> View` | Group related form controls under a visible legend. For example, pass `ui.fieldset("Contact details", [name_field, email_field])` as a child of a form. |
| `link(label: String, path: String) -> View` | A link to the supplied application path. |
| `page(title: String, content: List<View>) -> String` | An HTML document that renders the views in order. |

## Serve a page and an API

In a project created by `corvid new notes`, use this as `src/main.cor`.

```corvid-fragment
import "std:ui" as ui

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
import "std:ui" as ui

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

The shelf-review example exposes `apply_reviewed_titles` on the proposal form. The user confirms the three edited titles and the form posts to `/proposals/apply`. The server checks the session, reviewer permission and proposal owner on submission. Retrying the same edits with the same proposal key returns the recorded outcome. Run `python scripts/verify_reading_list.py target/debug/corvid.exe --http-only` for the packaged permission and retry checks. On a WebMCP-enabled browser, the trial option `--webmcp --channel chrome` exercises discovery, fills the form through the discovered tool and confirms submission through the visible button. It checks the reply and shelf state after revocation and after access is restored. Chrome 157 passed the packaged trial. Keep the registered form available until its tool reply is delivered. The example guards duplicate submissions and renders the reply in a separate status region because altering or removing the form during execution can cancel the browser's pending reply.

The archive includes application sources and compiled browser assets. Static applications can omit `src/events.cor`. A build error preserves the previous successful archive.

### Edit a keyed cart and reuse a color picker

The [storefront](../../examples/storefront/src/main.cor) renders its catalog links, quantity inputs and checkout form on the server. The input name identifies the product in the submitted record. Its row key identifies the same product in the browser. Checkout computes the receipt from the two submitted quantities and the server's prices.

After the event module attaches, each line total reads the current input value, including edits made while the module was loading. Use the move buttons, or press Alt+Up or Alt+Down in a quantity field. The move handler shifts the adjacent row so the active input stays attached. A status message gives the row's new position.

The notebook and pencil screens each import `ColorPicker` and `Color` from [the same local module](../../examples/storefront/src/color_picker.cor). Each supplies an initial value and a form destination. For example, the notebook starts with `Color(0)` and calls `ColorPicker(value, "/notebook")`. With scripting enabled, the select's change event submits the form. **Apply color** submits it before enhancement. Each POST route decodes the selected shade into a `Color` and passes it to its screen handler. This example uses server-owned color previews and browser-local cart edits.

From the repository root, run `python scripts/verify_storefront.py target/debug/corvid.exe --http-only` to scaffold, check, test, package and serve the example on loopback. Omit `--http-only` to run the keyboard and focus tests with Python Playwright and Edge. The browser test uses 320 px and 960 px viewports.

The verifier edits the notebook price in the scaffold, rebuilds it and serves the two extracted archives separately. Twelve notebooks and one pencil total 5,525 cents in the original and 5,825 cents after the edit. It fetches the rebuilt JavaScript and Wasm over HTTP and checks their content types and bytes. To repeat the same keyboard journey with installed Playwright engines, run the command once each with `--browser chromium`, `--browser firefox` and `--browser webkit` in place of `--http-only`.

To check a shared edit across both screens, add `--lsp target/debug/corvid-lsp.exe` to the storefront trial command. The trial opens both callers and the picker over stdio. Go to Definition at either call opens the declaration in `color_picker.cor`. The trial changes `change_path` from `String` to `Int` and converts it back to a string inside the picker body. The component stays well typed, and both callers report the incompatible destination on the binding line. Restoring the original buffer clears both errors without saving.

To exercise editor feedback, open `src/notebook.cor` with the Corvid language server. Change the first argument of `ColorPicker(value, "/notebook")` to `"Moss"`. In the stdio trial the server reported that the argument required `Color` and received `String` on line 7. Restoring `value` cleared the diagnostic before saving.

### Compare a local action with the save form

The step 3 experiment adds **Read one page** beside the reading-list edit form. Its handler takes the displayed integer and returns the next value. **Save to shelf** submits a typed record to the server and updates SQLite. To build the experiment from the repository root, pass the path to your CLI executable and a new directory whose parent exists.

```powershell
python scripts/build_component_trial.py target/debug/corvid.exe target/pages-trial functions wasm
```

Extract the resulting web archive and migrate and serve it as in the reading-list example below. Open shelf 2, activate **Read one page**, and reload to see which value lasts. Use `indented` in place of `functions` to try the block grammar, or `js` in place of `wasm` to try the experimental JavaScript lowering. Each build needs its own output directory. The two small view files are inputs to the trial script. The ordinary CLI checks, tests and packages the generated Corvid project. [Source examples and measured sizes](../frontend-component-design.md#reading-list-prototype)

### Install the picker in a second app

Run `python scripts/verify_picker_package.py target/debug/corvid.exe --work-dir target/picker-trial` from the repository root on Windows. Choose a new directory for each run. The trial signs the storefront's original picker, publishes it into a local directory, runs `corvid new palette`, then installs `@trial/color-picker@1.0.0`. The [palette app](../../examples/palette/src/main.cor) imports the versioned package and declares its own typed POST route.

The picker imports the compiler's bundled UI module with `import "std:ui" as ui`. The trial disconnects the registry after verifying the lock and cached source, then checks, tests, packages and serves the palette offline. The extracted web archive includes the locked package. The picker arranges the select and submit button with `ui.row`. The extracted app serves their wrapping flex layout and the standard change-event module. The HTTP trial submits `shade=1` and checks that Moss is selected, then submits `shade=3` and checks for HTTP 400. The compiler check passes a string to the installed picker and verifies that the diagnostic points to the binding line in the palette source.

On a runner with Playwright and Edge, add `--browser-channel msedge` to the trial command. It checks the computed wrapping style, tabs to the select and chooses Moss with the arrow key. With scripting disabled, it tabs to Apply color and presses Enter. Both paths should post the same shade. The HTTP and diagnostic checks passed locally. The browser run stopped at Playwright import on this runner.

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
import "std:ui" as ui
import "./search" use Search
import "std:ui" use View
import "std:io" use io_write_text

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

When one request writes a shelf and displays the updated list, open the database once. In the example, `save_and_show` passes the same `DbHandle` to `update_shelf` and `read_shelves`. The update commits before the query runs. Callers that only need to list or save can still use `list_readings()` or `save(reading)`. [Reading-list source](../../examples/reading_list/src/main.cor)

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

The compiler supplies the standard modules and the scaffold configures a project-relative I/O root. The archive includes the SQL migration. Running it in the extracted directory creates and seeds the database there. Reuse that directory across restarts. No model setup is required.

The [verification script](../../scripts/verify_reading_list.py) repeats the scaffold, package, migration and restart in fresh directories. With Python 3.12+, Python Playwright and its Chromium browser installed, run `python scripts/verify_reading_list.py path/to/corvid` from the repository root. It reports list and save navigation durations from the browser and separate HTTP save round trips. Use `--channel msedge` for installed Edge or `--http-only` for the server checks alone.

### Search saved shelves

Choose **Search shelves**, enter part of a title and press Enter. The resulting URL, such as `/search?term=ody`, opens directly with the saved matches. The native GET form works with scripting disabled. With scripting enabled, typing previews matches and submitting adds the query to browser history. Back and Forward restore the submitted term.

Both routes call `find_titles`. The HTML route renders links on the server. The live preview calls `GET /search/results` and reads the Corvid `SearchResult` variant. `Matches` carries shelf numbers and titles. `Unavailable` lets the page offer another search after a database failure. Each edit aborts the preceding fetch. The callback checks its own signal after decoding the response so an obsolete search cannot replace newer links. Leaving the page aborts the pending request.

The verification script changes the required save field from `title` to `book_title` in a disposable installation and restarts the server on the same port. It compares the OpenAPI schemas and submits the old fields. The replacement service returns HTTP 400 with `invalid_form` and identifies the missing field before the save handler runs. The full trial keeps the old form open in a browser. The HTTP-only trial sends its encoded fields directly.

### Review edits to three shelves

The archive includes a second entry point, `src/reviews.cor`, for a shared reading list. Run the verification script for a local exercise with seeded sessions and a loopback model. For a configured service, start `corvid serve src/reviews.cor` from the migrated package, with the identity provider and durable session store configured. Use this entry point alone for a shared database. The starter in `src/main.cor` deliberately permits anonymous manual saves for the local exercise.

To discover the action for an existing proposal, POST its `proposal_key` as a form field to `/proposals/context` with the reviewer's session cookie and CSRF header. An owner with current review permission receives the same `apply_reviewed_titles` form with three empty title fields. Discovery keeps the key bound to that proposal and leaves the titles and revisions in storage. A peer reviewer gets a page without the form. An expired session receives HTTP 401 and loss of review permission produces HTTP 403. To expose this proposal to a local MCP host, run `python scripts/reading_review_mcp.py --app-port PORT` and enter its key at the prompt. Configure the printed loopback endpoint with the calling reviewer's session cookie and CSRF header. The adapter binds the proposal when it starts. It discovers the three editable string fields from the named form and rechecks discovery before each call. Have the host show the three edits for confirmation before submitting them. The tool returns the typed receipt from `/proposals/apply-result` as JSON text.

To exercise the adapter in a disposable installation, install `mcp==1.30.0` in a Python environment and run `python scripts/verify_reading_list.py PATH_TO_CORVID --http-only --remote-mcp`. The SDK client negotiated protocol version 2025-06-18. The packaged trial exercised revocation after discovery, a concurrent shelf edit and retry after restarting both processes.

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

### Trace a suggestion and replay its preview

Open **Suggestion trace** beneath the proposed title. For a two-call suggestion, the first `recommend` chose `Lookup`. The trace shows the shelf number, the title read from the request's snapshot and the final `Suggest` result. A direct suggestion skips the lookup. The disclosure belongs to that preview, so two open tabs can be inspected separately.

To make a regression from a successful suggestion, save the HTML response body for that one `POST /suggest` request from the browser's Network panel as `preview.html`. Keep the raw response local because the form and trace contain the original titles. From the repository root, run the following commands with your scaffolded project in `reading-list` and a new output directory named `regression`.

```powershell
python scripts/promote_reading_suggestion.py preview.html regression
Copy-Item regression/suggestion.jsonl reading-list/src
Get-Content reading-list/src/main.cor,regression/suggestion-test.cor -Encoding utf8 | Set-Content reading-list/src/suggestion-test.cor -Encoding utf8
cd reading-list
corvid test src/suggestion-test.cor
```

The exporter keeps the recorded action sequence, shelf arguments, title equality and HTML-escaping cases. It substitutes example text for the titles. The generated test supplies an in-memory shelf snapshot and runs the same suggestion agent and form renderer as the HTTP route. Corvid's `from_trace` runtime matches the prompt inputs, supplies the recorded decisions and quarantines external writes. Inspect the assertions for the selected shelf, escaped title, save action and lookup branch.

The browser trial captures the response to the click that displayed the trace. It generates the regression from that response and runs it after shutting down the loopback model. Run `python scripts/verify_reading_list.py path/to/corvid --channel chrome --work-dir browser-trial` to keep the files. Changing the second prompt's context failed the regression. Adding a call to the save handler also failed, and every shelf retained its title and revision. Run `python scripts/verify_reading_list.py path/to/corvid --http-only --work-dir reading-trial` to retain the generated fixture and suite for inspection.

## Displaying a streamed answer

```corvid-fragment
# src/main.cor
import "std:ui" as ui

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
import "std:ui" as ui

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

## A component with named state (prototype)

The compiler accepts the short counter directly in a project's `src/main.cor`. From `examples/reading_counter`, run `corvid check` and `corvid build src/main.cor --target web`. The web package gives a single component its own page and includes its compiled click handlers. A project with an explicit server can compose the component as a view. [Native counter](../../examples/reading_counter/src/main.cor)

Each copy of a component owns its state. In this example both buttons update the same count. Mounting a second counter gives it its own count. Reloading starts each counter at zero.

```text
component ReadingCounter:
    state pages_read = 0

    text("Pages read this session: {pages_read}")

    button("Read one page") on click:
        pages_read += 1

    button("Read two pages") on click:
        pages_read += 2
```

The initializer `0` gives the state its integer type. You can also write `state pages_read: Int = 0`. The braces in the text display the current value. Each assignment runs when its button is activated, including by keyboard. To count five pages per click, change the label to `Read five pages` and the assignment to `pages_read += 5`.

The prototype supports one nonnegative integer initializer, one labeled output ending in the state reference, and one or more buttons that add a nonnegative integer literal. It generates an ordinary Corvid view and typed browser handlers, then uses Corvid's web packager. The generated host composes two instances using ordinary function calls. The click handlers run locally. AI action exposure is a separate design decision.

From the repository root, use a new output directory and the path to your built CLI (shown here for Windows).

```shell
python scripts/prototype_named_state.py examples/named_state/reading_counter.view counter-app --cli target/debug/corvid.exe
```

The package is written to `counter-app/target/web.tar`. With Python Playwright and Edge installed, run `python scripts/verify_named_state.py target/debug/corvid.exe` to exercise the packaged page. [Editable example](../../examples/named_state/reading_counter.view)

### Navigation and visibility design

For future component examples, use a link when the reader chooses a destination, as in `link("Reading history", to: ReadingHistory)`. Use an ordinary `if` around content that appears conditionally. Reserve `open ReadingHistory` for navigation initiated by code and `on open:` for work that should run once when a component instance first appears. These are design directions for a future navigation example. The counter prototype implements named state, text, buttons and click handlers.

### Sharing a component with another app

Use `import "std:ui" as ui` inside a component that will be published. The compiler supplies the standard library even after the component moves to a registry. Export the component and its input type with `public`. The consuming screen can pass a typed value and its own form action. On an interactive page, standard change-submit selects work without a custom browser module. [Publisher exercise](../reference/package-imports.md#publishing-a-component-with-standard-library-dependencies)

### Use a browser calendar in a Corvid form

The booking screen imports its picker on one line.

```corvid
import "./pickers" use calendar_picker as picker
```

Change `calendar_picker` to `native_picker` to use the browser's date control. Both pickers take the saved date and return a view. The server receives the same `Booking` value and checks the date before writing to SQLite. [Booking application](../../examples/booking/src/main.cor)

Use `ui.date_field(label, name, initial)` for a required date input. On an interactive page, `ui.bind_field(field, component)` connects that input to a custom element with a string `value` property and a `change` event. The example loads a pinned Cally module with an integrity hash. The native input remains usable if the download fails. [Picker definitions](../../examples/booking/src/pickers.cor)

Wrap the picker and submit button in `ui.agent_post_form(name, description, path, controls)` to expose the form in a WebMCP-capable browser. Chrome derives the tool argument from the named field. The person can review the filled date and submit it. The tool receives the server's receipt. Run `python scripts/verify_booking.py target/debug/corvid.exe --channel chrome` with Python Playwright, Chrome and network access to the calendar module. [Browser trial](../../scripts/verify_booking.py)

### Embed a Corvid counter in a JavaScript app

Write the initial view in `src/component.cor`. Its entry agent returns the HTML from ordinary Corvid view helpers. Put browser handlers in `src/events.cor`. In the [reading counter](../../examples/embedded_counter/src/component.cor), the button calls a Corvid handler that adds one to the current count.

From the checkout root, export the component with a CLI built from this branch:

```shell
python scripts/export_browser_component.py examples/embedded_counter --cli target/debug/corvid.exe
```

Copy the resulting `target/wasm/` directory into your site's static assets, for example at `./widgets/counter/`. Register its default export once in your JavaScript entry module:

```js
import ReadingCounter from "./widgets/counter/index.js";
customElements.define("reading-counter", ReadingCounter);
```

Place `<reading-counter></reading-counter>` wherever the control belongs. Each instance owns its shadow DOM and count. Moving an instance preserves its state. Reloading creates fresh instances. The host serves static files over HTTP, including the generated JavaScript and WebAssembly. It needs no running Corvid service. [Complete HTML host](../../examples/embedded_counter/index.html)

The exporter renders trusted component markup at build time. The generated events module provides `componentElement(html)`, which returns a custom element class and binds Corvid state controls when an instance first connects. This example covers locally managed state and keyboard actions. To run the browser trial with Python Playwright and Chrome, use `python scripts/verify_embedded_counter.py target/debug/corvid.exe`.

## Draw pixel strokes in a worker

From `examples/drawing_editor`, build the stroke module and serve the example on loopback.

```sh
corvid check src/stroke.cor
corvid build src/stroke.cor --target wasm
npm ci
npm test
python -m http.server 8000 --bind 127.0.0.1
```

Open `http://127.0.0.1:8000/` and drag across the canvas when “Drag to draw” appears. A tap should paint one cell. The page maps pointer coordinates onto a 96 × 64 grid and captures the pointer until the stroke ends. The module worker loads the generated Wasm wrapper, calls Corvid to interpolate the cells between consecutive positions, and transfers a typed array back to the canvas.

Wait for “Drawing saved.” before reloading. Each worker response writes its painted coordinates to IndexedDB in one transaction. Repainting the same cell reuses its key, so two tabs can add strokes without overwriting each other's saved marks. The editor loads the keys before enabling pointer input. If storage is unavailable, you can still draw for the current session.

To try offline drawing, wait for “Ready for offline reload.” and stop the local HTTP server. Reload the same URL, add another mark, wait for the save and reload again. The service worker caches the HTML, JavaScript modules and compiled Wasm. When changing those files, bump the cache version in `offline.js`, restart the server and close older editor tabs so the update can activate. Offline setup requires a first online visit over HTTPS or loopback.

The Node tests exercise the generated worker, reopen the sketch using an in-memory IndexedDB implementation and check the cache handler against the built files. With Python Playwright and Chrome installed, run `python scripts/verify_drawing_editor.py` from the repository root after building the example. That trial draws a line, stops its loopback server, checks the restored canvas pixels and repeats the reload after an offline edit.

### Check the storefront across browsers and devices

Run the packaged storefront trial with Python Playwright. Install its Firefox and WebKit engines with `python -m playwright install firefox webkit` and use an installed Google Chrome for the Chromium check.

```shell
python scripts/verify_storefront.py target/debug/corvid.exe --channel chrome
python scripts/verify_storefront.py target/debug/corvid.exe --browser firefox
python scripts/verify_storefront.py target/debug/corvid.exe --browser webkit
```

Each engine checks keyboard navigation, checkout at widths of 320 and 960 pixels, and the two picker screens with and without scripting. Chrome and WebKit also run touch-device simulations. Corvid's `ui.link` uses a native anchor with `tabindex="0"` so a link participates in normal keyboard focus order in the tested WebKit configuration.

For a physical iPhone trial, open the packaged storefront on a reachable test origin in Safari. Tap **Open cart**, then **Notebook quantity**. Check that the numeric keyboard appears. Replace the quantity with **12**, leave the pencil quantity at **1**, dismiss the keyboard, tap **Move Notebook later**, then **Checkout**. Expect **Total: 5525 cents**. Record the iOS version, whether each control was reachable and the displayed total.

For a screen-reader trial, listen for **Open cart**, **Notebook quantity**, the move button's label and the updated position when the notebook moves. The Windows Narrator run on November 23 confirmed the link, quantity and position. The move button's exact spoken label still needs confirmation. Narrator's speech recap can be used to check a phrase. [Microsoft Narrator guide](https://support.microsoft.com/en-us/accessibility/windows/narrator/complete-guide-to-narrator)

### Debug a browser calculation

When a browser handler hits a failing integer division or remainder, the message identifies the expression that failed. For example, entering two notebooks in the deliberately broken storefront trial gives this message in Chrome.

```text
Corvid handler notebook_cents failed: divide by zero at src/pricing.cor:4:12 in line_total (integer division)
```

Open `src/pricing.cor` at line 4, column 12. The cart called `notebook_cents`, which called `line_total` in that file. Its divisor was `quantity - 2`. For this cart the line total is simply `quantity * unit_price`. After changing that line and rebuilding, the same two notebooks cost 900 cents and checkout with one pencil totals 1,025 cents.

With Python Playwright and its three browser engines installed, run `python scripts/verify_browser_diagnostics.py target/debug/corvid.exe` from the repository root. Pass `--channel chrome` to use installed Chrome. The trial triggers both operations, checks their individual source locations and completes checkout with the corrected calculation in Chromium, Firefox and WebKit.

### Browser asset caching

Build the web package and serve its extracted contents with `corvid serve`. The loader keeps its stable URL, `/target/wasm/events.js`. Browsers can send its ETag on subsequent visits. When the bytes are unchanged, the server responds with HTTP 304 and an empty body.

When the page needs Wasm, the loader requests `events.wasm?v=<SHA-256>`, where the version identifies the exact compiled bytes. Corvid serves that URL with a one-year immutable cache policy. A backend-only edit can reuse both assets. A change to the Wasm produces a new URL and updates the loader automatically. Keep the full query string in any CDN cache key. An uncached request for a previous build's hash receives HTTP 404 from the new server. Reloading the page retrieves the current loader.

Run `python scripts/verify_browser_cache.py target/debug/corvid.exe --channel chrome` with Python Playwright, installed Chrome, Firefox and WebKit. The trial serves three extracted packages at one local origin and records the actual browser requests using persistent profiles.

### Small integer handlers in the browser

Write the calculation once in `src/events.cor`. For example:

```corvid
public agent line_cents(quantity: Int) -> Int:
    return quantity * 450
```

The compiler can emit JavaScript for a single return expression using integer parameters, integer literals, addition, subtraction and multiplication. Each operation keeps the browser backend's signed 64-bit result. Calls, branches, division, remainder and other operations use the existing Wasm implementation. The application author uses the same Corvid syntax for either path.

A packaged standard page containing quantity fields, optional keyed lists, and the standard event script at the end of its content can load `events.quantity.js`. It binds the fields and reordering controls and can run eligible handlers directly, even if another function in the module needs Wasm. A field referencing a different handler can load the general entrypoint. Pages that also use state panels, form enhancements, assistant controls or custom scripts keep the general bindings.

From JavaScript, the public `instantiate(hostOrImports, options)` function still returns the handler object. An eligible events module uses JavaScript automatically if all its handlers qualify. Use `instantiate({}, {backend: "wasm", trace: events})` to force Wasm for comparison or `instantiate({}, {trace: events})` to record the automatic path, where `events` is an array. Both report run arguments, results and failures. Browser integer arguments and results use `bigint`. [Performance measurements](../guides/performance.md#compiled-browser-handlers)

### Reordering long lists

The standard keyed list keeps keyboard focus on the moved row and announces its current position. In generated lists the browser runtime obtains an interior row's position directly from the DOM, avoiding a JavaScript copy of all sibling elements on every move. It counts only elements when hand-authored markup introduces whitespace or comments. The position follows the live order even after application code moves rows. [Browser measurements](../guides/performance.md#reordering-long-lists)

## Drawing and resizing with a Stroke

A drag handler receives two points measured in local pixels. Read their integer coordinates as `stroke.start.x`, `stroke.start.y`, `stroke.end.x`, and `stroke.end.y`. Each mounted component owns its integer state. Reloading the page restores the initial value.

```corvid
component Sketch:
    state marks = 0
    text("Brush marks: {marks}")
    canvas(320, 160)
    on drag(stroke: Stroke):
        line(stroke.start, stroke.end)
        marks += 1
```

On a canvas, pressing the pointer makes a dot. Moving it emits a segment from the previous sample. Arrow keys move the drawing cursor one pixel, or ten with Shift. The optional `line` statement paints the segment. The state assignment runs once per segment.

```corvid
import "std:ui" as ui

component Sidebar:
    state width = 200
    text("Panel width: {width}")
    panel(width, 96, 480, [ui.text("Notes")])
    on drag(stroke: Stroke):
        width += stroke.end.x - stroke.start.x
```

The panel takes its width state, minimum and maximum widths, and a list of child views. Drag its separator or focus it and use Left and Right to resize. The width stays inside the declared bounds.

To expose a mounted instance to a compatible assistant, wrap it in `ui.assistant_drag(Sketch(), "add_stroke", "Draw a line between two canvas points.")`. Choose a unique tool name per instance on the page. The tool accepts the same start and end points as the local handler and returns the updated state as decimal text. See the [complete app](../../examples/pointer_components/src/main.cor).

## Build a color picker from a record

The [RGB picker](../../examples/rgb_picker/src/color_picker.cor) is an ordinary function returning a view. Its author chooses the record, layout, controls, and preview. Each slider name matches a field of the record.

```corvid
public type Color:
    red: Int where between(0, 255)
    green: Int where between(0, 255)
    blue: Int where between(0, 255)

public fn ColorPicker(color: Color, path: String) -> ui.View:
    red = ui.slider("Red", "red", color.red, 0, 255)
    green = ui.slider("Green", "green", color.green, 0, 255)
    blue = ui.slider("Blue", "blue", color.blue, 0, 255)
    return ui.selection(path, ui.stack([red, green, blue]), selected_color(color))
```

The page mounts the picker inside `ui.interactive_page`. A pointer or keyboard change sends the three current values to the same-origin form route. Corvid decodes and validates the record, then calls the application's renderer.

```corvid
route POST "/color" form Color -> html:
    return selected_color(body).html
```

Return a trusted view fragment from this route. The browser updates only that picker's preview and keeps keyboard focus. A failed request restores the last accepted values. Treat the route as a preview calculation because multiple selections can overlap in flight.

Use `ui.assistant_selection(picker, "select_color", "Choose an RGB color.")` to publish a mounted instance's selection action to a compatible browser assistant. The assistant supplies all three named integers and receives the preview's text. Each picker can use its own action name. The [complete page](../../examples/rgb_picker/src/main.cor) shows how to mount it.

## Keep a record in browser memory

The [RGB picker](../../examples/rgb_picker/src/color_picker.cor) opens with a local editor at `/`. Declare the record once, including each field's valid range. Inside the component, `browser state selected = color` takes the initial value and its declared bounds from the record. Backend code can use the same `Color` type.

```corvid
public type Color:
    red: Int where between(0, 255)
    green: Int where between(0, 255)
    blue: Int where between(0, 255)

public fn LocalColorPicker(color: Color) -> ui.View:
    browser state selected = color
    controls = ui.stack([
        ui.local_slider("Red", bind selected.red),
        ui.local_slider("Green", bind selected.green),
        ui.local_slider("Blue", bind selected.blue)
    ])
    preview = ui.row([
        ui.record_swatch(bind selected.red, bind selected.green, bind selected.blue),
        ui.record_text("RGB {red}, {green}, {blue}")
    ])
    return ui.local_selection(selected, controls, preview)
```

Mount it on an interactive page. Each instance owns a separate record in browser memory. Slider movement updates the preview during the gesture. Wrapping the editor with `ui.assistant_selection` exposes one action taking a complete record. Both input paths validate the candidate before updating the preview. The initial page and runtime must load online. Edits then work disconnected until you close or reload the page. For a server-computed preview, the same app offers the earlier form example at `/server`.

## Edit agent settings offline

The [agent settings editor](../../examples/agent_settings/src/main.cor) defines one model for both human and assistant input.

```corvid
choice RunMode:
    Manual
    Scheduled

type AgentProfile:
    name: String where len_between(1, 60)

type AgentTools:
    web_search: Bool
    search_limit: Int where between(1, 20)

type AgentSettings:
    profile: AgentProfile
    tools: AgentTools
    mode: RunMode
    interval_minutes: Int where between(1, 60)
    retries: Int where between(0, 5)
```

Declare the state in the component and bind each control to a field. Editing `profile.name` preserves the other settings. Preview text can read the same path with `{profile.name}`.

```corvid
browser state settings = initial
controls = ui.grid:
    ui.field("Agent name", bind settings.profile.name)
    ui.checkbox("Allow web search", bind settings.tools.web_search)
    if settings.tools.web_search:
        ui.field("Search result limit", bind settings.tools.search_limit)
    ui.segmented_choice("Run mode", bind settings.mode)
    if settings.mode == Scheduled:
        ui.field("Interval in minutes", bind settings.interval_minutes)
    ui.field("Retry limit", bind settings.retries)
```

A typo such as `bind settings.mdoe` fails compilation. Binding the checkbox directly to `settings.profile.name` also fails because that field is text. For a dropdown, use `ui.field("Run mode", bind settings.mode)`. For adjacent radio options, use `ui.segmented_choice` as above. Both edit the same `RunMode` value and expose the same assistant action.

Select **Allow web search** to reveal the search result limit. Clearing the checkbox hides the limit and preserves its value. Choose **Scheduled** to reveal the interval, change it, then switch to **Manual**. Tab skips the hidden interval. Choose **Scheduled** again to recover its value. An assistant changing the mode triggers the same update.

An opted-in assistant supplies a complete record in one call. For example, `{"profile":{"name":"Scout"},"tools":{"web_search":true,"search_limit":5},"mode":"Scheduled","interval_minutes":15,"retries":3}` updates every control and the preview together. Invalid values leave the committed record intact. The editor keeps its values in browser memory until reload. A run mode entered in this example represents a setting. Connecting a scheduler to it is application code.

### Edit and reorder a list of agents

Open `/agents` in the packaged agent settings example. Each row starts with a record from the application's typed list. The browser keeps its editable values and position until reload.

```corvid
fn agent_rows(initial: List<WorkspaceAgent>) -> ui.View:
    browser state members = initial
    return ui.stack:
        for agent in members by agent.id:
            ui.field("Agent name", bind agent.name)
            ui.checkbox("Enabled", bind agent.enabled)

fn agent_list(workspace: Workspace) -> ui.View:
    new_agent = WorkspaceAgent("agent", "New agent", true)
    editor = ui.list_editor(agent_rows(workspace.agents), agent_rows([new_agent]), "Add agent", "Remove agent")
    return ui.assistant_reorder(editor, "move_agent", "Move an agent up or down by its ID.")
```

Give each agent a unique, nonempty string ID. Use editable fields such as the name for the controls and keep the ID stable. Start editing the second name and press Alt+Up. The row moves with the caret. An assistant can call the opted-in action with `{"key":"writer","direction":"up"}` to make the same move while the user is typing. On an editable list, calling the reorder action with `{}` returns the current IDs in display order.

**Add agent** creates a row from the supplied template, including its initial name and enabled setting. The template key supplies the prefix for new IDs such as `agent-1` and `agent-2`. **Remove agent** removes just that row. If you remove the last row, the Add button remains available. Use `ui.value(bind agent.id)` when displaying the generated ID.

### Show a field in a custom live preview

Use `ui.value` to place an editable field's current value inside a view you design. For example, a nameplate can accept a view and place it beside a label.

```corvid
fn nameplate(name: ui.View) -> ui.View:
    return ui.row:
        ui.text("Agent name")
        name

fn name_editor(initial: AgentProfile) -> ui.View:
    browser state profile = initial
    control = ui.field("Agent name", bind profile.name)
    preview = nameplate(ui.value(bind profile.name))
    return ui.local_selection(profile, control, preview)
```

The nameplate updates as the user types or an opted-in assistant submits a valid change. The browser writes the name as plain text and keeps the input focused. Use the same pattern for nested fields, booleans, bounded integers, and named choices. The layout function runs when the page is rendered. Only the bound text needs to change during editing.

### Give a custom container the same syntax

A child block passes one list to the function named before the colon. Each line adds one child. A nested block adds the view returned by its container.

```corvid
fn settings_panel(children: List<ui.View>) -> ui.View:
    return ui.stack(children)

fn panel_preview() -> ui.View:
    return settings_panel:
        ui.text("Agent settings")
        ui.row:
            ui.text("Manual")
            ui.text("Scheduled")
```

Use `settings_panel(children)` or `ui.grid(children)` when you already have a list, including a list built in a loop. Use `ui.grid([])` for an empty grid.
