# Performance

## Storefront quantity edit

On November 23, 2026, Chrome 157 on Windows 11 completed 160 checked edits across the packaged Corvid storefront and a compiled TypeScript pricing handler. Each cell shows the median / 95th percentile in milliseconds, with 40 samples per row.

| Handler | Visit | Navigation to ready | Key to DOM update | Key to estimated frame |
| --- | --- | ---: | ---: | ---: |
| Corvid | Cold | 57.80 / 82.30 | 1.80 / 2.30 | 12.65 / 25.10 |
| TypeScript | Cold | 46.20 / 65.50 | 1.70 / 2.60 | 12.35 / 25.10 |
| Corvid | Warm | 29.95 / 36.40 | 1.30 / 1.80 | 11.10 / 22.40 |
| TypeScript | Warm | 25.95 / 35.40 | 1.30 / 2.10 | 7.90 / 19.10 |

The largest median difference was on the first visit. Corvid took 11.60 ms longer to become interactive. Chrome reported 16,700 transferred bytes for Corvid's cold browser assets and 11,438 for TypeScript's. All 40 warm Corvid visits reused the Wasm from cache. Both implementations updated the DOM in a median 1.30 ms on warm visits. Frame estimates also include browser scheduling.

Both sides used the same loopback Corvid server from a debug CLI build, generated DOM controls, HTML, CSS and checkout logic. The client used the packaged Wasm loader or TypeScript 5.9.3 compiled to JavaScript. These results cover one desktop browser and one interaction. See the [method and reproduction commands](../../benches/typescript/README.md#storefront-interaction) and [all 160 observations](../../benches/typescript/storefront-windows-chrome.jsonl).

### Startup profile

A second 160-edit run recorded each browser asset's request start and response end. Cold readiness medians were 57.3 ms for Corvid and 43.8 ms for TypeScript. Both reached the DOM update in 1.65 ms on cold visits and 1.20 ms on warm visits.

The cold JavaScript request took about 7.5 ms for either implementation. After that response ended, Corvid took a median 34.85 ms to become ready and TypeScript took 22.80 ms. Within Corvid's interval, Chrome started the Wasm request a median 21.30 ms after the JavaScript response ended. Fetching Wasm took 2.00 ms. The final interval from Wasm response end to cart readiness was 11.65 ms and includes browser scheduling, Wasm startup and control setup. Each median was computed separately. The measurements identify the serial request as a candidate for an early-fetch experiment. The experiment below measures the effect of overlapping these requests. [Startup samples](../../benches/typescript/storefront-startup-windows-chrome.jsonl)

### Early Wasm fetch experiment

Chrome 157 on Windows completed 252 checked edits across 42 rounds, with a cold and a warm visit per implementation. Adding a cart preload moved the median cold Wasm request start from 43.55 ms to 14.20 ms after navigation. Chrome recorded one Wasm resource per Corvid visit. Every preloaded cold visit transferred 846 bytes for Wasm and every warm visit reused it with zero transferred bytes.

| Cart | Cold readiness median / p95 (ms) | Warm readiness median / p95 (ms) |
| --- | ---: | ---: |
| Corvid | 56.95 / 65.90 | 29.30 / 34.90 |
| Corvid with preload | 57.15 / 62.20 | 30.40 / 34.50 |
| TypeScript | 45.75 / 56.50 | 25.25 / 28.30 |

The preload achieved overlap without improving median readiness on loopback. It remains a benchmark variant. Slower networks could give a different result. A follow-up profile can time module evaluation, Wasm instantiation and control setup independently. [Raw observations](../../benches/typescript/storefront-preload-windows-chrome.jsonl)

### Handler startup profile

In a separate 160-edit run on the same Windows Chrome setup, timestamp probes in temporary loader copies measured these median cold-visit intervals. Each implementation had 40 cold and 40 warm visits.

| Interval (ms) | Corvid | TypeScript |
| --- | ---: | ---: |
| Loader response end to first module statement | 22.25 | 22.45 |
| Module entry to beginning handler initialization | 2.00 | 2.00 |
| Begin initialization to handlers available | 13.50 | 0.90 |
| Handlers available to quantity controls bound | 2.75 | 1.40 |
| Navigation start to observed cart readiness | 68.75 | 54.40 |

Corvid's streaming Wasm startup took 13.30 ms including the fetch. A median 8.70 ms elapsed between the Wasm response ending and the instance becoming available. This elapsed interval includes browser scheduling. On warm visits, handler initialization took 6.20 ms for Corvid and 0.70 ms for TypeScript. The measurement supports testing a compiler-generated JavaScript path for small, pure integer handlers. The prototype should preserve signed 64-bit behavior, diagnostics and tracing and be compared with the existing Wasm backend using the same inputs. Repeat the profile with `--profile-stages`. [Raw samples](../../benches/typescript/storefront-stages-windows-chrome.jsonl)

### Compiled browser handlers

On Windows 11 with Chrome 157, the compiler-generated JavaScript and quantity-specific page loader eliminated the Wasm request for the two storefront calculations. Each row below uses 24 cold and 24 warm visits per CPU setting. Every visit verified the visible quantity edit and submitted checkout receipt. The four variants ran in balanced order against the same loopback backend. The TypeScript reference uses shared general DOM bindings.

| Browser handler and bindings | Desktop cold / warm ready (ms) | 4? CPU throttle cold / warm ready (ms) | Cold browser asset transfer (desktop bytes) |
| --- | ---: | ---: | ---: |
| Corvid forced to Wasm, general controls | 47.15 / 24.15 | 413.45 / 218.45 | 17,302 |
| Compiled TypeScript, general controls | 38.35 / 20.40 | 280.85 / 106.15 | 11,511 |
| Corvid generated JavaScript, general controls | 38.05 / 20.40 | 282.10 / 109.25 | 16,291 |
| Corvid generated JavaScript, quantity controls | 38.05 / 20.10 | 264.25 / 103.20 | 3,974 |

These are medians for this desktop, including an emulated CPU slowdown. At normal CPU speed, eliminating Wasm accounted for the median cold-start improvement. Narrowing the bindings reduced the transferred bytes further. Under throttling, the quantity page had a 16.60 ms lower median cold readiness time than TypeScript, while TypeScript had the lower cold p95 (305.0 ms versus Corvid's 319.9 ms). Under throttling both updated the DOM in a median 10.1 ms on the cold visit. The measurements establish the benefit over the previous Wasm path for this workload. Performance for other UI workloads and physical phones needs separate measurement. [Desktop observations](../../benches/typescript/storefront-handlers-chrome-1x.jsonl) ? [Throttled observations](../../benches/typescript/storefront-handlers-chrome-4x.jsonl) ? [Reproduction commands](../../benches/typescript/README.md#storefront-interaction)

An independent 12-round desktop run added 96 verified edits. The optimized cold median was 39.15 ms, versus 53.35 ms for forced Wasm and 38.80 ms for TypeScript. The improvement over Wasm repeated, while the two desktop runs put Corvid and TypeScript within 0.35 ms of each other in opposite orders. [Confirmation observations](../../benches/typescript/storefront-handlers-confirmation.jsonl)

### Reordering long lists

Moving a row used to copy every element in the list into a JavaScript array to find its new position for the status announcement. Generated lists now use sibling checks for the ends and a reusable DOM range for interior rows. Lists with intervening text or comments count element positions separately.

In Chrome 157 on Windows, 20 alternating rounds compared the shipped controls with commit `72d5c899`. The benchmark checked row order, focus and status text. Batch timings cover JavaScript event dispatch and synchronous DOM work. The shortcut timings cover trusted keydown to the status DOM mutation after scrolling settled, summarized as the median of each round's eight shortcuts and then across rounds.

| Rows | 100 moves before / after | One shortcut to DOM announcement before / after |
| ---: | ---: | ---: |
| 2 | 1.35 / 1.15 ms | 0.45 / 0.40 ms |
| 1,000 | 18.80 / 2.75 ms | 1.07 / 0.90 ms |

At 1,000 rows the batch ran about 6.8 times faster. The measurements exclude paint and screen-reader speech latency. Quantity edits were a control workload. [Raw observations](../../benches/results/keyed-list-windows-chrome.json) ? [Reproduce the comparison](../../benches/README.md#long-list-browser-controls)

### Durable reading-list saves

On Windows with Chrome 157, each server received three warm-up saves and 40 measured saves in alternating order. Every pair returned byte-identical receipt HTML apart from its timing comment. New server processes read back the final titles on all three shelves. Direct database checks also recovered exactly 15, 14 and 14 revisions. This verifies a process restart on the same disk.

| Interval (ms), median / p95 | Corvid preview | Node TypeScript |
| --- | ---: | ---: |
| Save keydown to visible confirmation DOM | 43.75 / 51.80 | 40.30 / 48.30 |
| Browser request start to first response byte | 14.80 / 17.30 | 12.00 / 13.40 |
| Save handler, including rendering | 14 / 16 | 11 / 12 |
| Open connection and commit one update | 10 / 13 | 9 / 10 |
| Open connection and materialize three shelves | 3 / 3 | 2 / 3 |

The application timings include runtime, connection and data conversion costs. Each stack took monotonic millisecond readings around the same operations. Both used prepared parameters, a separate read connection, SQLite DELETE journaling, FULL synchronization, foreign keys and 4 KiB pages. The release preview used SQLite 3.45.0 and Node 25 used SQLite 3.50.4. TypeScript used a static template for the generated form and suggestion controls and rendered the shelf list after each save. These are warm-process loopback timings with one active user. The browser measurement ends when the receipt elements have layout, before a confirmed display paint.

The original comparison used the two-connection reading-list example at commit `919b5b94`. [Per-save observations and configuration](../../benches/results/reading-save-windows-chrome.json)

### Reusing the save connection

The reading-list save handler now opens SQLite once, commits the update, then queries the shelves on the same request-local connection. Its SQLite durability settings are unchanged. In two Windows Chrome trials, three variants ran in all six possible orders. Each produced identical receipt HTML and recovered every expected shelf revision after restart.

| Median interval (ms), 36 rounds per variant | Corvid, two connections | Corvid, one connection | Node TypeScript, two connections |
| --- | ---: | ---: | ---: |
| Open write connection | 0 | 0 | 1 |
| Execute and commit update | 9 | 9 | 8 |
| Obtain read connection | 2 | 0 | 2 |
| Query three shelves | 1 | 1 | 0 |
| Decode rows into readings | 0 | 0 | Included in query |
| Render receipt | 1 | 1 | 0 |
| Whole handler | 14 | 12 | 11.5 |
| Save keydown to visible confirmation DOM | 43.1 | 40.6 | 41.6 |

The 24-round repeat measured median handlers of 14, 12.5 and 11 ms respectively. The consistent saving came from avoiding the second open. Browser results varied more. In the first trial the one-connection Corvid variant had a lower median but a higher p95 than the two-connection variant (52.0 versus 50.9 ms). Stage clocks resolve whole milliseconds. Both instrumented Corvid variants keep the first handle in scope through rendering. Node closes its connections explicitly, and the individual stage columns exclude its close time. [36-round profile](../../benches/results/reading-save-stages-windows-chrome.json) | [24-round repeat](../../benches/results/reading-save-stages-repeat-windows-chrome.json)

A separate 40-round run timed the updated example handler and the existing TypeScript implementation. Median / p95 time from Save to visible confirmation was **42.75 / 54.90 ms** for Corvid and **42.95 / 55.00 ms** for TypeScript. The median handler took 14 ms for Corvid and 13 ms for TypeScript. The browser results were effectively tied on this machine. Both servers recovered revisions 15, 14 and 14 after restart. [40-round application results](../../benches/results/reading-save-reused-windows-chrome.json)

Run `python scripts/benchmark_reading_save.py PATH_TO_CORVID --output saves.json` with Python Playwright, Chrome and Node 25. Add `--profile --rounds 36` to compare the individual stages across all three variants.

### Saving while an agent is working

A reader presses Save in Chrome while zero, nine or 49 other clients submit edits. A second version of each burst starts a shelf suggestion first. The local model fixture receives a real HTTP request, waits 500 ms and returns the same suggestion to either stack. The runner verifies that the model was still waiting at the browser keydown and that its answer eventually reached the application. Corvid executes the packaged suggestion agent. The Node baseline reads the shelf, calls the fixture asynchronously and returns the proposed title.

The browser edits shelf 1. Its trusted Enter key releases the HTTP clients, which split their edits between shelves 2 and 3. The browser clock stops when the saved title and confirmation both have layout. Each stack uses one connection per save, prepared parameters, the same initial data and SQLite FULL synchronization. A test-only SQLite trigger records each unique edit within its update transaction so overwrites can be checked individually. After restarting both applications, the runner reads the shelves through HTTP and compares titles, revision counts and the audit log with successful responses.

Windows Chrome 157 ran 20 measured bursts per cell using Corvid preview `8a0c4161` and Node 25. All 2,562 edits per stack, including warm-up, received confirmations and were recovered exactly once. The model request was still pending at Save in every agent trial. The single Corvid editor saw the saved title before the provider answered in all 20 measured trials.

| Simultaneous editors | Corvid, no agent | Corvid, pending agent | TypeScript, no agent | TypeScript, pending agent |
| ---: | ---: | ---: | ---: | ---: |
| 1 | 37.0 / 40.9 | 36.8 / 43.4 | 30.1 / 38.0 | 34.0 / 42.9 |
| 10 | 122.0 / 184.1 | 49.0 / 205.6 | 64.0 / 78.5 | 64.7 / 77.5 |
| 50 | 253.4 / 525.0 | 173.1 / 509.2 | 167.8 / 298.7 | 67.2 / 289.2 |

Each cell gives median / p95 from keydown to confirmation in milliseconds. These are bursts on one Windows host with one real browser and HTTP peers. The order of arrival within each burst varies. With only 20 browser samples per cell, the p95 is the second-largest observation. The figures do not predict sustained throughput, hosted-model latency or physical display paint. [Per-request observations and SQLite versions](../../benches/results/reading-concurrent-saves.json)

When 49 peers also saved and the model was pending, their HTTP responses took 726 ms at p95 on Corvid and 302 ms on Node. Time inside the instrumented save handler reached 690 ms at p95 in Corvid and 7 ms in Node. Node executes its synchronous SQLite handlers serially, so requests can queue before the handler clock starts. Both browser and HTTP intervals include that wait.

In the ten-round control with 49 peers loading edit forms and the model pending, Corvid's browser-save median / p95 fell to 51.4 / 66.0 ms and TypeScript's to 50.5 / 71.7 ms. This isolates competing writes as a contributor to the long tail. The run does not distinguish SQLite lock waits from scheduling inside the Corvid handler. Both stacks recovered all 66 browser saves including warm-up. One Corvid peer loading an edit form lost its connection. The preview server logged a worker panic at `crates/corvid-vm/src/value/heap.rs:82` with `shadow count underflow`. Its process continued serving. [Read-burst observations](../../benches/results/reading-concurrent-reads.json)

The patched VM tracks map, choice and closure handles on clone and drop, and only inspects the ownership graph when workers have released their heap access guards. A repeat of the ten-round read control on a local build completed all 1,276 Corvid peer reads and all 66 browser saves, including warm-up. The restarted app recovered all 66 edits. The earlier panic was intermittent, so this trial establishes the observed result on one Windows machine. [Patched read trial](../../benches/results/reading-collector-fix.json)

Run the following commands with Python Playwright, Chrome, Node and the Corvid CLI. The second run replaces competing saves with requests for edit forms. Each case has a warm-up burst. The reported medians and nearest-rank p95s exclude warm-up.

```sh
python scripts/benchmark_reading_concurrency.py PATH_TO_CORVID --rounds 20 --output saves.json
python scripts/benchmark_reading_concurrency.py PATH_TO_CORVID --read-only-peers --rounds 10 --output reads.json
```

### SQLite writer acquisition

A ten-round diagnostic on the patched VM used the same concurrent-save benchmark. In the temporary Corvid app, each save explicitly acquired the writer with `BEGIN IMMEDIATE`, updated the shelf, and issued `COMMIT`. Both stacks used FULL synchronization and recovered all 1,342 acknowledged edits each after restart, including warm-up. No request failed.

| Corvid writers with a pending suggestion | Saves measured | Begin median / p95 (ms) | Update median / p95 (ms) | Commit median / p95 (ms) | Handler median / p95 (ms) |
| ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | 10 | 0 / 1 | 2 / 3 | 3 / 4 | 7.5 / 10 |
| 10 | 100 | 56 / 142 | 2 / 3 | 3 / 5 | 67.5 / 153 |
| 50 | 500 | 13.5 / 587 | 2 / 3 | 3 / 18 | 28 / 595 |

For the 25 slowest handlers with 50 writers and a pending suggestion, 16,926 of their combined 17,229 ms elapsed inside `BEGIN IMMEDIATE` (98.2%). Without a suggestion, the same interval accounted for 98.4% of the slowest 25 handlers. These clocks include SQLite lock acquisition, tool dispatch and thread scheduling. The explicit transaction also adds tool calls relative to the shipped implicit transaction. Results pool the browser and peer requests, exclude warm-up and measure only time inside the handler. The next experiment is to queue competing writers per database before they enter SQLite and measure the full browser confirmation and both read and write tail latencies. [Per-save timings](../../benches/results/reading-writer-acquisition.json)

Run `python scripts/benchmark_reading_concurrency.py PATH_TO_CORVID --rounds 10 --profile-writer --output writer.json`.

### Queueing SQLite writers

The preview now queues `db_execute` calls on autocommit connections that open the same SQLite file in one process. A waiting task yields its worker. Read queries run independently, and a connection already in a transaction can continue through commit or rollback. SQLite still coordinates external writers. This requires no change to application code.

Two Windows Chrome trials compared local Corvid builds before and after the queue using the same packaged app. Each cell had 20 measured bursts, with one browser editor and the other clients making HTTP requests. Each pair alternated which build went first. SQLite used DELETE journaling and FULL synchronization. With a shelf suggestion pending at Save, the 50-client results were:

| Operation | Before median / p95 (ms) | Queued median / p95 (ms) |
| --- | ---: | ---: |
| Browser Save to visible confirmation, 50 writers | 48.8 / 646.0 | 88.2 / 420.8 |
| Other writers' HTTP responses, 50 writers | 320.2 / 811.5 | 232.9 / 437.4 |
| Browser Save to visible confirmation, 25 writers and 25 readers | 134.7 / 428.4 | 46.6 / 237.9 |
| Edit-form HTTP reads, 25 writers and 25 readers | 91.4 / 248.4 | 53.7 / 149.2 |

The queue reduced the slowest save and read times under contention. In the all-writer trial, the foreground browser's median rose with and without a pending suggestion. Arrival order varies within a burst, and 20 browser observations give a coarse tail estimate. Without a model call the 50-writer browser p95 fell from 526.6 to 424.5 ms. Every request succeeded, and each build recovered all 2,562 write-trial edits and all 1,302 mixed-trial edits after restart, including warm-up. [Simultaneous writes](../../benches/results/reading-queue-writes.json) and [mixed reads and writes](../../benches/results/reading-queue-mixed.json)

Run `python scripts/benchmark_reading_concurrency.py PATH_TO_CANDIDATE --compare-cli PATH_TO_BASELINE --rounds 20 --output writes.json`. Add `--mixed-peers` to alternate edit-form reads with saves.

### Where a browser save finishes in a write burst

The earlier 20-round run found a higher browser median with the queue when all 50 clients saved. A 40-round repeat on the same Windows machine with the same two binaries found a lower median with the queue, both with and without the pending suggestion. Slow-save latency improved in both runs. Absolute times varied across runs.

| 50 simultaneous saves, 40 measured bursts per cell | Before median / p95 (ms) | Queued median / p95 (ms) |
| --- | ---: | ---: |
| Browser confirmation, no suggestion | 429.5 / 974.8 | 237.1 / 578.4 |
| Browser confirmation, suggestion pending | 367.3 / 841.7 | 272.6 / 620.1 |
| Other writers' HTTP responses, suggestion pending | 389.5 / 957.4 | 296.9 / 597.4 |

After restart, the runner uses the audit row order to find the browser save's position among successful writes in its burst. With the queue and a pending suggestion, the browser committed among the first ten in 19 bursts and the last ten in 12 bursts. Median confirmation was 77 ms and 583 ms in those groups, respectively. Baseline medians for those groups were 61 ms and 778 ms. All 4,100 edits per build were confirmed and recovered, including warm-up. The original run did not record commit positions, so its median difference cannot be attributed to a specific arrival order. [40-round observations](../../benches/results/reading-queue-commit-order.json)

In a separate 20-round profile, each server also recorded the time just before each handler called the update helper. Equal timestamps give a range of possible positions. Across the 40 measured bursts (with and without a suggestion), the queued browser's commit position was within two places of that range **40 times**, compared with **8 times** on the baseline. All 2,100 edits per build survived restart. This supports keeping the queue's existing ordering. The pre-update clock measures application progress, and the queue itself is reached a little later. [Write-order profile](../../benches/results/reading-queue-write-order.json)

Run `python scripts/benchmark_reading_concurrency.py PATH_TO_CANDIDATE --compare-cli PATH_TO_BASELINE --editors 50 --rounds 40 --output order.json`. Add `--profile-order` to record the process-local millisecond timestamps at handler entry and just before calling the update helper. Compare those timestamps only within the same burst and server process. Timestamps that share a millisecond have an uncertain order.

### Sustained editing and backlog

On Windows Chrome, a Node client scheduled saves at a fixed rate and 20 edit-form reads per second regardless of earlier response times. Chrome on the same machine attempted to open the edit form every two seconds, one navigation at a time. A slow navigation reduced the number of browser probes completed during the window. Both Corvid builds served separate copies of the same packaged app and initial SQLite database. A trigger logged each edit in its save transaction. The two 12-second trials reversed build order and load order.

| Offered saves/s | Build | Outstanding requests at cutoff, trials 1 / 2 | Time to finish after cutoff, seconds | Successful HTTP save / read p95, ms | Failed saves / reads |
| ---: | --- | ---: | ---: | ---: | ---: |
| 20 | Before queue | 0 / 0 | 0 / 0 | 21 / 10 | 0 / 0 |
| 20 | Queued | 0 / 0 | 0 / 0 | 25 / 14 | 0 / 0 |
| 80 | Before queue | 22 / 257 | 0.50 / 4.29 | 4,348 / 3,922 | 2 / 0 |
| 80 | Queued | 1 / 329 | 0.002 / 5.57 | 5,066 / 347 | 0 / 0 |
| 160 | Before queue | 1,071 / 1,110 | 12.72 / 14.65 | 17,245 / 13,554 | 319 / 59 |
| 160 | Queued | 603 / 762 | 5.48 / 8.44 | 7,619 / 66 | 0 / 0 |

Outstanding requests are measured from HTTP dispatch to response completion, including failures. That count includes work in the client, transport and server. Drain time starts at the end of the offered-load window. The client recorded scheduled and actual dispatch times separately. The p95 dispatch lag ranged from 8 to 14 ms across the pooled rate/build cells. At the highest rate, the baseline recorded 374 refused connections and four HTTP 200 responses without a save confirmation. Across the complete short trial each build recovered exactly its acknowledged writes, including warm-up (6,260 with the queue and 5,939 before it). [Request timings, browser visits and restart counts](../../benches/results/reading-sustained-windows-chrome.json)

At 160 saves/s the browser saw the edit form within 113 ms at p95 with the queue (12 visits). On the baseline it took 18.2 seconds at p95, with only seven visits because slow visits overlapped later scheduled probes. These small samples describe this machine. At the same offered rate the queue still took seconds to drain, and the 80-save/s windows varied substantially between trials.

A separate 60-second run offered 80 saves and 20 HTTP reads per second. On the queued build, all 4,800 measured saves and 1,200 reads succeeded. At cutoff 149 requests were outstanding and the last response arrived 1.82 seconds later. Save p95 was 1.76 seconds, HTTP read p95 was 102 ms, and Chrome observed the form within 163 ms at p95 across 30 visits. The baseline had 2,283 requests outstanding at cutoff and took 26.81 seconds for all client requests to finish or fail. It had 1,064 failed saves and 184 failed reads. Both builds recovered every acknowledged edit. The baseline database additionally contained 56 edits for which the client never received confirmation. A client timeout therefore did not guarantee that its edit had been rolled back. The first attempt at this longer trial stopped on a baseline browser navigation timeout. The completed repeat recorded browser failures independently and ran the baseline first. [One-minute observations](../../benches/results/reading-sustained-minute.json)

Run `python scripts/benchmark_reading_arrivals.py PATH_TO_QUEUED_CLI --baseline PATH_TO_PREVIOUS_CLI --output sustained.json`. Use `--rates 80 --seconds 60 --rounds 1` for the one-minute workload. The benchmark includes warm-up edits in restart verification and excludes them from latency summaries.

### Choosing a Postgres pool size

These measurements show what a `connections:` value buys. Agents run concurrently against the pool of a Postgres table database. Each one edits as the runtime does, with a `get` then a revision-checked `update`. That is two statements, each borrowing its own connection. Agents edit disjoint rows, so no edit conflicts and the numbers measure the pool and the database. Each cell ran 3 seconds on a 4-core WSL Ubuntu host with a local PostgreSQL 16, in a release build.

| Agents | Connections | Edits/s | Wait at the pool p50 / p95 / p99, ms | Edit p50 / p95, ms |
| ---: | ---: | ---: | ---: | ---: |
| 8 | 1 | 482 | 7.03 / 12.57 / 19.22 | 15.64 / 23.25 |
| 8 | 2 | 754 | 3.88 / 5.60 / 7.01 | 10.39 / 12.48 |
| 8 | 4 | 1,334 | 1.37 / 2.71 / 3.50 | 5.81 / 7.61 |
| 8 | 8 | 2,481 | 0.00 / 0.00 / 0.00 | 2.95 / 4.68 |
| 64 | 1 | 473 | 64.05 / 108.20 / 146.20 | 129.25 / 184.66 |
| 64 | 2 | 631 | 46.61 / 91.04 / 128.42 | 91.72 / 153.05 |
| 64 | 4 | 1,263 | 22.91 / 35.24 / 56.21 | 47.89 / 62.60 |
| 64 | 8 | 2,383 | 11.56 / 14.90 / 18.64 | 26.55 / 30.66 |
| 64 | 16 | 3,687 | 6.27 / 9.09 / 13.93 | 16.68 / 21.52 |
| 64 | 32 | 4,882 | 3.08 / 5.09 / 7.39 | 12.48 / 17.86 |

Edits per second grow almost in step with connections until there is one connection per busy agent. Beyond that point the pool is no longer what limits throughput. With 8 agents and 8 connections, no agent waited at all. Below that point, most of an edit's time is spent waiting for a connection rather than in the database. With 64 agents on 8 connections, an edit took 26.6 ms at the median, and 2 × 11.6 ms of that was waiting for a turn. A full pool queues work rather than failing: no cell had an error.

To size a pool, take the number of operations a program has in flight at its busiest and choose the smaller of that and the cap your database plan allows. Leave room for other clients of the same database, including migrations. These are local numbers. A database across a network holds each connection longer per statement, which makes each connection worth fewer edits and the queue longer. Measure against your own database the same way:

```sh
CORVID_TEST_POSTGRES_URL=postgres://... CORVID_POOL_LOAD_SECONDS=10 \
  cargo test --release -p corvid-runtime --lib pool_under_concurrent_agents -- --ignored --nocapture
```

The test creates and drops a table named `corvid_pool_load`, so point it at a scratch database. It measures the pool directly. A served program on Postgres under sustained load, as in [Sustained editing and backlog](#sustained-editing-and-backlog), is still to be measured.

### Cancellable AI stream

On Windows with Chrome 157, Corvid preview 8a0c4161 and Node 25 TypeScript each served 40 measured streaming requests in alternating order. Both served identical HTML and browser code and sent identical JSON requests to the same local model fixture. The fixture waited 30 ms before its first fragment and 10 ms between subsequent fragments. Every measured cancellation closed the upstream connection and ended the fixture's generation loop before its deadline.

| Interval (ms), median / p95 | Corvid preview | Node TypeScript |
| --- | ---: | ---: |
| Start click to first text in the visible output | 51.10 / 66.10 | 50.95 / 68.10 |
| Cancel click to cancelled DOM status | 0.35 / 0.50 | 0.30 / 0.50 |
| Cancel click to fixture stopping generation | 12.52 / 13.23 | 12.66 / 13.75 |

First text and status timings use the browser clock. The upstream stop measurement uses a local monotonic clock aligned with the browser before each request using seven round trips. Median half-round-trip uncertainty was about 0.5 ms per stack. The 0.14 ms difference between stop medians is within that uncertainty. These measurements cover framework and transport behavior with a controlled token schedule. A hosted model adds its own inference, network and cancellation behavior.

Run `python scripts/benchmark_text_stream.py PATH_TO_CORVID --output stream.json` with Python Playwright, Chrome and Node 25 available. [Per-stream observations](../../benches/results/text-stream-windows-chrome.json)

## What's slow, what's fast

Corvid's optimization target is the AI-orchestration surface, not raw
throughput. Honest numbers (Phase 17 baselines):

- **Orchestration overhead** (no model call): ~25–36× slower than
  Python LangChain on equivalent code paths. ~1.7–2.6× slower than
  TypeScript on `refund_bot`, `rag_qa_bot`, `support_escalation_bot`.
- **Compile-time guarantee surface**: zero runtime cost. `approve`,
  `Grounded<T>`, and effect rows are typecheck-time constructs.
- **Replay**: byte-identical re-execution at memory speed (no
  network).
- **Codegen**: Cranelift backend; comparable to debug-mode rustc for
  agent-shaped programs.

The "honest slower" framing is deliberate — Corvid's value is the
correctness surface, not raw orchestration speed. See the
[benchmarks](https://corvid-lang.org/benchmarks) page for full archives.

## Corvid, CPython, Node.js and Rust on the same workload

One Windows x64 session compared fresh command-line processes on two inputs. Each cell is Corvid's median elapsed time divided by the other implementation's median, with a 95% paired bootstrap interval. A ratio of 0.50 means the Corvid command took half as long. Both native programs were compiled before measurement.

| Input | Corvid / CPython | Corvid / Node.js | Corvid / Rust |
|---|---:|---:|---:|
| Zero records | 0.36 (0.34–0.38) | 0.20 (0.20–0.22) | 1.08 (0.99–1.18) |
| Score one million records | 0.12 (0.12–0.13) | 0.28 (0.27–0.30) | 1.47 (1.38–1.53) |

For the million-record batch, native Corvid finished sooner than CPython and Node.js and took 47% longer than optimized Rust. Startup accounts for much of the elapsed time, particularly for Node.js. The scoring loop uses integer arithmetic with deterministic inputs and makes zero model or network calls. See the [implementations and reproduction command](../../benches/process_latency/README.md) and the [raw samples and tool versions](../../benches/results/process-latency-windows-x64/session.json).

## When to optimize

In order:

1. The model. Most of the latency and cost is the LLM call. Provider
   routing (`requires` clauses) and prompt tightening dominate.
2. The retrieval. RAG quality > LLM quality > orchestration speed.
3. The agent shape. Agents that loop over `n` calls cost `n` × per-
   call cost; loop bounds and budget annotations are cheap to add.
4. The orchestration. If after the above three you're still
   bottlenecked on Corvid's orchestration overhead, drop the inner
   loop into Rust via the FFI.

## Budgets are real

`@budget($X)` is a compile-time ceiling. The compiler computes the
worst-case composed cost; the binary refuses to ship if it exceeds X.
Use this to keep production agents from drifting upward in cost as
they accrete features.

## Concurrent jobs

The durable runner (`corvid jobs run --source <path>.cor --workers=N`) scales to N async
workers per process. Past a few thousand jobs/second per host, scale
horizontally — multiple runner processes against one queue.

### Windows x64 release baseline

The [candidate run](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/44866623552) measured the same release binary twice on one runner. Each column gives the median in milliseconds from 15 samples after three warmups. The HTTP workloads use a loopback provider and average 16 sequential requests per sample.

| Workload | First run (ms) | Repeat (ms) |
|---|---:|---:|
| CLI startup | 17.482 | 16.465 |
| Check 100 functions | 19.795 | 18.217 |
| Native build of 100 functions | 462.281 | 457.090 |
| Interpreter list slices | 25.713 | 25.502 |
| HTTP ping | 13.860 | 13.784 |
| Direct provider stream | 3.636 | 3.806 |
| Corvid stream | 10.248 | 12.005 |

The paired median overhead for a 16-chunk stream was 6.533 ms and 8.199 ms. The [first report](../../benches/results/application-windows-release/baseline.json) and [repeat report](../../benches/results/application-windows-release/repeat.json) include the raw samples and CLI hash.

On [candidate 05c20e08](https://github.com/Micrurus-Ai/Corvid-lang/actions/runs/44883568390), both profiles used the same Windows x64 executable. Median startup took 14.732 ms and 14.712 ms. The paired stream overhead was 6.138 ms and 6.431 ms. All seven repeat medians fell within the benchmark's same-runner thresholds. With performance verification enabled, the candidate workflow now checks the repeat against its first profile automatically and retains the measurements for inspection if the comparison fails. This comparison measures repeatability for one build.

## Reproducible benchmarks

```sh
corvid bench src/main.cor
corvid bench compare python --source src/main.cor target/python_traces
```

Published baselines live in `benches/` and ship via Phase 22's
reproducibility scripts.
