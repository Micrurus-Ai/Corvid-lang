# Large uploads held on disk (limits plan row 13)

**Status:** proposal, 2026-11-27. It adds an argument to `@upload(...)`, which is written in `docs/reference/grammar.md`, the copy both sides parse, so it is posted for frontend review (item 7 of [the review brief](frontend-review-2026-11-27.md)) and built after one working day without objection.
**Owner:** backend (Claude Code).
**Roadmap:** [limits plan](removing-limits.md) row 13; [HTTP client decision 8](http-client.md#decisions), which defers sending an upload from disk until `corvid serve` keeps uploads out of memory.

## The problem

`corvid serve` reads an upload whole into memory before the handler runs, up to its declared `@upload(max_mb: N)`. A route that accepts 500 MB videos holds 500 MB per request, so ten people uploading at once need 5 GB, and the server falls over at a load a Python or Node server would handle by writing large files to disk.

Writing a person's file to disk is a privacy choice, though. A file in memory disappears with the request; a file on disk can outlive it, be read by another process, or survive a crash. So the program declares it. Silence must keep today's behavior, where nothing a person sends is written to disk.

## The design

```corvid
@upload(max_mb: 500, mime: "video/mp4", disk_over_mb: 16)
route POST "/videos" body Upload<Video> requires authenticated -> json Receipt:
    return publish(body)
```

- `disk_over_mb: N` holds a file larger than N MB in a private temporary file instead of memory. A smaller one stays in memory, as today. Without the argument, every file stays in memory. That exposes nothing new, so leaving it out is safe, and writing to disk is the choice the source makes.
- `N` must be at most `max_mb`, so the argument always does something; a larger one is a compile error that says so.
- The file is created inside a directory only the server's user can read (mode `0700`, the file `0600`). It is deleted when the request ends, whether it succeeds, fails or is cancelled, and `corvid serve` removes any left by a crash when it starts. Its name is random, never the person's filename.
- `Upload`'s methods work the same: `filename()`, `content_type()` and `size()` without reading the file, and `text()` and `bytes()` read it into memory when the program asks. A connector operation whose body is the upload (`body file`) streams it from disk, the step HTTP client decision 8 deferred.
- The trace is unchanged: an upload is recorded by name, type, size and SHA-256, never its content. The digest is computed while the file is written, so a large file is read once.
- Where the directory is, is deployment configuration, not a policy: `CORVID_SERVE_DATA_DIR/uploads` when a data directory is configured, otherwise a private directory under the system's temporary directory. A path the server cannot make private (another user owns it, or it is world-writable without the sticky bit) is a refusal to start, never a silent fallback to memory.

## Three tiers

- **Backend:** memory per request is bounded by `disk_over_mb` instead of `max_mb`, so a server takes many large uploads at once.
- **Frontend:** nothing changes on the wire. The contract may publish `disk_over_mb` beside `max_mb` for documentation; the browser uploads as before.
- **AI:** an assistant never sees file content unless an action reads it. Holding a file on disk changes nothing it can reach.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| Spill to disk past a fixed size by default | Writes people's files to disk without the source choosing it. |
| `@upload(storage: disk)` for every size | Small files gain nothing from disk and pay a write each; a threshold keeps the common small upload in memory. |
| Word it `spill_over_mb` | "Spill" is jargon; "disk over 16 MB" says what happens. |
| A server-wide setting in `corvid.toml` | A privacy choice per route belongs in the source, next to the route it concerns. |

## Slices

- [ ] U1. `disk_over_mb` parsed and checked (at most `max_mb`, only on an `Upload` route), carried in the contract beside `max_mb`.
- [ ] U2. `corvid serve` writes a file past the threshold to a private temporary file, with the digest computed on the way, deleted when the request ends, swept at start, and a refusal to start on a directory it cannot make private. Tests: a 64 MB upload under a 16 MB threshold keeps the server's memory under a bound, the file is gone after the request and after a cancelled one, and the methods read the same values from disk as from memory.
- [ ] U3. A connector `body file` sends an on-disk upload by streaming it.
- [ ] U4. Native code, with native serving.

## Non-scope

- Keeping uploads after the request (object storage, a saved file): that is a table or connector the program writes to, its own decision.
- Encrypting the temporary file: it lives only as long as the request, in a private directory; encryption at rest is the key-storage design's.
