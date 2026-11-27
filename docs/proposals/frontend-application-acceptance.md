# F18 application trials

Use the existing storefront and sketchbook as ordinary project inputs. A shopper
who enters zero for both products should see an empty-cart message and a link
back to the cart. The existing quantity refinements also define the largest
valid checkout. Exercise both bounds and recovery from a rejected submission
against the extracted web package. Pass empty and 1,000-entry lists through
the same catalog function the storefront uses. Keep prices and validation in
the project.

Start the sketchbook with an empty shared drawing. Load a nearly full 96 by 64
canvas, finish it offline, reopen it and synchronize it. Keep the existing
two-tab migration and conflict exercise in the same browser run. A temporary
HTTP fixture supplies conditional reads and writes for this editor.

## Independent component trial

An independent developer can start now. Use the [browser views learning guide](../book/19-browser-views.md), the public documentation and the standard CLI.

1. Choose a new component that you want to use. Pick its domain type and operation yourself.
2. Build it in a fresh project. Decide which state and actions an assistant may access.
3. Share your report on [the acceptance pull request](https://github.com/Micrurus-Ai/Corvid-lang/pull/44). Include the component source, CLI version, documentation pages used, commands and results, time to the first useful result, remaining blockers and any maintainer help. If you get stuck, include the last command and its output.

## Acceptance at `5299b6fd33cc4d820a21284971f6a86583038fde`

Both application checks passed on hosted Windows with Edge. F18 remains open.
[Hosted run](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45624130574)

| Trial | Verified behavior | Scope of the trial |
| --- | --- | --- |
| Storefront | The packaged app served empty and 1,000-entry catalogs. Checkout showed the empty cart, rejected 100 notebooks with HTTP 400, then accepted 99 of each product for 56,925 cents. Edge exercised checkout at 320 and 960 CSS pixels with scripting enabled and disabled. | The large catalog links to one notebook detail screen. Checkout has two product lines. Touch checkout used Pixel 7 emulation. |
| Sketchbook | Edge opened an empty drawing, loaded 6,143 marked cells and saved the final cell offline. A service-worker reload restored all 6,144 marks and reconnect uploaded the same cells. The two-tab migration and conflict checks also passed. | The 96 by 64 canvas supports additive marks. Synchronization uses conditional HTTP requests against the trial server. |

The hosted workflow ran these commands from the repository root.

```powershell
cargo build --locked -p corvid-cli --bin corvid
$cli = (Resolve-Path target/debug/corvid.exe).Path
python -B scripts/verify_drawing_editor.py --cli $cli --channel msedge
python -B scripts/verify_storefront.py --cli $cli --channel msedge
```

## Collaborative workspace

The [board project](../../sdk/typescript/client/test/boards.cor) serves a Corvid page that subscribes to its cards. The [hosted Edge run](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/45815905898) passed on `0c080ee80f4769e190861677854f914a44319807`.

- The typed client received an empty snapshot for one board. A separate HTTP client then created 1,000 cards through the authenticated write route. Opening the board rendered every title in card order.
- On another board, the browser went offline. The HTTP client committed an edit while the browser was disconnected and another after it reconnected. Both the Corvid component and the generated TypeScript observer received both edits. The unaffected row kept its identity, input text, focus and selection.
- Removing the component unsubscribed it. The remaining observer received the next edit. Both clients used the same authenticated test identity against one SQLite-backed server.

Reproduce the [workspace check](../../scripts/verify_generated_live_client.py) from the repository root with the CLI built above.

```powershell
python -B scripts/verify_generated_live_client.py --cli $cli --channel msedge
```

## Remaining acceptance criteria

F18 stays open until F08 merges and an independent developer completes the public-docs-only component trial above and submits the outcome. The three automated application trials have passed.
