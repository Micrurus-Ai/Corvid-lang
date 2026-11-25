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

## Evidence on the F18 working tree

| Trial | Observed result | Still required |
| --- | --- | --- |
| Storefront | A fresh CLI project built a web archive. HTTP checks found the empty catalog message and all 1,000 edition links in the large catalog. Checkout returned the empty-cart page, rejected 100 notebooks with HTTP 400 and then returned 56,925 cents for 99 of each product. | Run the responsive Edge trial. The large catalog fixture links to one notebook detail screen. Checkout has two product lines. |
| Sketchbook | The Wasm build and all eight Node tests passed. With simulated IndexedDB and HTTP, reopening restored all 6,144 cells, and synchronization uploaded the complete drawing. The suite also covered migration, conflicting writes and a lost acknowledgement. | Run the browser trial with real IndexedDB, service-worker reload and reconnect. The canvas is fixed at 96 × 64 and supports additive marks. |
| Collaborative workspace | The roadmap still lists the shared B14/F08 subscription contract as a dependency. | After F08 merges, disconnect one client, commit an edit from another, reconnect and verify the resumed subscription delivers missed and subsequent edits. |
| Independent component | No independent developer report is available for this revision. | Record the developer's own component and report from the public-docs-only trial. |

The two Edge checks stopped while importing Playwright on this Windows runner
with `PermissionError: [Errno 13]`. Run both from the repository root on hosted
Windows with the CLI built from the revision under test.

```powershell
python scripts/verify_storefront.py --cli target/debug/corvid.exe --channel msedge
python scripts/verify_drawing_editor.py --cli target/debug/corvid.exe --channel msedge
```
