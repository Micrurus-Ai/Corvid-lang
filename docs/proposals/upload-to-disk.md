# Large uploads held on disk (limits plan row 13)

**Status:** U1 and U2 built 2026-11-27, together, after frontend review (item 7 of [the review brief](frontend-review-2026-11-27.md)). Frontend development agreed, adding that the threshold is server storage policy and must not become a second client-side size limit. A `disk_over_mb` the server ignored would be a declared choice that does not run, so U1 and U2 shipped as one.
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

### Decided when building (2026-11-27)

- **Not published.** The design allowed publishing `disk_over_mb` in the contract. The frontend answer asked that it never become a client-side limit, so the contract does not carry it at all, and a test holds that.
- **Below the maximum.** `N` must be below `max_mb`, not at most: a threshold equal to the maximum would never hold a file on disk.
- **Deleted with its last reference.** A held file belongs to a VM value (`Value::HeldUpload`) and is deleted when the last value holding it drops. For a request, that is when it ends. A request queued for approval keeps its arguments, as it always has, so its file lives until the approval is decided.
- **Each server's own directory.** Each server holds a lock on its own directory under the upload base. The startup sweep removes only directories whose lock is free, so two servers sharing a data directory never delete each other's files.
- **Private, or refused.** On Unix the base and the server's directory are `0700` and must belong to the server's user (found from the owner of a file it creates, since std has no call for it). A symbolic link, another user's directory, or a file in the way refuses to start. On Windows the server cannot yet check a directory's ACL, so `disk_over_mb` refuses to start there, and the roadmap holds the check.
- **A field's upload** inside a multipart record refuses `disk_over_mb` by name; only a route's direct `body Upload<Format>` is held on disk for now.
- **Writes off the request threads.** Each chunk is written on Tokio's blocking pool. The digest the trace records is computed as the file is written, and every reader of an upload (`bytes()`, `text()`, the trace digest, a connector body) goes through `corvid_vm::upload`.

## Slices

- [x] U1. `disk_over_mb` parsed and checked (below `max_mb`, only on a route's `Upload`). It is kept out of the contract (decided above). Built 2026-11-27.
- [x] U2. `corvid serve` writes a file past the threshold to a private temporary file, with the digest computed on the way, deleted when the request ends, swept at start, and a refusal to start on a directory it cannot make private. Built 2026-11-27. Tests (`serve_uploads_on_disk.rs`, Linux): a 64 MB upload under a 16 MB threshold grows the server's peak memory by less than 40 MB; the file is gone after the request, after a refused one and after a cancelled one; the directory is `0700`; and `bytes()`, `text()` and the digest read the same values from disk as from memory. `held_upload.rs` unit tests cover the file modes, deletion, and the sweep keeping a running server's directory.
- [ ] U3. A connector `body file` sends an on-disk upload by streaming it.
- [ ] U4. Native code, with native serving.

## Non-scope

- Keeping uploads after the request (object storage, a saved file): that is a table or connector the program writes to, its own decision.
- Encrypting the temporary file: it lives only as long as the request, in a private directory; encryption at rest is the key-storage design's.
