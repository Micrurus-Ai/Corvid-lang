# General-purpose browser frontend roadmap

**Design target, November 2026.** A developer can build and ship a browser application in Corvid, from a static storefront to an offline editor or a live AI workspace. Standard features should work with the Corvid toolchain alone. Developers can bring an existing web package or use a new browser API before the standard library gains a convenience function for it. Browser permissions, device capabilities and engine support still govern what an application can do.

## Starting point

**Full-stack language commitment.** Every Corvid application should be able to define its own data types, components, screens, state, persistence, server operations and optional AI behavior in Corvid. The author chooses which layers the application needs. A local editor can keep an operation on device. A networked application can run its authoritative operation on the server. Both should be able to expose selected actions to an assistant using the same types and handlers their UI uses. Existing libraries and services remain available by choice.

**Extensibility test.** Starting from an ordinary project, an independent developer defines a domain type and an operation the Corvid team has never seen. They connect it to their own component, choose what an assistant may see and invoke, and build with the standard CLI. This must work without a change to the compiler, a domain plugin, a second agent-specific handler or an application template. The sample applications below exercise these capabilities.

**Accepted product decisions.** Ordinary frontend development should be possible in a Corvid project with the standard toolchain. CSS, JavaScript packages and web components are optional interoperability paths. Use ordinary Corvid types and control flow, typed component inputs and events, and an explicit browser/server boundary. The first preview must support a storefront cart and cancellable search. Ease of reading, writing and debugging is a release criterion. The precise view syntax and default browser compilation target remain open. [Component design candidates and usability checks](frontend-component-design.md)

Corvid already documents server-rendered HTML, typed forms and routes, browser-compiled handlers, streamed text and a web archive. The frontend-layout branch adds three layout functions and a shared integer control. Today's views carry HTML strings, and the shipped handler examples update integer or string values. These give us compatibility cases for a more general model. The checkboxes below track proposed work. [Browser guide](book/19-browser-views.md) · [View implementation](../std/ui.cor) · [Browser bindings](../crates/corvid-codegen-wasm/src/browser_controls.js)

## What to build first

Follow the [ordered implementation queue](../ROADMAP.md#remaining-work-in-execution-order) for sequencing across frontend, backend, agents and release work. This guide supplies the detailed frontend acceptance criteria.

### Grow a simple app in the same codebase

Developers choose their starting architecture. The growth trial below checks one path. A team building a large product can start the independent full-stack trial directly with its intended modules, UI, server, storage and AI workflows. For a headless agent, use the [large-project agent trial](../ROADMAP.md#building-agents-in-corvid). The delivery order describes how the Corvid team verifies the language.

Judge language and grammar proposals by two questions. How much must a newcomer learn to ship the first useful version? Can that developer build a more ambitious version by extending the code they already understand?

| Commitment | What the developer should experience |
| --- | --- |
| Invent new building blocks | Define an application-specific type, component and operation and compose them with ordinary Corvid code. |
| Read a feature end to end | Follow a screen event to its handler and identify where data leaves the device, where a model is called and where a change becomes durable. |
| Use application types with AI | Receive a model proposal in a developer-defined type. Validate, preview, edit, reject or apply it through the application's existing logic. |
| Keep local interactions fast | Handle typing, dragging and preview on the device. Make the server boundary explicit when an edit needs shared state or authoritative validation. |

- [x] **First version.** In the first working application below, build a screen that lists, edits and saves records using ordinary Corvid. Start it and use it without model credentials or agent configuration.
- [ ] **Later version.** Extend that same project with a model suggestion and an author-selected action that an authorized assistant can use. Reuse the original types, view components and save handler where their meaning still applies. Generate agent bindings from the exposed action. Exercise both UI and assistant invocation without screen scraping or a separately authored agent API.
- [ ] **Review the change.** Give another developer the diff between versions. Ask them to trace one edit, identify new concepts and show which existing code they could reuse. Record any forced rewrite, duplicated domain rule or unnecessary setup as a design issue. Use the developer's own application domain for the independent trial.

**First-version trial, November 21, 2026.** The [reading list](book/19-browser-views.md#package-and-restart-a-reading-list) started with `corvid new`. Setup required copying the application entry point and migration into the scaffold, extracting the web archive, then applying its migration from the extracted directory. The generated form exposes both fields, including the destination shelf number. No browser-side code or model configuration was needed to serve the form.

A Windows loopback trial with the existing Corvid 0.0.1 debug CLI (4e3ba2b0) saved five successive titles on shelf 2, rejected a malformed integer with HTTP 400, and read the last title and both untouched shelves from a replacement server. The five POST round trips took 26.42, 23.78, 40.61, 21.04 and 22.70 ms (median 23.78 ms), timed from the Python HTTP request through reading the full HTML response, including the SQLite write and list readback. A separate five-run Edge trial (158.0.4370.33) used keyboard navigation to edit and save a shelf. Median list navigation was 25.50 ms, save navigation was 49.10 ms and HTTP save time was 31.63 ms on loopback. After restart, all three shelves matched their expected values. The application test passed through `corvid test`. Rebuilding the Rust browser-view suite exhausted the available disk space.

### How developers should experience Corvid

| Design goal | Trial for the first working application |
| --- | --- |
| Easy to read | Give an unfamiliar developer one screen and its handler. Before running it, ask them to point to the network call, the data sent to the model and the line that saves an edit. Compare their prediction with the actual behavior. |
| Easy to write | From the documented starter, ask the developer to add a field and show it on screen. Track time to a working change, duplicated declarations, documentation lookups and places where they need help. Try both candidate view styles on the same task. |
| Easy to understand when something breaks | Change the type of the field. Ask the developer to use the diagnostic to find the incompatible caller and fix it. Separately, interrupt a request and ask them to explain what the user sees and what was saved. |
| Reliable during collaboration | Have a model propose edits to three independently editable records. Display the proposed differences in the existing view. While the preview is open, manually change one record. Applying all three should save the two eligible edits and identify the conflict, under an application policy that permits partial application. Retry the request and replay the scenario as a test with a recorded model response. |

Use existing language constructs for the trial. If developers repeatedly misunderstand the same line, revise the API, naming or grammar and repeat the task with fresh readers. Document each adopted function or grammar rule with a runnable example in the learning guide. The three-record exercise extends the single-record application once its basic save and reload path works.

**Design goal for shared actions.** Define an operation once in Corvid. From its typed inputs, outcomes and explicit exposure, the toolchain should derive browser bindings, server boundary validation and the assistant's action schema. The developer owns the handler and its authorization rules. An exposed input or result can be narrower than an internal database record. The editor should let the developer follow a UI event or assistant binding back to the handler that can change application state.

Use a change to a real action as an acceptance test for the component and action prototype. For example, replace a required integer `quantity` with a record containing `amount` and `unit`. Use equivalent types in the independent developer's chosen application.

| Check | Expected outcome |
| --- | --- |
| Rebuild the app | The compiler identifies incompatible Corvid call sites at their source lines. Generated schemas reflect the new public input. A field private to the server stays out of browser payloads and assistant schemas. |
| Deploy while an old tab is open | Compare the public contract to the previously released version and flag the breaking change. Exercise the author's chosen transition: an explicit version adapter or a structured incompatibility response that leaves stored data intact. |
| Receive a malformed model suggestion | Validate it before presenting it as actionable. Show a recoverable error. A valid suggestion can be previewed; applying it goes through the same authorized handler as a human edit. |
| Retry a committed write after a lost response | For actions that support retry, reuse the caller's operation key. The same logical request returns its recorded outcome and applies the change once. Reusing the key with different input returns a conflict. Independent user edits get distinct keys. |

Use one small application to integrate the stack early. The [main roadmap](../ROADMAP.md) already records typed HTTP routes and SQLite persistence (S01–S02), scoped sessions (S04), typed model results and streaming (A01–A02), and browser forms and packaged handlers (F01–F03). Those checks used specific runtimes and providers. Reuse those paths and test them together through the public tooling.

| Order | Deliverable | Decision it enables |
| --- | --- | --- |
| First working application | Start with `corvid new`. Build a keyboard-accessible screen that edits a typed record, saves it, reloads it after a server restart, and requests a typed model suggestion. Let the user apply the suggestion through an ordinary action. Run from the packaged build. Exercise malformed model output and a canceled request. | Identify where developers still need glue code, undocumented setup or a second application language. Put each gap in the work areas below. |
| Component and action prototype | Compare the two candidate component styles on that application. Bind a developer-defined action once, test it from the UI and a compatible assistant, and report binding errors at their Corvid source line. Include a local-only action. | Choose syntax and execution target after measuring readability, edit-to-result time and browser cost. |
| Independent full-stack trial | A developer outside the language team chooses a different app, builds it from the book and deploys it. Exercise its UI, authenticated server operation, persistent data, real model call and authorized assistant action in the same deployment. Retain deterministic provider tests for repeatable development. | Make this a condition for claiming that developers can ship a full-stack AI-native web application in Corvid. Record the Corvid version, external services and every manual workaround. |

Bring a minimal edit, rebuild, debug and package workflow into the first application. Expand browser API coverage, advanced offline features and package interoperability as separate deliverables. Core preview usability can be tested while a browser-specific assistant adapter is still experimental. Feature-detect that adapter and keep ordinary interaction usable when it is absent. [Web Platform Baseline](https://web.dev/baseline) · [Chrome WebMCP](https://developer.chrome.com/docs/ai/webmcp)

## Design and implementation order

The numbered sections group related engineering work. Use the delivery order above to choose the tasks needed for each working application.

### One action for the user and the assistant

A music player developer writes `pause_song` once. Clicking **Pause**, using the keyboard, or asking an authorized assistant to pause should call that same function. The developer can keep `delete_playlist` available only in the human interface. The player is an example. Any developer should be able to introduce an action with their own name, types and behavior.

The existing `ui.agent_query_form` prototype emits discovery metadata for a read-only GET form. Discovery and invocation of arbitrary developer-defined actions are planned work. Start the following prototype alongside component design, before settling the syntax.

- [ ] **Write and bind.** In an ordinary project, define a local action and bind it to a keyboard-accessible control. Let the author explicitly expose it to an assistant and choose which context fields the assistant receives. Derive the input and result schemas from Corvid types.
- [ ] **Exercise both paths.** Verify that the button and a compatible assistant produce the same state change through one handler. Hide playlist deletion from assistant discovery and verify that a direct assistant request for it also fails. Repeat after renaming or moving the visible button.
- [ ] **Try a server-owned action.** Keep the same action contract and component binding. Make the network boundary, pending state and possible failure visible in source. Test a permission change between discovery and invocation and a retry after a lost response.
- [ ] **Teach and generalize.** Record the working syntax and functions in the learning guide and reference. Have another developer use them for an unfamiliar action in a fresh project without editing Corvid itself. Check whether they can predict where it runs and who may invoke it by reading the source.

### 1. Settle the component and execution model

- [ ] Specify how an ordinary handler becomes a typed action, where it executes, and how its author exposes selected inputs, results and context. Review a browser-local example alongside a server-owned one before choosing syntax.

- [x] Draft cart and search behavior with two candidate view styles for developer review. [Read the examples](frontend-component-design.md)
- [ ] Reuse ordinary functions, records, lists, variants and imports. Specify typed inputs, children, callbacks, component identity and cleanup. Give every proposed keyword a motivating example. Ask developers unfamiliar with the design to predict each program's behavior from its source.
- [ ] Prototype one interactive component through the normal web build. Compare DOM updates compiled to JavaScript with Corvid's existing WebAssembly handler path on initial bytes, startup, event latency and diagnostic quality. Record the default target and how compute-heavy modules interoperate. Keep existing forms and pages running during migration.

**Exit:** approve a small language specification, two readable source examples and an execution-target decision backed by the prototype.

### 2. Render, style and compose real components

- [ ] Bind a developer-defined action to a native control. Check that a keyboard user can invoke it and that changing the control's label, layout or styling preserves the action identity.

- [ ] Implement typed elements, attributes, text, children, conditionals and keyed lists. Preserve focus, selection and element identity when a list changes. Escape data by default and make trusted markup explicit.
- [ ] Support semantic HTML, SVG, native forms, accessible names, keyboard navigation, and component-scoped or global styles in Corvid projects. Preserve the CSS cascade, media and container queries, custom properties and the ability to import a stylesheet.
- [ ] Specify server rendering and how interactive regions attach to existing HTML. Handle a server/client mismatch with a source diagnostic. Let links and forms work before browser code loads.

**Exit:** build a responsive catalog and editable cart with developer-authored components. Reordering items preserves focus in the active quantity field. Checkout works by keyboard and the catalog remains usable with scripting disabled.

### 3. Make state, events and asynchronous work general

- [ ] Route UI and assistant invocations through the same handler. Check authority at execution, capture the intended target across selection changes, and return typed failures for stale requests. Exercise retry of a server mutation after a lost response and verify that it applies once.

- [ ] Support local and shared typed state, derived values, batched updates and predictable scheduling. Bind typed input, pointer, keyboard and composition events. Define event propagation, default actions and subscription cleanup.
- [ ] Provide typed requests, streams, loading and error states, cancellation, retry and cleanup on navigation. Carry the same types across a Corvid client/server route. Diagnose browser imports of server-only code or secrets.
- [ ] Integrate URL parameters, browser history, nested routes and deep links with loading and focus restoration.

**Exit:** move and edit tasks on a board with keyboard or pointer. Two views update from one record. An older search response cannot overwrite a newer result. Leaving a streaming AI view cancels its subscription. Provider credentials stay on the server.

### 4. Open the browser platform and ecosystem

- [ ] Generate discoverable schemas from explicitly exposed action types. Test discovery and invocation with a compatible browser assistant. Verify the same UI works with the adapter absent and that direct requests for an unexposed action are refused.

- [ ] Design typed bindings for DOM and browser APIs using Web IDL where applicable. Cover promises, callbacks, events, nullable values, typed arrays, exceptions and object lifetime. Offer an explicit interop declaration for platform extensions. [Web IDL](https://webidl.spec.whatwg.org/)
- [ ] Import JavaScript modules and custom elements, and export a Corvid component for another app to embed. Specify package resolution, versions, browser/server compatibility and source locations for interop errors.
- [ ] Exercise files and clipboard with permissions, IndexedDB storage, workers and transferable data, canvas graphics, audio/media and capability detection. Add offline caching and version updates. Document a fallback when a capability is unavailable.

**Exit:** build an offline drawing editor in Corvid that handles pointer input, moves work off the UI thread, persists a drawing and restores it after reload. Separately, consume a third-party custom element and embed a Corvid component in a JavaScript host.

### 5. Make the daily development loop and deployment dependable

- [ ] Show developers which actions a build exposes, their types, and where they run. When a handler's input changes, point to incompatible bindings in the Corvid source. Teach exposure and debugging through a runnable learning example.

- [ ] Provide a web project template, local dev server, fast rebuilds, state-preserving updates where safe, editor completions, component navigation and browser stack traces mapped to Corvid source.
- [ ] Package assets, fonts and scoped styles with deterministic URLs, production caching and deployable static and server builds. Exercise upgrades with an old browser tab still open.
- [ ] Support component tests, real-browser interaction tests and server rendering tests through documented commands. Cover invalid programs in compiler diagnostics. Document every public function and grammar change alongside its implementation in the book and reference.

**Exit:** on a clean machine, a developer follows the book to create, debug, test, build and deploy a custom component. An intentional browser-side error points to the original Corvid line.

## Developer preview and stable release

### Application runtime speed

Aim for a shorter wait for users on equivalent applications. Start measuring with the first packaged app and repeat after each runtime optimization. Use an idiomatic production TypeScript implementation as the first comparison. Keep validation, authorization, durability, rendered results, and cancellation behavior equivalent. Exercise these paths separately so a fast browser update cannot hide a slow save.

| User action | Measure | Optimization to test |
| --- | --- | --- |
| Edit one record in a long list | Input to next visible update on a named phone, including slow interactions. Also measure first load on a cold cache. | Update only affected views and keep transient input state on the device. |
| Save and reopen the record | Click to durable acknowledgement, then verify the value after restart. Record median and p95 latency under the same concurrent load. | Profile serialization, database operations and network round trips before changing the runtime. |
| Request and cancel an AI suggestion | Time to first visible token, time to completion and time from cancel to upstream disconnect. | Stream directly and release abandoned work. Use the same deterministic streaming responder to isolate Corvid overhead, then report a separate real-provider run. |

Use the same hardware, network conditions, payloads and database configuration. Separate cold and warm runs, interleave implementations, retain raw samples and report uncertainty. Pick a primary metric for each workload before timing it. Publish a speed advantage only for workloads where the improvement repeats and exceeds measurement noise. Track regressions and slower results alongside wins. [Benchmark protocol](../benches/README.md)

Track the extensibility test alongside the general component work. Exercise both a browser-local operation and a server-backed operation through human and agent interfaces. Check typed context, concurrent edits and permission changes wherever they apply. [Research, prototype and acceptance tests](ai-operated-apps.md)

Start testing accessibility, performance and browser behavior with the first prototype. Use the following criteria to decide what a release can promise.

| Decision | Acceptance criteria |
| --- | --- |
| Developer preview | The component model and cart/search workflows work end to end. Publish the supported browser versions, known gaps, install instructions, migration path from existing UI helpers and a runnable non-AI example. |
| Shared-action preview | An independent developer adds a new action in a fresh project. A keyboard user and a compatible assistant invoke it through one handler. A second action stays private to the human interface. Record the tested browser and assistant versions. |
| Browser compatibility | Run core journeys in Chromium, Firefox and WebKit, plus real mobile checks for touch and text input. Track optional capabilities individually. Use Baseline and relevant Web Platform Tests to inform the support matrix. [Baseline](https://web.dev/baseline) · [Web Platform Tests](https://web-platform-tests.org/) |
| Accessibility | Check keyboard interaction, focus order, screen-reader announcements, zoom, reduced motion and native control semantics in representative applications. Track applicable WCAG 2.2 AA criteria. [WCAG 2.2](https://www.w3.org/TR/WCAG22/) |
| Performance | On a named midrange phone, compare equivalent production builds for transferred bytes, input latency, layout shifts and memory after repeated navigation. Report model time separately from UI work. Once field traffic is sufficient, track LCP, INP and CLS at the 75th percentile by mobile and desktop. [Core Web Vitals](https://web.dev/articles/vitals) |
| Stable release | Complete the independent full-stack trial. Use the storefront, offline editor and streaming workspace to cover capabilities the chosen app does not exercise. Publish compatibility and upgrade commitments for components, browser bindings and packages. |

### AI features to exercise across the whole application

- [ ] Connect the UI to the same agent used by the CLI and worker. From the progress view, let a person pause, correct a result, revise the task and resume. Use the main roadmap's handoff and replay scenarios as the acceptance tests. [Building agents in Corvid](../ROADMAP.md#building-agents-in-corvid)

The main roadmap records runtime work on streams, durable jobs, budgets, provenance and replay. Track the remaining user-facing behaviors during the independent app trials.

- [ ] Close a view-owned search and verify cancellation. Close a view observing a saved job, reopen it after a worker restart, and recover progress under the same user account.
- [ ] Follow a model-extracted field from its visible value to the source revision. Correct the field manually and check that the UI distinguishes the correction from the original extraction.
- [ ] Give parallel work and retries one operation budget. Show completed and skipped work, the reason it stopped, and whether cost is estimated or provider-reported.
- [ ] Turn a reported UI interaction into a sanitized regression test. Reproduce the visible outcome using recorded model data with external writes disabled.

### Will developers choose Corvid again?

Use the independent app trial and the existing friends-and-family round to learn where developers reach for another stack. Record these outcomes separately from release engineering checks. Choosing an existing package through supported interop is a successful outcome when the developer keeps building their application in Corvid.

| Question | What to observe |
| --- | --- |
| Can a newcomer start without help? | From a clean install, let them build a feature they chose using public learning materials. Record time to the first useful result, first blocker and any maintainer intervention. Include people building ordinary apps as well as AI apps. |
| Can a team live with the app? | A teammate clones it, debugs a reported problem, changes a shared type, upgrades the toolchain and deploys the change. Record missing packages, migration surprises, debug time and any step requiring private instructions. |
| Do they choose it again? | At an opt-in follow-up after 30 days, ask what they actually used for their next feature or project and why. Separate continued use, switching stacks, no new work and no response. |
| Does a recommendation lead to use? | Record voluntary referrals separately from invited trial participants. With consent, follow whether a referred developer installed Corvid and built something independently. |

Report counts alongside the number invited and the follow-up window. Small trials identify friction and cannot establish a market-wide retention rate. Turn recurring blockers into named implementation or documentation tasks, then ask fresh developers to repeat the affected journey.

**First task:** take the smallest editable record through the first working application above using today's documented syntax. Keep an existing `std/ui` page running through the same CLI. Use the first point of friction to guide the component prototype.
