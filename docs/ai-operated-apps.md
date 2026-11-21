# Applications people and AI can operate together

**Research and design, November 2026.** Let a developer express “move this meeting” once. The calendar screen, keyboard shortcut and a delegated assistant invoke the same application operation. The assistant can discover it without reading pixels. The person can see what changed, keep working, and take over at any point.

## What existing systems give us

| Primary source | What it supports | Decision for Corvid |
| --- | --- | --- |
| [Chrome WebMCP declarative forms](https://developer.chrome.com/docs/ai/webmcp/declarative-api) | A named, described form can expose its fields to browser agents. The page chooses whether to enable automatic submission. | Start with an explicit read-only GET form. Preserve its native keyboard and submission behavior, including in browsers that ignore the metadata. |
| [Chrome WebMCP imperative API](https://developer.chrome.com/docs/ai/webmcp/imperative-api) and [preview status](https://developer.chrome.com/blog/webmcp-epp) | A page can register dynamic operations for supporting browser agents. The integration depends on browser support. | Keep browser registration in an optional adapter. Register and remove actions as the active view and permissions change. |
| [MCP tool specification](https://github.com/modelcontextprotocol/modelcontextprotocol/blob/main/docs/specification/2026-07-28/server/tools.mdx) | A client can discover tools, inspect schemas and invoke operations with structured results. The tool list can depend on authorization. | Derive an optional remote adapter from the same action declaration. Give it its own authenticated identity and explicit application context. |

Corvid can already consume tools from other MCP servers. Publishing selected actions from a Corvid application is a separate implementation task. The existing typed routes, form checking, effect system and browser views are the starting points. [MCP support](reference/stdlib/mcp.md) · [Browser views](book/19-browser-views.md)

## Where Corvid can add value

**Share the user's current task.** “Move this one” should refer to the meeting selected in the visible calendar. Publish a small typed context containing the meeting ID, time zone and revision. Let the author select every exposed field. Private notes and unrelated records stay outside that context. A remote client must obtain a scoped context explicitly; switching tabs must never silently retarget an outstanding action.

**Make actions survive a redesign.** Give each published operation a stable ID, typed inputs, typed outcomes and an application-owned handler. Bind the visible control and optional protocol adapters to that handler. Bindings use the compiler's route and type information so a renamed field produces a source diagnostic. Translate button labels or rearrange the page without changing the action contract. A machine interface still exists; generating it removes the second, independently maintained implementation.

**Make collaboration visible.** Both clients observe the same application change. If the assistant proposes a move while the person edits the meeting, the UI can show the proposed change against its original revision. Agents can wait for a scoped change or cancellation instead of repeatedly polling the screen. Reuse the frontend roadmap's typed state, stream cleanup and component lifetime rules.

**Make recovery ordinary.** A move preview returns old and proposed times plus affected attendees. Applying it checks current authorization, revision and availability in one transaction. Bind retries to an idempotency key scoped to the caller and operation. Return a typed conflict when a precondition changes. Mark reversibility per operation; sending invitations can have external effects even if the calendar entry can be moved back. Record a redacted trace of who requested the change and what committed. Replay for debugging must leave external effects disabled.

## Calendar acceptance test

| Step | Expected behavior |
| --- | --- |
| Select one meeting; request open times next week | The agent sees allowed times in the selected calendar's time zone. The response omits private notes. |
| Preview a move and change the selection before applying | The preview still identifies the original meeting. Applying cannot silently move the newly selected meeting. |
| Another user takes the slot or revokes edit permission | The server returns a conflict or permission failure even if a browser or agent cached the action list. |
| Retry a successful operation | The client receives the original outcome. Attendees get at most one notification per committed move. |
| Reskin the calendar and repeat by keyboard and agent | Both paths produce the same domain change. Neither the action ID nor its input schema depends on layout or language. |

## Delivery order

1. **Discover a form.** The branch implements `ui.agent_query_form` for author-selected GET routes. It emits declarative WebMCP metadata and uses ordinary user submission. Check the live discovery and fill flow with a compatible agent/browser before advertising interoperability. [Learning guide](book/19-browser-views.md)
2. **Define a single typed action.** Prototype discovery for a typed `find_slots` handler with shared input and result schemas. Use the existing type checker and route representation. Compare an explicit opt-in on an existing handler with a separate declaration for readability. Keep the official grammar unchanged until one compiles and passes the learning tests.
3. **Share context and changes.** Implement scoped selection, stable IDs, revocation and versioned outcomes. Exercise the calendar test against a redesigned UI. Preserve normal mobile and keyboard behavior with the adapter disabled.
4. **Ship one delegated write.** Implement preview, apply, authorization recheck, conflict handling, duplicate suppression and redacted replay together. Test user and agent calls against the same server handler.
5. **Add remote operation.** Publish a scoped MCP adapter after the in-app contract is stable. Test with a real host, an expired session and a read-only identity. Document the supported protocol and browser versions.

Measure setup effort and duplicated declarations, success after a UI redesign, recovery from conflicts, keyboard completion, and correctness after permission changes. Compare agent latency and completion against a screenshot-driven baseline on the same calendar tasks before making speed or reliability claims.
