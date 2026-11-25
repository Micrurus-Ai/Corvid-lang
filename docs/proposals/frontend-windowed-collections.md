# F17 fixed-height collection trial

A parts inventory has 10,000 numbered bins. A person can inspect a bin in a list or a two-column stock table, open its detail page, and return to the same bin. New bins arrive at the end of the inventory while the person is reading. Only visible rows, a small margin, and the focused row need DOM nodes. Rendering every bin would make startup and DOM memory proportional to inventory size.

The browser calls ordinary Corvid integer functions compiled by the public Wasm target to determine the visible range and stock level. A shared DOM adapter handles native links, keyboard input, scroll geometry and history. Compare this build with the same adapter calling TypeScript functions over the same data. Keep data synthetic and public. This exercises existing frontend interop, introduces no grammar or backend interface, and grants no assistant access. A real backend could supply the same append events from an authorized feed.

Build the collection and its bounded rendering first, then check keyboard movement, Back restoration and streamed appends alongside cold/repeat load, bytes, input, frames and memory. Use the same viewport and workflow in Chromium, Firefox and WebKit. Variable-height rows, arbitrary cell editors and changing sort order are outside this trial.

## Measurement protocol

For ordinary bin activation, push the link URL into browser history and render the detail in the current document. Back and Forward render from the URL and saved collection state. This keeps one model instance across detail visits and avoids allocating another document for each visit. Keep native links for direct entry, modified clicks and opening another tab. Explicitly include both view links in sequential focus order along with the active bin.

The active bin link is the collection's single Tab stop. Give the scrolling viewport a negative tabindex so browsers that make scrolling containers keyboard-focusable skip the container during sequential navigation. Native Tab and Shift+Tab then enter and leave through the active link without custom key handlers.

Run on an idle host, on loopback, headless at 960 × 720 CSS pixels and device scale 1. Record the revision, CLI build, Node and browser versions, OS and host model alongside stdout. Each view gets four paired trials per engine, alternating which implementation runs first. A fresh browser context provides a cold cache, then revisits the same URL for the warm measurement. Both versions use identical HTML, CSS, DOM updates and response cache headers.

| Metric | Regression ceiling | Sample |
| --- | ---: | --- |
| Startup body bytes | 65,536 | HTML and all scripts and Wasm required by the page, uncompressed. The browser also reports transfer sizes where supported. |
| Cold / warm ready | 2,000 / 1,000 ms | Navigation start until the collection has mounted and handlers are installed. |
| Input latency | 100 ms | Larger of the p95 from a trusted movement key to the first animation callback observing changed focus, and the time from keyboard activation of Receive shipment to its first DOM update. Verify each of the 32 intermediate counts. |
| Frame interval | 50 ms | Larger of the p95 frame intervals during keyboard movement and the 32-event shipment. |
| Navigation round trip | 2,000 ms | p95 over five keyboard detail visits and Back operations, including driver time and verification of focus and scroll offset. |
| Working-set growth | 128 MiB | Largest sampled process-tree resident set after load, keyboard movement, navigation or shipment above the blank page. Includes Playwright and shared pages counted by each process. |
| Mounted rows | 16 | Maximum observed in each trial, including overscan and a pinned focused row. |

Timing and memory ceilings apply to the median of four trials for each engine, implementation and view. The row cap applies to every trial. Both implementations met these ceilings in both views on Chromium, Firefox and WebKit. [Hosted run](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45585464423) Compare the printed medians for the paired implementations and keep the individual samples for diagnosis.
