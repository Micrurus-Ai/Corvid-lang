# General-purpose browser frontend roadmap

**Design target, November 2026.** A developer can build and ship a browser application in Corvid, from a static storefront to an offline editor or a live AI workspace. Standard features should work with the Corvid toolchain alone. Developers can bring an existing web package or use a new browser API before the standard library gains a convenience function for it. Browser permissions, device capabilities and engine support still govern what an application can do.

## Starting point

**Full-stack language commitment.** Every Corvid application should be able to define its own data types, components, screens, state, persistence, server operations and optional AI behavior in Corvid. The author chooses which layers the application needs. A local editor can keep an operation on device. A networked application can run its authoritative operation on the server. Both should be able to expose selected actions to an assistant using the same types and handlers their UI uses. Existing libraries and services remain available by choice.

**Extensibility test.** Starting from an ordinary project, an independent developer defines a domain type and an operation the Corvid team has never seen. They connect it to their own component, choose what an assistant may see and invoke, and build with the standard CLI. This must work without a change to the compiler, a domain plugin, a second agent-specific handler or an application template. The sample applications below exercise these capabilities.

**Accepted product decisions.** Ordinary frontend development should be possible in a Corvid project with the standard toolchain. CSS, JavaScript packages and web components are optional interoperability paths. Use ordinary Corvid types and control flow, typed component inputs and events, and an explicit browser/server boundary. The first preview must support a storefront cart and cancellable search. Ease of reading, writing and debugging is a release criterion. The precise view syntax and default browser compilation target remain open. [Component design candidates and usability checks](frontend-component-design.md)

Corvid already documents server-rendered HTML, typed forms and routes, browser-compiled handlers, streamed text and a web archive. The frontend-layout branch adds three layout functions and a shared integer control. Today's views carry HTML strings, and the shipped handler examples update integer or string values. These give us compatibility cases for a more general model. The checkboxes below track proposed work. [Browser guide](book/19-browser-views.md) · [View implementation](../std/ui.cor) · [Browser bindings](../crates/corvid-codegen-wasm/src/browser_controls.js)

## Design and implementation order

### 1. Settle the component and execution model

- [x] Draft cart and search behavior with two candidate view styles for developer review. [Read the examples](frontend-component-design.md)
- [ ] Reuse ordinary functions, records, lists, variants and imports. Specify typed inputs, children, callbacks, component identity and cleanup. Give every proposed keyword a motivating example. Ask developers unfamiliar with the design to predict each program's behavior from its source.
- [ ] Prototype one interactive component through the normal web build. Compare DOM updates compiled to JavaScript with Corvid's existing WebAssembly handler path on initial bytes, startup, event latency and diagnostic quality. Record the default target and how compute-heavy modules interoperate. Keep existing forms and pages running during migration.

**Exit:** approve a small language specification, two readable source examples and an execution-target decision backed by the prototype.

### 2. Render, style and compose real components

- [ ] Implement typed elements, attributes, text, children, conditionals and keyed lists. Preserve focus, selection and element identity when a list changes. Escape data by default and make trusted markup explicit.
- [ ] Support semantic HTML, SVG, native forms, accessible names, keyboard navigation, and component-scoped or global styles in Corvid projects. Preserve the CSS cascade, media and container queries, custom properties and the ability to import a stylesheet.
- [ ] Specify server rendering and how interactive regions attach to existing HTML. Handle a server/client mismatch with a source diagnostic. Let links and forms work before browser code loads.

**Exit:** build a responsive catalog and editable cart with developer-authored components. Reordering items preserves focus in the active quantity field. Checkout works by keyboard and the catalog remains usable with scripting disabled.

### 3. Make state, events and asynchronous work general

- [ ] Support local and shared typed state, derived values, batched updates and predictable scheduling. Bind typed input, pointer, keyboard and composition events. Define event propagation, default actions and subscription cleanup.
- [ ] Provide typed requests, streams, loading and error states, cancellation, retry and cleanup on navigation. Carry the same types across a Corvid client/server route. Diagnose browser imports of server-only code or secrets.
- [ ] Integrate URL parameters, browser history, nested routes and deep links with loading and focus restoration.

**Exit:** move and edit tasks on a board with keyboard or pointer. Two views update from one record. An older search response cannot overwrite a newer result. Leaving a streaming AI view cancels its subscription. Provider credentials stay on the server.

### 4. Open the browser platform and ecosystem

- [ ] Design typed bindings for DOM and browser APIs using Web IDL where applicable. Cover promises, callbacks, events, nullable values, typed arrays, exceptions and object lifetime. Offer an explicit interop declaration for platform extensions. [Web IDL](https://webidl.spec.whatwg.org/)
- [ ] Import JavaScript modules and custom elements, and export a Corvid component for another app to embed. Specify package resolution, versions, browser/server compatibility and source locations for interop errors.
- [ ] Exercise files and clipboard with permissions, IndexedDB storage, workers and transferable data, canvas graphics, audio/media and capability detection. Add offline caching and version updates. Document a fallback when a capability is unavailable.

**Exit:** build an offline drawing editor in Corvid that handles pointer input, moves work off the UI thread, persists a drawing and restores it after reload. Separately, consume a third-party custom element and embed a Corvid component in a JavaScript host.

### 5. Make the daily development loop and deployment dependable

- [ ] Provide a web project template, local dev server, fast rebuilds, state-preserving updates where safe, editor completions, component navigation and browser stack traces mapped to Corvid source.
- [ ] Package assets, fonts and scoped styles with deterministic URLs, production caching and deployable static and server builds. Exercise upgrades with an old browser tab still open.
- [ ] Support component tests, real-browser interaction tests and server rendering tests through documented commands. Cover invalid programs in compiler diagnostics. Document every public function and grammar change alongside its implementation in the book and reference.

**Exit:** on a clean machine, a developer follows the book to create, debug, test, build and deploy a custom component. An intentional browser-side error points to the original Corvid line.

## Developer preview and stable release

Track the extensibility test alongside the general component work. Exercise both a browser-local operation and a server-backed operation through human and agent interfaces. Check typed context, concurrent edits and permission changes wherever they apply. [Research, prototype and acceptance tests](ai-operated-apps.md)

Start testing accessibility, performance and browser behavior with the first prototype. Use the following criteria to decide what a release can promise.

| Decision | Acceptance criteria |
| --- | --- |
| Developer preview | The component model and cart/search workflows work end to end. Publish the supported browser versions, known gaps, install instructions, migration path from existing UI helpers and a runnable non-AI example. |
| Browser compatibility | Run core journeys in Chromium, Firefox and WebKit, plus real mobile checks for touch and text input. Track optional capabilities individually. Use Baseline and relevant Web Platform Tests to inform the support matrix. [Baseline](https://web.dev/baseline) · [Web Platform Tests](https://web-platform-tests.org/) |
| Accessibility | Check keyboard interaction, focus order, screen-reader announcements, zoom, reduced motion and native control semantics in representative applications. Track applicable WCAG 2.2 AA criteria. [WCAG 2.2](https://www.w3.org/TR/WCAG22/) |
| Performance | Set budgets from representative workloads on a named midrange mobile device. Track shipped bytes, time to first usable interaction, input latency during large-list edits and memory after repeated navigation. Compare equivalent production builds and publish the workload and raw measurements. |
| Stable release | A responsive storefront, an offline editor and a streaming AI workspace pass their documented journeys outside the language team. Publish compatibility and upgrade commitments for components, browser bindings and packages. |

**First deliverable:** open the component design for review with the cart and cancellable search sources, the proposed type rules, and one old `std/ui` page that must continue to work. Use those programs to settle the grammar before expanding the widget catalog.
