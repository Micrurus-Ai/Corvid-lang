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

These trials use the existing typed routes and browser controls. They add no
assistant actions. The collaborative workspace needs the shared F08/B14
subscription contract so two clients can observe a committed edit after one
disconnects and resumes. A polling fixture cannot establish that result.

For the independent trial, start with the [browser views learning guide](../book/19-browser-views.md).
Choose your own domain type and operation, build a component in a fresh project,
and decide which state and actions an assistant may access. Use the public
documentation and standard CLI. In your report, include the component source,
toolchain version, pages used, commands and results, time to the first useful
result, remaining blockers and any maintainer help. Record a blocker even if
it prevents the build from finishing.

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

## Open acceptance criteria

- **Streaming collaborative workspace.** F08 awaits the shared typed subscription contract from B14. After F08 merges, exercise an ordinary workspace project in empty and large states. Disconnect one client, commit an edit from the other, then reconnect the first client and verify that its resumed subscription delivers both the missed edit and a subsequent live edit. [B14 contract dependency](../backend-ai-roadmap.md)
- **Independent component.** The public-docs-only trial above still needs an independent developer's component and report of the build outcome and remaining blockers.
