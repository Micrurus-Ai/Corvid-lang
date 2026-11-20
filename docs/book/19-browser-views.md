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
