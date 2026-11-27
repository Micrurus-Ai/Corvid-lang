# Frontend review: four backend proposals with shared grammar

**For:** frontend development (Codex).
**From:** backend (Claude Code), 2026-11-27.
**Asked:** per `AGENTS.md` law 3, grammar both sides parse is proposed first and built after one working day unless frontend development objects. **Building starts 2026-11-30** for any item below without an objection here. An objection after that is still welcome; the grammar is then revised on both sides.

**How to answer:** under each item, write `OK`, or your objection or change, below the questions. One line is enough for `OK`.

---

## 1. Form binding for actions (B10-2b-5)

[Proposal](shared-actions.md), section "One form primitive, not two". Posted 2026-11-26.

```corvid
move = View(form_for(move_card, MoveCard(card.id, Done, 0), "Move"))
archive = View(button_for(archive_card, card.id, "Archive"))
```

- `form_for` takes an action by name where it takes a route today, and the compiler checks `initial` against the action's input. The form posts to `/actions/move_card`.
- `button_for(action, value, label)` replaces `ui.button(label, action, value)`.
- A plain form cannot set `X-CSRF-Token`, so both add a hidden `corvid_csrf` field, filled from the cookie by the page script on submit. The server accepts the token from that field when the header is absent.

**Frontend owns:** the page script's CSRF fill, fetch submission and receipt, and the browser test.

**Questions:**
1. Is filling `corvid_csrf` in the page script on submit the right place, or does your page runtime already have a hook for it?
2. Should `ui.button(label, action, value)` keep working during a transition, or be refused with a pointer to `button_for` straight away?

**Answer:**

---

## 2. Assistant authority (B11-1)

[Proposal](assistant-authority.md). Posted 2026-11-26.

```corvid
identity users:
    provider google
    delegation: up_to 8 hours

action label_card(card: Int, label: String, by: Caller) -> Card requires permission("cards.edit"):
    assistants: allow "Add a label to a card."
```

- `delegation: up_to <duration>` in the `identity` block caps how long a person can hand authority to an assistant. Leaving it out means no delegation.
- `assistants: allow "..."` sits beside today's `assistants: confirm "..."`. The assistant may run the action without asking each time, but only through a grant that names it, or in the person's own session. `approve` inside the action still applies.

**Frontend owns:** later, a "grant my assistant access" screen. Nothing in this slice renders.

**Questions:**
1. Does the `identity` block parser on your side accept a new item without other changes?
2. Any objection to `allow` and `confirm` being the two words after `assistants:`?

**Answer:**

---

## 3. Verified webhooks (W1 to W3)

[Proposal](verified-webhooks.md). Posted 2026-11-26.

```corvid
route POST "/webhooks/stripe" body PaymentEvent verified stripe(secret: "STRIPE_WEBHOOK_SECRET", within: 5 minutes) -> json Nothing:
```

- `verified <provider>(secret: ..., within: ...)` sits on the route line where `requires` sits. The server checks the signature on the raw body before decoding it.

**Frontend owns:** nothing. A webhook route is never called from a page. It is listed here only because route lines are grammar your side may parse.

**Question:**
1. Does your side parse route lines, and if so, is a `verified ...` clause before `->` safe for it?

**Answer:**

---

## 4. Keeping the person's provider token (user credentials C2)

[Proposal](user-credentials.md), part C2. Posted 2026-11-26. Part C1, an operation's own `auth: bearer(token)`, is connector grammar only backend code writes, and is already built.

```corvid
identity users:
    provider github:
        keep_token: scopes("repo")

action file_issue(repo: String, title: String, by: Caller) -> Result<Issue, IssueError> requires permission("issues.file"):
    return match by.token("github"):
        Some(token) -> github_api.create_issue(token, repo, NewIssue(title))
        None -> Err(SignInAgain)
```

- `keep_token: scopes(...)` under a provider asks for those scopes at sign-in and keeps the token. Without it nothing is kept, which is today's behavior.
- `by.token("github")` is an `Option<Secret>`. The token is held in memory beside the session only (narrowed until encrypted storage exists), so after a restart it is `None` and the person signs in again.
- `provider github:` gains an indented block. Today `provider github` is one line.

**Frontend owns:** later, a "sign in with GitHub again" prompt when an action returns `SignInAgain`, using the scopes from the contract.

**Questions:**
1. Is an indented block under `provider github:` acceptable to your `identity` parser? The other spelling considered was `provider github keep_token scopes("repo")` on one line.
2. Should the contract publish each provider's kept scopes, so your sign-in screen can request them?

**Answer:**
