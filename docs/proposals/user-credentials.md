# Calling a service as the signed-in user (limits plan row 7)

**Status:** proposal, 2026-11-26. C1 is connector grammar, which only backend code writes ([HTTP client decision 1](http-client.md#decisions)), so it is built now. C2 adds an item to the `identity` block and is posted for frontend review under the propose-then-build law.
**Owner:** backend (Claude Code).
**Roadmap:** [limits plan](removing-limits.md) row 7; [HTTP client decision 4](http-client.md#decisions).

## The problem

A connector has one credential, the application's own: `auth: bearer(secret("GITHUB_TOKEN"))`. Many real applications call a service as the person using them: list *my* repositories, read *my* calendar, file an issue *as me*. In Python or TypeScript that is a token from the OAuth sign-in, kept somewhere and passed to the client on each call. In Corvid today it cannot be done:
- a connector operation's signature holds no `Secret`, by B04's rule;
- a `header` line cannot carry one, because a `Secret` cannot be joined with text;
- sign-in reads the provider's access token once, to fetch the user's profile, and then drops it.

## The design, in a realistic program

```corvid
identity users:
    provider github:
        keep_token: scopes("repo")
    provisioning:
        first_login: open
        tenant: fixed("public")
    roles:
        member: "issues.file"

connector github_api:
    base_url: "https://api.github.com"
    auth: bearer(secret("GITHUB_APP_TOKEN"))
    timeout: 10s
    redirects: never
    modes: [mock, replay, real]

    operation create_issue(token: Secret, repo: String, issue: NewIssue) -> Result<Issue, IssueError>:
        auth: bearer(token)
        POST "/repos/{repo}/issues" body issue
        on status 401 -> SignInAgain
        on failure -> Unavailable

type IssueError:
    SignInAgain
    Unavailable(reason: String)

action file_issue(repo: String, title: String, by: Caller) -> Result<Issue, IssueError> requires permission("issues.file"):
    assistants: confirm "File an issue in {repo}?"
    return match by.token("github"):
        Some(token) -> github_api.create_issue(token, repo, NewIssue(title))
        None -> Err(SignInAgain)
```

### C1. An operation's own credential

- `auth: bearer(token)` on an operation names one of its parameters. The parameter must be a `Secret`, and that is the only place an operation may hold a `Secret`; the B04 rule stays for every other parameter. The checker names the parameter and the rule when either is broken.
- The operation's `auth:` replaces the connector's for that call. A connector with no `auth:` of its own may still declare operations that have one.
- The token is sent as `Authorization: Bearer <token>` and reaches nothing else. The trace records the argument as `<redacted>`, as every `Secret` argument already is. Replay matches the call without the token, so a recorded run replays without anyone's credential.
- The existing credential rules hold for it unchanged: a redirect to another origin never carries it, and it is never in an `HttpExchange` record.
- It is refused on an `async:` operation. A protocol's arguments are persisted so it can be polled later, and a person's token must never be stored that way.
- `basic(...)` and `header(...)` take parameters the same way when an example needs them; C1 ships `bearer`, the form these tokens use.

### C2. Keeping the user's provider token

- `keep_token: scopes("repo", ...)` on a provider asks for those scopes at sign-in and keeps the access token the provider returns. Without the item, sign-in keeps nothing, as today. Not keeping a token exposes nothing, so leaving the item out is safe, and the program never holds a token it did not ask for. Keeping one is the consequential choice, and it is written in source.
- `by.token("github")`, on a `Caller`, is an `Option<Secret>`. The checker refuses a provider that is not declared with `keep_token:`, naming the item to add. The value is `None` when the token has expired, the person signed in before the item existed, or the session was started by another means, so the program decides what to show. It never fails at run time.
- **Narrowed for now:** the token is held only in memory, beside the session that signed in. A restart, or a request served by another instance, sees `None` until the person signs in again. Keeping tokens across restarts needs encryption at rest, which waits on the key-storage design (Ed25519 and encryption, in the plan's planned work). Refresh tokens wait on the same design. The reference page says so, and the roadmap holds the widening.
- A delegated assistant (B11) reaches the token only through an action the source declares with `assistants:`, and the trace marks each call made with a person's token, so people see which calls an agent made as them.

## Three tiers

- **Backend:** the compiler knows which operations act as a person, and the contract can say so per action.
- **Frontend:** an action whose result is `Err(SignInAgain)` gives the UI a typed reason to offer "Sign in with GitHub again", with the scopes from the declaration, instead of a generic error.
- **AI:** an assistant acting for a person uses that person's token only through declared actions, with the confirmation and trace those already have. Nothing about the token is exposed to it: it never sees the value, only the result.

## Alternatives considered

| Alternative | Why not |
| --- | --- |
| A `Secret` `header` line | Decided against in B05 slice 3b: it cannot build `Bearer <token>`, and `Authorization` belongs to `auth:`. |
| A connector-level `auth: bearer(caller)` | Hides where the token comes from and ties a connector to one identity provider. A parameter keeps the source explicit and lets a program pass a token from anywhere. |
| Keeping tokens by default | Would silently store every person's credential. The item makes it a declared choice. |
| `by.token(github)` with a bare name | A new kind of name for one call. A string the checker verifies against the `identity` block reads as plainly and catches the same typo. |
| Storing tokens in a table now | No encrypted column exists, and a plain one would put credentials at rest. Refused until encryption lands. |

## Slices

- [ ] C1. Operation `auth: bearer(param)` on the interpreter: parser, checker (a `Secret` parameter, only in `auth:`, refused on `async:`), the runtime sending the revealed token, `<redacted>` recording and replay without the token, and tests for each refusal, for a cross-origin redirect, and that the token reaches no trace or log.
- [ ] C2. `keep_token: scopes(...)` and `Caller.token("provider")`, with the token held in memory beside its session, after frontend review.
- [ ] C3. Durable, encrypted token storage and refresh, after the key-storage design.
- [ ] C4. Native code, with the native connector dispatch.

## Non-scope

- Acting as a person with a service that is not their sign-in provider (an API key the person pastes). C1 already takes a token from anywhere; storing pasted keys needs the same encryption as C3.
- Token exchange and on-behalf-of flows between services.
