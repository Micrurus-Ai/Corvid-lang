# Proposal: JSON keys that differ from field names (B01)

Status: **awaiting frontend agreement.** No code is written yet.
Owner: backend and AI development. Needs agreement from frontend development because it adds field and type syntax.

## Problem

External APIs use key styles that Corvid field names do not, such as `userId`, `created-at` and `ID`. Today a record decodes from JSON only if every key matches a field name exactly. A Corvid program consuming a camelCase API must either name its fields `userId`, which breaks the project's snake_case naming rule, or decode with `json_get_*` one field at a time.

Python and TypeScript developers expect this to be solved. pydantic has `Field(alias="userId")` and `alias_generator=to_camel`. serde has `#[serde(rename = "userId")]` and `#[serde(rename_all = "camelCase")]`. zod and TypeScript handle it in a mapping layer.

## Proposed syntax

Two attributes, using the leading `@name(key: value)` form fields already have for `@ui(...)` and `@upload(...)`.

**One field:**

```corvid
type User:
    @json(key: "userId")
    user_id: Int
    email: String
```

**A whole type:**

```corvid
@json(case: camel)
type Order:
    order_id: Int
    created_at: String
    @json(key: "ID")
    legacy_id: String
```

`case: camel` derives every key from the snake_case field name: `order_id` becomes `orderId` and `created_at` becomes `createdAt`. A field's own `@json(key: ...)` overrides the type's case, so `legacy_id` is `ID`. The allowed values are:

| `case` | `created_at` becomes |
| --- | --- |
| `camel` | `createdAt` |
| `pascal` | `CreatedAt` |
| `kebab` | `created-at` |

Without either attribute, the key is the field name, as today. That is a naming default, not a consequential policy in the CLAUDE.md sense, so no declaration is required.

**Alternatives considered:**
- `user_id: Int as "userId"` collides visually with the trailing `where` refinement and adds a keyword.
- pydantic-style `user_id: Int = field(alias="userId")` reads as a default value.
- A bare `@json("userId")` is shorter, but `key:` leaves room for later options such as `omit_if_none` without a second syntax.

## Semantics

The key applies wherever the **public JSON mapping** is used, and nowhere else:

| Surface | Uses the JSON key |
| --- | --- |
| HTTP route bodies and JSON replies | yes |
| `T.from_json`, `T.from_json_strict`, `value.to_json()` | yes |
| OpenAPI schema property names | yes; the Corvid name is also recorded as `x-corvid-field` |
| TypeScript client property names | yes, because the client types the wire shape |
| `JsonError.path` | yes; the path names the key the sender wrote, such as `userId` |
| Corvid source (`user.user_id`) | no; code always uses the field name |
| Prompt structured output, tool arguments, recorded traces | no; the internal envelope keeps field names so traces and replay stay stable |
| HTML form posts | no; see question 2 |

Compile-time rules, each with an error that names the fields involved:

1. Two fields of one type may not map to the same key, whether through `key:` or `case:`.
2. A key must be a non-empty string.
3. `@json` on a variant payload field is an error. Variant payloads travel positionally in `"fields": [...]`, so a key would have no effect, and an attribute with no effect is not allowed.
4. `case:` accepts only the listed values; anything else is an error listing them.

Strict decoding rejects the field name when a different key is declared: `from_json_strict` on `{"user_id": 1}` fails for the `User` above. Accepting both spellings (serde's `alias`) is left out of this slice.

## Implementation outline (backend)

Following the single-source-of-truth rule, one function computes a field's key, `json_key(field, type_decl) -> String`, and every consumer calls it: `public_json` decode and encode, the checker's collision rule, OpenAPI, the TypeScript client and the Swift, Kotlin and Python SDKs. The parser change is small: `@json` joins the field attribute loop in `parser/decl/type_field.rs`, and `type` declarations gain a leading attribute for `@json(case: ...)`. That second change is the new grammar this proposal needs agreement on.

## Questions for frontend development

1. **Is the syntax acceptable?** That is `@json(key: "...")` on fields, and `@json(case: ...)` as the first leading attribute a `type` declaration can take. If you have plans for type-level attributes such as `@ui(...)` on a type, the parser change should leave room for them.
2. **Form posts.** Generated forms (`interp/form.rs`, `form_controls.js`) post URL-encoded fields named after the Corvid fields. This proposal leaves forms alone, since they are a round trip between Corvid's own renderer and parser and not JSON. Do you agree, or should forms use the key too?
3. **Browser decoding.** Roadmap item "B01 with all client data" says browser code decodes the same records with the same errors. Does any browser code you own read record JSON by field name? If so, it should read the key from the published contract, and we should agree where in the contract the key lives. The proposal above puts it in the OpenAPI property name.
4. **The TypeScript client.** Its property would become `userId`, matching the wire. Does any frontend code you own depend on the client using Corvid field names?

Please answer in a "Frontend response" section at the end of this file, or through the user. Backend work starts once the syntax in question 1 is agreed.
