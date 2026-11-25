# A web project from the first command

F16 starts with `corvid new palette --web`. A new developer can edit a
page, run its rendering test and preview the result on loopback. Requiring
developers to copy an example first makes it difficult to tell which files
are necessary. A flag lets the web starter share the existing project
manifest and directory checks. The Python tools option can accompany it.

The starter uses `ui.page("Welcome", [ui.text("Hello, Corvid!")])`. The
backend serves the rendered page through an ordinary GET route. Browsers
can read it with scripting disabled. The app declares no assistant actions.
This slice introduces no grammar or runtime interface. The remaining F16
work checks the edit and debug loop, editor support and package distribution.

When the starter imports a component from a registry, its editor should
complete the same public names as a local module and open the source the
build uses. Read the exact URI from the project lock and verify the installed
bytes. Fetching registry content during completion could select a different
source and would make routine editing depend on a connection.
