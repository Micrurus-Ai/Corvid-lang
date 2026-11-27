# Error codes

Every diagnostic Corvid prints carries a code, such as `[E0208] error: type mismatch`.
Each error declares its own code, so a code never changes when its message is reworded,
and no two errors share one. A code is never reused for a different error.
Codes are grouped by area. `E` marks an error and `W` a warning.

This page is generated from the compiler's diagnostic kinds by
`corvid-driver/src/error_reference.rs`, and a test fails when it falls behind.

## E00xx: Lexer and parser

| Code | Error | Meaning |
| --- | --- | --- |
| `E0001` | `LexErrorKind::UnterminatedString` | A string literal was opened but never closed. |
| `E0002` | `LexErrorKind::TabIndentation` | A tab character was used for indentation. |
| `E0003` | `LexErrorKind::UnexpectedChar` | A character we don't recognize (e.g. `!` alone, `$`, non-ASCII symbol). |
| `E0004` | `LexErrorKind::InvalidEscape` | A `\x` escape where `x` is not a known escape character. |
| `E0005` | `LexErrorKind::InconsistentDedent` | A dedent doesn't land on any previous indent level. |
| `E0006` | `LexErrorKind::InvalidNumber` | A number literal couldn't be parsed. |
| `E0051` | `ParseErrorKind::ChainedComparison` | `a < b < c` — chained comparison, disallowed in v0.1. |
| `E0052` | `ParseErrorKind::UnclosedParen` | Something inside parentheses didn't close. |
| `E0053` | `ParseErrorKind::ExpectedBlock` | Expected the start of a block (indent) but found something else. |
| `E0054` | `ParseErrorKind::EmptyBlock` | A block had no statements. Use `pass` for an intentional no-op. |
| `E0055` | `ParseErrorKind::UnexpectedEof` | Parser reached EOF but expected more tokens. |
| `E0056` | `ParseErrorKind::UnexpectedToken` | Found one token; expected something else. `got` is a human-readable description of what we saw. |
| `E0057` | `ParseErrorKind::UnclosedBracket` | Something inside brackets didn't close. |

## E01xx: Effects, approval and secrets

| Code | Error | Meaning |
| --- | --- | --- |
| `E0101` | `TypeErrorKind::UnapprovedDangerousCall` | The headline error: a `dangerous` tool was called without a matching prior `approve` in the same block. |
| `E0102` | `TypeErrorKind::InvalidQualifiedApproval` | A qualified approval label that does not name a function declared in a Python import block. |
| `E0103` | `TypeErrorKind::SecretOperatorMismatch` | An operator other than `==` and `!=` between two secrets, or a `Secret` combined with another type. |
| `E0104` | `TypeErrorKind::SecretMethod` | A method called on a `Secret` other than `decode_base64_after` and `verify_password`. |
| `E0105` | `TypeErrorKind::SecretAtBoundary` | A `Secret` at a place its text would leave the program: a route, a store, a prompt, a connector operation or `main`. `at` is empty or names the record field that holds it. |
| `E0106` | `TypeErrorKind::SecretShownToPerson` | `ask` answering with, or `choose` offering, a `Secret`. |
| `E0107` | `TypeErrorKind::WeakUpgradeAcrossEffects` | The checker cannot prove that `upgrade()` happens before an invalidating effect in the weak's effect row. |
| `E0108` | `TypeErrorKind::ApprovalPolicyInvalid` | An approval-capable route or action omitted or contradicted its complete source-declared queue/reviewer policy. |
| `E0109` | `TypeErrorKind::UnapprovedHighTrustCall` | A tool whose composed effect row carries `trust: supervisor_required` or `trust: human_required` was called without a matching prior `approve` — the approve requirement is DERIVED from the trust tier, same machinery as `dangerous`. The variant names the deriving effect and tier so the author can see which declaration created the obligation. |
| `E0110` | `TypeErrorKind::ApprovalReachabilityViolation` | An externally reachable route, schedule, or exported agent can reach a dangerous tool without a matching approval contract in lexical scope. |
| `E0111` | `TypeErrorKind::EvalUnknownApproval` | `assert approved <label>` references an approval label that does not match any dangerous tool label in the file. |
| `E0112` | `TypeErrorKind::NonReplayableCall` | An agent marked `@replayable` calls a function that introduces nondeterminism the trace schema cannot capture. See `crate::determinism` for the catalog of nondeterministic builtins and `docs/phases/phase-21-determinism-sources.md` for which trace events must capture each source. |
| `E0113` | `TypeErrorKind::NonDeterministicCall` | An agent marked `@deterministic` calls something the compiler cannot prove is pure: an LLM prompt, a tool, an approve block, a catalog-registered nondeterministic builtin, or another agent not itself marked `@deterministic`. `@deterministic` is strictly stronger than `@replayable` — it requires the agent body to be a pure function of its parameters, trace or no trace. |
| `E0114` | `TypeErrorKind::GroundedPureLaundering` | An agent marked `@grounded_pure` (Provenance Propagation D6) launders a `Grounded<T>` value somewhere in its body — either via a silent `Grounded<T> -> T` coercion (slice 7 recorded site), an explicit `.unwrap_discarding_sources()` call, or a transitive call into a non-`@grounded_pure` agent that might launder internally. The proof obligation composes through the call graph the same way `@deterministic` does. |
| `E0115` | `TypeErrorKind::TaintedDangerousArgument` | Slice 50i — UNTRUSTED content (a `data: untrusted` source or a prompt output derived from one) reached an approval-requiring call's arguments. The injection sink rule. |
| `E0116` | `TypeErrorKind::ReplayTraceNotATraceId` | The `<expr>` in `replay <expr>:` didn't evaluate to `TraceId` (or `String`, which coerces to `TraceId` inside a replay context). |
| `E0117` | `TypeErrorKind::ReplayArmTypeMismatch` | A replay arm body's type doesn't match the first arm's type, so the replay expression can't have a single result type. `context` points at whether this arm is a `when` or `else`. |
| `E0118` | `TypeErrorKind::SecretKeyPrefixNotLiteral` | `secret.decode_base64_after(prefix)` with a prefix that is not a string literal. |

## E02xx: Types

| Code | Error | Meaning |
| --- | --- | --- |
| `E0201` | `TypeErrorKind::ArityMismatch` | Wrong number of arguments in a call. |
| `E0202` | `TypeErrorKind::UnknownField` | A field that doesn't exist on the given struct. |
| `E0203` | `TypeErrorKind::NotCallable` | Calling something that isn't callable (e.g. a primitive value). |
| `E0204` | `TypeErrorKind::NotAStruct` | Field access on a non-struct value. |
| `E0205` | `TypeErrorKind::TypeAsValue` | A type name used where a value was expected. E.g. `x = String` or `greet(Int)`. |
| `E0206` | `TypeErrorKind::BareFunctionReference` | A tool or agent referenced without `()`. E.g. `x = get_order` instead of `x = get_order(id)`. |
| `E0207` | `TypeErrorKind::ReturnTypeMismatch` | The return type declared doesn't match what the body returns. |
| `E0208` | `TypeErrorKind::TypeMismatch` | An argument's type doesn't match the parameter's declared type. |
| `E0209` | `TypeErrorKind::UngroundedReturn` | An agent returns `Grounded<T>` but the compiler cannot prove a provenance path from a `data: grounded` source feeds into the return value. |
| `E0210` | `TypeErrorKind::ImpossibleConflictCase` | A pattern names a conflict case the matched value can never be: `Err(Changed(_))` on an `insert`, which only returns `Taken`. |
| `E0211` | `TypeErrorKind::InvalidAssignTarget` | The target of a place assignment (`x.field = v`, `xs[i] = v`, compound `+=`) is not assignable — e.g. its root is not a local variable (slice 45b). |
| `E0212` | `TypeErrorKind::TimeOperatorMismatch` | B06 — an operator that does not apply to the time types involved. |
| `E0213` | `TypeErrorKind::InvalidTryPropagate` | `expr?` was used on a value that is neither `Result` nor `Option`. |
| `E0214` | `TypeErrorKind::TryPropagateReturnMismatch` | `expr?` was used in a function whose return type cannot absorb the propagated error/none branch. |
| `E0215` | `TypeErrorKind::InvalidRetryTarget` | `try expr on error retry ...` was used on a value that is neither `Result` nor `Option`. |
| `E0216` | `TypeErrorKind::GenericArityMismatch` | A compiler-known generic received the wrong number of type arguments. |
| `E0217` | `TypeErrorKind::InvalidWeakTargetType` | `Weak<T>` was declared over a non-heap-backed target type. |
| `E0218` | `TypeErrorKind::InvalidWeakNewTarget` | `Weak::new(value)` only accepts heap-backed strong values. |
| `E0219` | `TypeErrorKind::InvalidWeakUpgradeTarget` | `Weak::upgrade(value)` requires a weak reference. |
| `E0220` | `TypeErrorKind::DirectPythonCallUnsupported` | A call to a Python function the import block declares no signature for. |
| `E0221` | `TypeErrorKind::LoopFlowOutsideLoop` | `break` or `continue` outside any enclosing loop (slice 45k). |
| `E0222` | `TypeErrorKind::AliasCycle` | A type-alias chain that never reaches a concrete type (slice 45n): `type A = B` / `type B = A`. |
| `E0223` | `TypeErrorKind::FnBodyNotPure` | A `fn` body containing an effectful construct (slice 45r): tool/prompt/agent/fixture calls, `approve`, `ask`/`choose`, `replay`, or `yield`. A `fn` is statically effect-free. |
| `E0224` | `TypeErrorKind::ParallelArmInvalid` | A malformed `parallel:` arm (slice 46e). |
| `E0226` | `TypeErrorKind::ActorFromInput` | A `Caller` where a request, a client or a model supplies the value. |
| `E0227` | `TypeErrorKind::TryElseFallbackType` | `try ... else <fallback>` where the fallback's type is not the body's. |
| `E0228` | `TypeErrorKind::TryElseAroundOnFailure` | `try ... else` around a connector operation that declares `on failure`: its outages already arrive as a value. |
| `E0229` | `TypeErrorKind::AnnotationInvalid` | A malformed `@retry(...)` / `@idempotency(...)` annotation (slice 45q): zero attempts, key naming no parameter, or a key parameter of non-derivable type. |
| `E0230` | `TypeErrorKind::AssertNotBool` | `assert <expr>` inside an eval must typecheck to Bool. |
| `E0231` | `TypeErrorKind::InvalidConfidence` | Statistical assertion modifiers must stay in range. |
| `E0232` | `TypeErrorKind::InvalidCustomDimension` | A custom dimension declared in `corvid.toml` under `[effect-system.dimensions.*]` failed validation. Rejects unknown composition rules, unknown value-types, malformed defaults, and collisions with built-in dimension names. |
| `E0233` | `TypeErrorKind::RolloutPercentOutOfRange` | A `rollout N%` clause's percentage is outside `[0.0, 100.0]`. |
| `E0234` | `TypeErrorKind::AdversarialStageArity` | An `adversarial:` stage's target prompt has the wrong number of parameters for its position in the pipeline. |
| `E0235` | `TypeErrorKind::AdversarialStageParamType` | An `adversarial:` stage's target prompt has a parameter whose type does not match the previous stage's return type (or, for the proposer, the outer prompt's parameter type). |
| `E0236` | `TypeErrorKind::AdversarialStageReturnType` | The `adjudicate` stage's return type does not match the outer prompt's declared return type. |
| `E0237` | `TypeErrorKind::NonScalarInExternC` | `pub extern "c"` currently supports only scalar ABI types. |
| `E0238` | `TypeErrorKind::AmbiguousExternOwnership` | The compiler cannot infer a sound ownership contract for an extern-visible boundary slot without an explicit annotation. |
| `E0239` | `TypeErrorKind::ExternOwnershipMismatch` | A user-declared ownership annotation disagrees with the compiler's inference for the same extern-visible boundary slot. |
| `E0240` | `TypeErrorKind::UnknownMethod` | A method call on a type that has no method of that name: neither a builtin method nor, for a record, one added with `extend`. |
| `E0250` | `TypeErrorKind::EffectConstraintViolation` | A dimensional effect constraint was violated. |

## E03xx: Names and imports

| Code | Error | Meaning |
| --- | --- | --- |
| `E0301` | `ResolveErrorKind::UndefinedName` | An identifier was used but never declared in any enclosing scope. |
| `E0302` | `ResolveErrorKind::DuplicateDecl` | Two top-level declarations share the same name. `first_span` points to the first declaration; `span` (on the outer `ResolveError`) points to the duplicate. |
| `E0303` | `TypeErrorKind::CorvidImportNotYetResolved` | `alias.TypeName` type reference encountered, but the Corvid `.cor` import-resolver is not yet wired up. The grammar parses cleanly; what's missing is module loading + qualified-name resolution, which lands in `lang-cor-imports-basic-resolve`. |
| `E0304` | `TypeErrorKind::UnknownImportAlias` | `alias.Name` references an import alias that isn't declared in the current file's `import` statements. Either a typo or a missing `import` line. |
| `E0305` | `TypeErrorKind::ImportedDeclIsPrivate` | `alias.Name` references an imported file that exists, but the declaration `Name` in that file is `Private` (no `public` / `public(package)` modifier). The `lang-pub-toplevel` private-by-default rule means imports only see declarations their owner opted to expose. |
| `E0306` | `TypeErrorKind::UnknownImportMember` | `alias.Name` references a name that isn't declared in the imported file at all (publicly or privately). |
| `E0307` | `TypeErrorKind::UnknownGenericHead` | A generic type application whose head is not a known generic type (slice 45q): `Lst<Int>`, `option<T>`. Unknown heads used to silently resolve to `Type::Unknown`. |
| `E0308` | `TypeErrorKind::ReservedTypeName` | A declaration reuses the name of a core scalar (`Int`, `Float`, `String`, `Bool`, `Nothing`), which always means the built-in, so the program's type could never be named. |
| `E0320` | `ResolveErrorKind::ExtendTargetNotAType` | `extend T:` block targets a type that doesn't exist (or isn't a type — e.g., the user wrote `extend foo:` where `foo` is an agent name). |
| `E0321` | `ResolveErrorKind::DuplicateMethod` | Two methods with the same name declared inside the same `extend T:` block (or split across multiple `extend T:` blocks for the same type). |
| `E0322` | `ResolveErrorKind::MethodFieldCollision` | A method's name collides with a field name on the same type. `.foo` would be ambiguous (field access vs zero-arg method call), so we reject at declaration time. |
| `E0323` | `ResolveErrorKind::BuiltinMethodName` | An `extend T:` method uses a name every type already has (`to_json`, `from_json`, `from_json_strict`). The built-in would win at every call site, so the declaration is refused rather than silently ignored. |
| `E0324` | `ResolveErrorKind::UnknownReplayPrompt` | `replay <trace>: when llm("<name>") -> ...` — `<name>` doesn't match any `prompt` declaration in scope. Either the prompt doesn't exist, is declared under a different name, or exists under a different declaration kind (tool, agent). |
| `E0325` | `ResolveErrorKind::UnknownReplayTool` | `replay <trace>: when tool("<name>", ...) -> ...` — `<name>` doesn't match any `tool` declaration in scope. |
| `E0326` | `ResolveErrorKind::UnknownReplayApproval` | `replay <trace>: when approve("<label>") -> ...` — `<label>` doesn't appear at any `approve Label(...)` site in the file. Labels aren't declarations (they're free-form identifiers at approval sites), so resolution is a presence check against the set of labels used elsewhere in the program. |
| `E0327` | `ResolveErrorKind::ReplayPatternKindMismatch` | A replay arm names the right kind of declaration (e.g. the name *does* resolve), but the kind is wrong for the event family. For example `when llm("get_order") -> ...` where `get_order` is a `tool`, not a `prompt`. The wrong-kind case is distinguished from UnknownReplayPrompt so the diagnostic can suggest the right event family. |
| `E0330` | `ModuleLoadError::FileNotFound` | An imported module's file does not exist. |
| `E0331` | `ModuleLoadError::ReadError` | An imported module's file could not be read. |
| `E0332` | `ModuleLoadError::HashMismatch` | An imported module's content does not match its pinned `hash:sha256:`. |
| `E0333` | `ModuleLoadError::RemoteImportMissingHash` | A remote import has no `hash:sha256:` pin, so it is not fetched. |
| `E0334` | `ModuleLoadError::RemoteFetchError` | A remote import could not be fetched. |
| `E0335` | `ModuleLoadError::PackageLockMissing` | A package import, with no package lock file to resolve it. |
| `E0336` | `ModuleLoadError::PackageLockError` | The package lock file could not be read or is invalid. |
| `E0337` | `ModuleLoadError::PackageNotLocked` | A package import that the lock file does not list. |
| `E0338` | `ModuleLoadError::Cycle` | Modules import each other in a cycle. |

## E04xx: JSON and records

| Code | Error | Meaning |
| --- | --- | --- |
| `E0401` | `TypeErrorKind::NonExhaustiveMatch` | A `match` doesn't cover every case of its scrutinee (slice 45i). |
| `E0402` | `TypeErrorKind::RefinementOutOfRange` | A constant given to a refined field breaks its refinement: `Search("dune", 500)` where `limit: Int where between(1, 50)`. |
| `E0404` | `TypeErrorKind::RefinementInvalid` | Slice 50j — a field refinement whose form does not fit the field's type (or whose bounds are inverted). |
| `E0405` | `TypeErrorKind::JsonKeyInvalid` | B01 — a type's JSON keys (`@json(key:)` / `@json(case:)`) are empty or name one key twice. |
| `E0407` | `TypeErrorKind::NoJsonForm` | `T.from_json(text)` or `value.to_json()` on a type that has no public JSON form, or whose JSON form would drop a guarantee (`Grounded` provenance, `Tainted` marking). |
| `E0408` | `TypeErrorKind::StructLiteralInvalid` | A malformed named struct literal or destructuring pattern (slice 45n): unknown/duplicate/missing fields, non-record targets, bare `..` in expression position. |
| `E0409` | `TypeErrorKind::RetiredJsonDecoderTool` | A tool declared as `decode_<Type>_from_json(text: String) -> Result<T, String>`, the retired convention `T.from_json` replaced. The runtime no longer decodes JSON for it. |

## E05xx: Scalars: decimals, time, currency, bytes, text

| Code | Error | Meaning |
| --- | --- | --- |
| `E0501` | `TypeErrorKind::RegexPatternNotLiteral` | A regex method (`find_regex`, `replace_regex`, ...) whose pattern is not a string literal, so the compiler cannot check it. |
| `E0502` | `TypeErrorKind::InvalidRegexPattern` | A regex pattern literal the regex parser rejects. |
| `E0503` | `TypeErrorKind::InvalidRegexReplacement` | A `replace_regex` replacement literal that refers to a group the pattern does not have, or uses `$` in a way that is not a group reference. |
| `E0504` | `TypeErrorKind::RegexSplitMatchesEmpty` | `split_regex` with a pattern that can match the empty string. |
| `E0505` | `TypeErrorKind::InvalidDecimalLiteral` | B06 — a number literal the checker reads as a `Decimal` that has no exact `Decimal` value (too many digits). |
| `E0506` | `TypeErrorKind::DecimalFloatMix` | B06 — `Decimal` and `Float` meeting in one operator. |
| `E0507` | `TypeErrorKind::DecimalDivisionNeedsRounding` | B06 — `a / b` on `Decimal` values that is not rounded at once with a `round_<mode>(places)` call. |
| `E0508` | `TypeErrorKind::DecimalPlacesOutOfRange` | B06 — a literal `places` argument outside 0 to 28. |
| `E0509` | `TypeErrorKind::DecimalDivisionByZero` | B06 — a `Decimal` division or remainder by the literal zero. |
| `E0510` | `TypeErrorKind::UnknownTimeZone` | B06 — a string literal in a `TimeZone` slot that names no IANA zone. |
| `E0511` | `TypeErrorKind::UnknownCurrencyOrLocale` | B06 — a string literal in a `Currency` or `Locale` slot that CLDR does not know. |
| `E0512` | `TypeErrorKind::BytesOperatorMismatch` | An operator other than `==` and `!=` with a `Bytes` operand, or `Bytes` compared with another type. |
| `E0515` | `TypeErrorKind::DurationLiteralInvalid` | B06 — a duration literal that names no exact length: a calendar unit, a fraction of a nanosecond, or a value out of range. |
| `E0516` | `TypeErrorKind::FloatLiteralInexact` | An integer literal where a `Float` is meant that no `Float` holds exactly (beyond 2^53), so reading it as one would round it. |
| `E0517` | `TypeErrorKind::HmacKeyType` | An HMAC key that is neither `Bytes` nor a `Secret`. |
| `E0518` | `TypeErrorKind::IntLiteralOutOfRange` | A whole-number literal beyond `Int`'s range where no `Decimal`, or `Float` that holds it exactly, is meant. |

## E06xx: Tables and databases

| Code | Error | Meaning |
| --- | --- | --- |
| `E0601` | `TypeErrorKind::SecretTableColumn` | A table column that holds a `Secret`. |
| `E0603` | `TypeErrorKind::TableQueryNotTranslatable` | B09 P2 — a query part the database cannot run. |
| `E0604` | `TypeErrorKind::TableColumnHistoryConflict` | B09 P3 — `renamed from` or `retired` contradicts the table's declared columns or another clause. |
| `E0605` | `TypeErrorKind::AtomicBlockRefused` | B09 P7a — something an `atomic:` block cannot hold. |
| `E0606` | `TypeErrorKind::TableKeyRequired` | B09 — a table declares no `key` field. |
| `E0607` | `TypeErrorKind::TableKeyType` | B09 — a table's key is neither `Int` nor `String`. |
| `E0608` | `TypeErrorKind::TableFieldReserved` | B09 — a table declares `revision`, which every row carries. |
| `E0609` | `TypeErrorKind::TableNotPublic` | B09 — a `public` table; tables are private to their module. |
| `E0610` | `TypeErrorKind::TableOutsideDatabase` | B09 — a table no `database` block lists. |
| `E0611` | `TypeErrorKind::TableDatabaseAmbiguous` | B09 P11 — a table without `in` in a module with several databases. |
| `E0612` | `TypeErrorKind::TableDatabaseUnknown` | B09 P11 — `in` names something that is not a database here. |
| `E0613` | `TypeErrorKind::TableUniqueRule` | B09 P10 — a `unique` line that names a missing, repeated or JSON-stored column, or restates the key or another line. |
| `E0614` | `TypeErrorKind::TableOrderUnordered` | B09 P8a — `order_by` on a column whose type has no order. |
| `E0615` | `TypeErrorKind::DatabaseSettingRequired` | B09 — a `database` block omits a setting that has no default. |
| `E0616` | `TypeErrorKind::DatabaseSettingInvalid` | B09 P4 — a database setting whose value this engine does not allow. |
| `E0617` | `TypeErrorKind::DatabaseEngineUnavailable` | B09 — an engine the runtime cannot run yet, or does not know. |
| `E0650` | `table schema` | Two modules of the program declare a table with the same name. |
| `E0651` | `table schema` | A table's indexes cannot be derived from its keys, unique rules and queries. |

## E07xx: Actions, routes and the browser

| Code | Error | Meaning |
| --- | --- | --- |
| `E0701` | `TypeErrorKind::LiveDeclarationInvalid` | A `live` declaration (B14) that is not one table query of the row type it delivers, or whose name is unusable. |
| `E0703` | `TypeErrorKind::IdentityConfigInvalid` | An `identity Name:` block with an invalid configuration (slice 51g): no providers, a duplicate provider, an unsafe cookie option without the explicit opt-out, or a malformed OIDC discovery URL. |
| `E0704` | `TypeErrorKind::RoutePolicyWithoutIdentity` | A route carries a `requires authenticated\|role\|permission` policy (slice 51h) but the program declares no `identity` block, so there is no way to authenticate a caller. |
| `E0705` | `TypeErrorKind::RoutePolicyUndeclaredAuthority` | A route `requires role("x")` / `requires permission("p")` names an authority (`kind` = "role" or "permission") that the identity block's `roles:` never declares (slice 52f) — the requirement could never be satisfied, so it is almost certainly a typo. |
| `E0706` | `TypeErrorKind::ApprovalRequiresAuthentication` | A route or an action can reach an `approve` boundary but does not require an authenticated caller. Without a verified requester there is no honest actor or tenant to put on the durable approval record. `subject` names it: "route `/x`" or "action `x`". |
| `E0707` | `TypeErrorKind::RouteUploadPolicyInvalid` | A route-level `@upload(...)` policy is missing, incomplete, or attached to a route whose body is not `Upload<Format>`. |
| `E0708` | `TypeErrorKind::RoutePathParamType` | A typed path placeholder, `{board: Int}`, of a type a path segment cannot be read as. |
| `E0709` | `TypeErrorKind::ActionPolicyWithoutIdentity` | An `action` whose `requires` names authorities, with no `identity` block to authenticate against (B10). |
| `E0710` | `TypeErrorKind::ActionPolicyUndeclaredAuthority` | An `action` requiring a role or permission the identity block does not declare (B10). |
| `E0711` | `TypeErrorKind::ActionPathTaken` | A route declared at the path an action is served at. |
| `E0712` | `TypeErrorKind::AssistantsWithoutIdentity` | An action offered to assistants in a program with no identity block: no person could sign in to confirm its calls. |
| `E0713` | `TypeErrorKind::ActionCallerWithoutSignIn` | An action reads its caller (a `Caller` parameter) but requires no sign-in, so no caller is ever verified. |
| `E0714` | `TypeErrorKind::InvalidForm` | A browser form that does not match what it submits to; `message` names the mismatch. |
| `E0715` | `TypeErrorKind::InvalidLiveBinding` | A browser binding to a `live` route that does not match its declaration; `message` names the mismatch. |
| `E0720` | `TypeErrorKind::DuplicateServerRoute` | A backend server declared the same method/path pair more than once. |
| `E0721` | `TypeErrorKind::GetRouteBody` | A GET route declared a request body. |
| `E0722` | `TypeErrorKind::RouteMethodAnsweredByServer` | A route declared with `HEAD` or `OPTIONS`, which the server answers itself (B05: the methods exist for connector operations). |
| `E0725` | `TypeErrorKind::RouteQueryFieldType` | A route's `query` record with a field the server cannot read from a query string; `field` is empty when the query type is not a record at all. |

## E08xx: Connectors

| Code | Error | Meaning |
| --- | --- | --- |
| `E0801` | `TypeErrorKind::ConnectorConfigInvalid` | A `connector Name:` block with an invalid configuration (slice 52g): a non-absolute `base_url`, a malformed reliability value, a duplicate operation, a path placeholder naming no parameter, etc. |
| `E0802` | `TypeErrorKind::ResponseOutsideCaller` | A connector `Response` outside the program that made the call. |

## E09xx: Streams, prompts, tools and agents

| Code | Error | Meaning |
| --- | --- | --- |
| `E0902` | `TypeErrorKind::YieldOutsideAgent` | `yield` is only valid inside agent bodies. |
| `E0903` | `TypeErrorKind::PromptHistoryInvalid` | A misused conversation-history parameter (slice 46c). |
| `E0904` | `TypeErrorKind::ModelFieldInvalid` | A model declaration field with an out-of-range value (slice 46a): `temperature: 9.5`. |
| `E0905` | `TypeErrorKind::YieldReturnTypeMismatch` | A yielded value did not match the agent's declared `Stream<T>`. |
| `E0906` | `TypeErrorKind::EvalUnknownTool` | `assert called <tool>` or `assert called A before B` references a name that does not resolve to a known callable. |
| `E0907` | `TypeErrorKind::PromptCitationUnknownParam` | A prompt-level `cites <param> strictly` clause names a parameter that does not exist on the prompt. |
| `E0908` | `TypeErrorKind::EnsembleDuplicateModel` | An `ensemble [..]` list names the same model more than once. Voting where two slots share a model degenerates to voting over fewer opinions than intended. |
| `E0909` | `TypeErrorKind::AdversarialStageNotPrompt` | An `adversarial:` stage (propose / challenge / adjudicate) points at a name that is not a `prompt` declaration. Stages dispatch to prompts because the runtime chains stage outputs as positional arguments to the next stage. |
| `E0910` | `TypeErrorKind::YieldRequiresStreamReturn` | An agent body used `yield` without declaring `Stream<T>`. |
| `E0911` | `TypeErrorKind::PromptCitationRequiresGrounded` | A prompt-level `cites <param> strictly` clause can only cite a `Grounded<T>` parameter, because runtime citation verification must be tied to a compile-time provenance-carrying context. |
| `E0912` | `TypeErrorKind::RouteTargetNotModel` | A `route:` arm inside a prompt points at a name that is not a `model` declaration. The runtime can only dispatch to models, so the target must be a `Decl::Model`. |
| `E0913` | `TypeErrorKind::RouteGuardNotBool` | A `route:` arm's guard expression is not a Bool. |
| `E0914` | `TypeErrorKind::ModelOutputFormatMismatch` | A prompt routed to a model whose declared output format cannot produce the prompt's return type. |
| `E0915` | `TypeErrorKind::AdversarialAdjudicatorMissingContradictionField` | The `adjudicate` stage's return type is not a struct, or the struct it returns has no `contradiction: Bool` field. The runtime reads this field to decide whether to emit `TraceEvent::AdversarialContradiction`. |

## E10xx: Builds and targets

| Code | Error | Meaning |
| --- | --- | --- |
| `E1001` | `native build` | The program cannot be built as native code yet; the message names what stops it. |

## E52xx: Identity, access and startup

| Code | Error | Meaning |
| --- | --- | --- |
| `E5204` | `startup refusal` | `corvid serve` refuses to start: a declared contract (such as a route's `requires`) has no runtime path that enforces it. |
| `E5205` | `startup refusal` | `corvid serve` refuses to start: a connector's mode, host or credential is not allowed or not resolved. |
| `E5210` | `TypeErrorKind::FirstLoginPolicyRequired` | An `identity Name:` block declares OAuth providers but does not declare a first-login provisioning policy (slice 52e). There is NO silent default: a program must not back into open-registration (or any other tenancy posture) by omission, so the missing decision is a compile error naming the allowed values. |

## W01xx: Warnings: effects and replay

| Code | Error | Meaning |
| --- | --- | --- |
| `W0101` | `TypeWarningKind::ReplayUnreachableArm` | A replay arm duplicates an earlier arm's pattern; the later arm can never match. Phase 21 slice 21-inv-E-3. |

## W02xx: Warnings: types

| Code | Error | Meaning |
| --- | --- | --- |
| `W0201` | `TypeWarningKind::UnsafePythonImport` | `effects: unsafe` on a Python import is explicit but should be reviewed. |
| `W0202` | `TypeWarningKind::InferenceNeedsAnnotation` | An unannotated binding whose initializer cannot determine its full type (slice 45q): `x = []` (element type Unknown) or `x = None` (payload type Unknown). Downstream checking weakens to Unknown-accepts-everything; an annotation restores full checking. |
| `W0251` | `TypeWarningKind::UnboundedCostAnalysis` | The cost explorer could not prove a static upper bound. |

## W03xx: Warnings: names

| Code | Error | Meaning |
| --- | --- | --- |
| `W0301` | `TypeWarningKind::BuiltinTypeHidden` | A type the file declares or imports has the name of a built-in type, so in this file the name means the program's type and the built-in is hidden. Allowed, so a built-in added later never breaks a program, but said, because a value a built-in returns (such as `from_json`'s `JsonError`) is still the built-in type. |

## W07xx: Warnings: identity

| Code | Error | Meaning |
| --- | --- | --- |
| `W0701` | `TypeWarningKind::IdentityInsecureSession` | An `identity` block explicitly weakened a session safe-default via `insecure_opt_out: true` (slice 51g). Allowed, but never silent: the weakened posture surfaces in review and audit. |

## W09xx: Warnings: streams and schedules

| Code | Error | Meaning |
| --- | --- | --- |
| `W0901` | `TypeWarningKind::StreamReturnWithoutYield` | An agent declared `Stream<T>` but never actually yielded. |
| `W0902` | `TypeWarningKind::ScheduleNotExecutable` | A `schedule` declaration fires only under the governed-cron scheduler runner (`corvid schedule run`), never during a plain `corvid run` — which executes `main` once. Without this note, a reviewer writing `schedule "0 9 * * *" zone "America/New_York" -> ...` and testing via `corvid run` would expect the cron to fire and have no signal about which command starts it (the original gap surfaced in self-trial round 4 against a job-coordinator project, before the runner shipped). |
