//! Type error kind definitions and user-facing diagnostics.

#[derive(Debug, Clone, PartialEq)]
pub enum TypeErrorKind {
    /// A `match` doesn't cover every case of its scrutinee
    /// (slice 45i).
    NonExhaustiveMatch { missing: String },

    /// A `live` declaration (B14) that is not one table query of the
    /// row type it delivers, or whose name is unusable.
    LiveDeclarationInvalid { path: String, problem: String },

    /// A constant given to a refined field breaks its refinement:
    /// `Search("dune", 500)` where `limit: Int where between(1, 50)`.
    RefinementOutOfRange { type_name: String, field: String, refinement: String, value: String },

    /// A pattern names a conflict case the matched value can never be:
    /// `Err(Changed(_))` on an `insert`, which only returns `Taken`.
    ImpossibleConflictCase { case: String, possible: String },

    /// The target of a place assignment (`x.field = v`, `xs[i] = v`,
    /// compound `+=`) is not assignable — e.g. its root is not a
    /// local variable (slice 45b).
    InvalidAssignTarget { reason: String },

    /// Wrong number of arguments in a call.
    ArityMismatch {
        callee: String,
        expected: usize,
        got: usize,
    },

    /// An argument's type doesn't match the parameter's declared type.
    TypeMismatch {
        expected: String,
        got: String,
        /// Optional context for where the mismatch was detected.
        context: String,
    },

    /// A field that doesn't exist on the given struct.
    UnknownField { struct_name: String, field: String },

    /// Calling something that isn't callable (e.g. a primitive value).
    NotCallable { got: String },
    InvalidQualifiedApproval,

    /// Field access on a non-struct value.
    NotAStruct { got: String },

    /// A type name used where a value was expected.
    /// E.g. `x = String` or `greet(Int)`.
    TypeAsValue { name: String },

    /// `T.from_json(text)` or `value.to_json()` on a type that has no
    /// public JSON form, or whose JSON form would drop a guarantee
    /// (`Grounded` provenance, `Tainted` marking).
    NoJsonForm {
        method: String,
        ty: String,
        reason: String,
        hint: String,
    },

    /// A tool declared as `decode_<Type>_from_json(text: String) ->
    /// Result<T, String>`, the retired convention `T.from_json`
    /// replaced. The runtime no longer decodes JSON for it.
    RetiredJsonDecoderTool { tool: String, replacement: String },

    /// A regex method (`find_regex`, `replace_regex`, ...) whose
    /// pattern is not a string literal, so the compiler cannot check it.
    RegexPatternNotLiteral { method: String },

    /// A regex pattern literal the regex parser rejects.
    InvalidRegexPattern { method: String, problem: String },

    /// A `replace_regex` replacement literal that refers to a group the
    /// pattern does not have, or uses `$` in a way that is not a group
    /// reference.
    InvalidRegexReplacement { problem: String, hint: String },

    /// `split_regex` with a pattern that can match the empty string.
    RegexSplitMatchesEmpty { pattern: String },

    /// B06 — a number literal the checker reads as a `Decimal` that has
    /// no exact `Decimal` value (too many digits).
    InvalidDecimalLiteral { literal: String, problem: String },

    /// B06 — `Decimal` and `Float` meeting in one operator.
    DecimalFloatMix { context: String },

    /// B06 — `a / b` on `Decimal` values that is not rounded at once
    /// with a `round_<mode>(places)` call.
    DecimalDivisionNeedsRounding,

    /// B06 — a literal `places` argument outside 0 to 28.
    DecimalPlacesOutOfRange { places: i64 },

    /// B06 — a `Decimal` division or remainder by the literal zero.
    DecimalDivisionByZero,

    /// B06 — a string literal in a `TimeZone` slot that names no IANA zone.
    UnknownTimeZone { name: String, suggestion: Option<String> },

    /// B06 — a string literal in a `Currency` or `Locale` slot that CLDR
    /// does not know.
    UnknownCurrencyOrLocale { problem: String },

    /// B06 — an operator that does not apply to the time types involved.
    TimeOperatorMismatch { left: String, right: String, hint: String },

    /// An operator other than `==` and `!=` with a `Bytes` operand, or
    /// `Bytes` compared with another type.
    BytesOperatorMismatch { left: String, right: String, hint: String },

    /// An operator other than `==` and `!=` between two secrets, or a
    /// `Secret` combined with another type.
    SecretOperatorMismatch { left: String, right: String, hint: String },

    /// A method called on a `Secret` other than `decode_base64_after`
    /// and `verify_password`.
    SecretMethod { method: String },

    /// `secret.decode_base64_after(prefix)` with a prefix that is not a
    /// string literal.
    SecretKeyPrefixNotLiteral,

    /// An HMAC key that is neither `Bytes` nor a `Secret`.
    HmacKeyType { method: String, got: String },

    /// A `Secret` at a place its text would leave the program: a route,
    /// a store, a prompt, a connector operation or `main`. `at` is empty
    /// or names the record field that holds it.
    SecretAtBoundary { boundary: String, at: String, hint: String },

    /// A table column that holds a `Secret`.
    SecretTableColumn { column: String, at: String },

    /// `ask` answering with, or `choose` offering, a `Secret`.
    SecretShownToPerson { builtin: String, at: String },


    /// A tool or agent referenced without `()`.
    /// E.g. `x = get_order` instead of `x = get_order(id)`.
    BareFunctionReference { name: String },

    /// `expr?` was used on a value that is neither `Result` nor `Option`.
    InvalidTryPropagate { got: String },

    /// `expr?` was used in a function whose return type cannot absorb the
    /// propagated error/none branch.
    TryPropagateReturnMismatch { expected: String, got: String },

    /// `try expr on error retry ...` was used on a value that is neither
    /// `Result` nor `Option`.
    InvalidRetryTarget { got: String },

    /// A compiler-known generic received the wrong number of type arguments.
    GenericArityMismatch {
        name: String,
        expected: usize,
        got: usize,
    },

    /// `Weak<T>` was declared over a non-heap-backed target type.
    InvalidWeakTargetType { got: String },

    /// `Weak::new(value)` only accepts heap-backed strong values.
    InvalidWeakNewTarget { got: String },

    /// `Weak::upgrade(value)` requires a weak reference.
    InvalidWeakUpgradeTarget { got: String },

    /// The checker cannot prove that `upgrade()` happens before an
    /// invalidating effect in the weak's effect row.
    WeakUpgradeAcrossEffects { effects: Vec<String> },

    /// The return type declared doesn't match what the body returns.
    ReturnTypeMismatch { expected: String, got: String },

    DirectPythonCallUnsupported { module: String, member: String },

    /// `alias.TypeName` type reference encountered, but the Corvid
    /// `.cor` import-resolver is not yet wired up. The grammar parses
    /// cleanly; what's missing is module loading + qualified-name
    /// resolution, which lands in `lang-cor-imports-basic-resolve`.
    CorvidImportNotYetResolved { alias: String, name: String },

    /// `alias.Name` references an import alias that isn't declared
    /// in the current file's `import` statements. Either a typo or
    /// a missing `import` line.
    UnknownImportAlias { alias: String },

    /// `alias.Name` references an imported file that exists, but
    /// the declaration `Name` in that file is `Private` (no
    /// `public` / `public(package)` modifier). The `lang-pub-toplevel`
    /// private-by-default rule means imports only see declarations
    /// their owner opted to expose.
    ImportedDeclIsPrivate { alias: String, name: String },

    /// `alias.Name` references a name that isn't declared in the
    /// imported file at all (publicly or privately).
    UnknownImportMember { alias: String, name: String },

    /// `yield` is only valid inside agent bodies.
    YieldOutsideAgent,

    /// `break` or `continue` outside any enclosing loop (slice 45k).
    LoopFlowOutsideLoop { keyword: String },

    /// A type-alias chain that never reaches a concrete type
    /// (slice 45n): `type A = B` / `type B = A`.
    AliasCycle { name: String },

    /// A `fn` body containing an effectful construct (slice 45r):
    /// tool/prompt/agent/fixture calls, `approve`, `ask`/`choose`,
    /// `replay`, or `yield`. A `fn` is statically effect-free.
    FnBodyNotPure { name: String, what: String },

    /// A malformed `parallel:` arm (slice 46e).
    ParallelArmInvalid { arm: String, message: String },

    /// A misused conversation-history parameter (slice 46c).
    PromptHistoryInvalid { prompt: String, message: String },

    /// A model declaration field with an out-of-range value
    /// (slice 46a): `temperature: 9.5`.
    ModelFieldInvalid {
        model: String,
        field: String,
        message: String,
    },

    /// An `identity Name:` block with an invalid configuration
    /// (slice 51g): no providers, a duplicate provider, an unsafe
    /// cookie option without the explicit opt-out, or a malformed
    /// OIDC discovery URL.
    IdentityConfigInvalid { identity: String, message: String },

    /// An `identity Name:` block declares OAuth providers but does not
    /// declare a first-login provisioning policy (slice 52e). There is
    /// NO silent default: a program must not back into open-registration
    /// (or any other tenancy posture) by omission, so the missing
    /// decision is a compile error naming the allowed values.
    FirstLoginPolicyRequired { identity: String },

    /// A route carries a `requires authenticated|role|permission`
    /// policy (slice 51h) but the program declares no `identity`
    /// block, so there is no way to authenticate a caller.
    RoutePolicyWithoutIdentity { server: String, path: String },

    /// A route `requires role("x")` / `requires permission("p")` names an
    /// authority (`kind` = "role" or "permission") that the identity
    /// block's `roles:` never declares (slice 52f) — the requirement
    /// could never be satisfied, so it is almost certainly a typo.
    RoutePolicyUndeclaredAuthority {
        kind: &'static str,
        name: String,
        path: String,
    },

    /// Executing a route can reach an `approve` boundary, but the
    /// route does not require an authenticated caller. Without a
    /// verified requester there is no honest actor or tenant to put
    /// on the durable approval record.
    RouteApprovalRequiresAuthentication { path: String },

    /// An approval-capable route omitted or contradicted its complete
    /// source-declared queue/reviewer policy.
    RouteApprovalPolicyInvalid { path: String, message: String },

    /// A route-level `@upload(...)` policy is missing, incomplete, or
    /// attached to a route whose body is not `Upload<Format>`.
    RouteUploadPolicyInvalid { path: String, message: String },
    /// A typed path placeholder, `{board: Int}`, of a type a path
    /// segment cannot be read as.
    RoutePathParamType { path: String, param: String, ty: String },
    /// An `action` whose `requires` names authorities, with no
    /// `identity` block to authenticate against (B10).
    ActionPolicyWithoutIdentity { action: String },
    /// An `action` requiring a role or permission the identity block
    /// does not declare (B10).
    ActionPolicyUndeclaredAuthority { kind: &'static str, name: String, action: String },
    /// A route declared at the path an action is served at.
    ActionPathTaken { action: String, path: String },
    /// An action reads its caller (an `Actor` parameter) but requires no
    /// sign-in, so no caller is ever verified.
    ActionCallerWithoutSignIn { action: String, param: String },
    /// An `Actor` where a request, a client or a model supplies the value.
    ActorFromInput { place: String, at: String },
    /// A route's `query` record with a field the server cannot read
    /// from a query string; `field` is empty when the query type is not
    /// a record at all.
    RouteQueryFieldType { path: String, field: String, ty: String },
    /// `try ... else <fallback>` where the fallback's type is not the
    /// body's.
    TryElseFallbackType { expected: String, got: String },
    /// `try ... else` around a connector operation that declares
    /// `on failure`: its outages already arrive as a value.
    TryElseAroundOnFailure { operation: String, variant: String },
    InvalidForm { message: String },
    InvalidLiveBinding { message: String },

    /// A `connector Name:` block with an invalid configuration (slice
    /// 52g): a non-absolute `base_url`, a malformed reliability value, a
    /// duplicate operation, a path placeholder naming no parameter, etc.
    ConnectorConfigInvalid { connector: String, message: String },

    /// A malformed `@retry(...)` / `@idempotency(...)` annotation
    /// (slice 45q): zero attempts, key naming no parameter, or a
    /// key parameter of non-derivable type.
    AnnotationInvalid { annotation: String, message: String },

    /// A generic type application whose head is not a known
    /// generic type (slice 45q): `Lst<Int>`, `option<T>`. Unknown
    /// heads used to silently resolve to `Type::Unknown`.
    UnknownGenericHead {
        name: String,
        suggestion: Option<String>,
    },

    /// A malformed named struct literal or destructuring pattern
    /// (slice 45n): unknown/duplicate/missing fields, non-record
    /// targets, bare `..` in expression position.
    StructLiteralInvalid { type_name: String, message: String },

    /// An agent body used `yield` without declaring `Stream<T>`.
    YieldRequiresStreamReturn { declared: String },

    /// A yielded value did not match the agent's declared `Stream<T>`.
    YieldReturnTypeMismatch { expected: String, got: String },

    /// The headline error: a `dangerous` tool was called without a matching
    /// prior `approve` in the same block.
    UnapprovedDangerousCall {
        tool: String,
        /// The `approve` label the user should have written (PascalCase).
        expected_approve_label: String,
        arity: usize,
    },

    /// A tool whose composed effect row carries `trust:
    /// supervisor_required` or `trust: human_required` was called
    /// without a matching prior `approve` — the approve requirement
    /// is DERIVED from the trust tier, same machinery as
    /// `dangerous`. The variant names the deriving effect and tier
    /// so the author can see which declaration created the
    /// obligation.
    UnapprovedHighTrustCall {
        tool: String,
        /// The `approve` label the user should have written (PascalCase).
        expected_approve_label: String,
        arity: usize,
        /// The effect in the tool's row whose trust tier derives the
        /// requirement.
        deriving_effect: String,
        /// The trust tier (`supervisor_required` or `human_required`).
        trust_tier: String,
    },

    /// An externally reachable route, schedule, or exported agent can reach a
    /// dangerous tool without a matching approval contract in lexical scope.
    ApprovalReachabilityViolation {
        entrypoint: String,
        tool: String,
        expected_approve_label: String,
        arity: usize,
    },

    /// A dimensional effect constraint was violated.
    EffectConstraintViolation {
        agent: String,
        dimension: String,
        message: String,
    },

    /// `assert <expr>` inside an eval must typecheck to Bool.
    AssertNotBool { got: String },

    /// `assert called <tool>` or `assert called A before B` references
    /// a name that does not resolve to a known callable.
    EvalUnknownTool { name: String },

    /// `assert approved <label>` references an approval label that does
    /// not match any dangerous tool label in the file.
    EvalUnknownApproval { label: String },

    /// Statistical assertion modifiers must stay in range.
    InvalidConfidence { value: f64 },

    /// An agent returns `Grounded<T>` but the compiler cannot prove a
    /// provenance path from a `data: grounded` source feeds into the
    /// return value.
    UngroundedReturn { agent: String, message: String },

    /// A prompt-level `cites <param> strictly` clause names a parameter
    /// that does not exist on the prompt.
    PromptCitationUnknownParam { prompt: String, param: String },

    /// A prompt-level `cites <param> strictly` clause can only cite a
    /// `Grounded<T>` parameter, because runtime citation verification
    /// must be tied to a compile-time provenance-carrying context.
    PromptCitationRequiresGrounded {
        prompt: String,
        param: String,
        got: String,
    },

    /// An agent marked `@replayable` calls a function that
    /// introduces nondeterminism the trace schema cannot capture.
    /// See `crate::determinism` for the catalog of nondeterministic
    /// builtins and `docs/phases/phase-21-determinism-sources.md` for
    /// which trace events must capture each source.
    NonReplayableCall {
        agent: String,
        call: String,
        source_label: String,
    },

    /// An agent marked `@deterministic` calls something the
    /// compiler cannot prove is pure: an LLM prompt, a tool, an
    /// approve block, a catalog-registered nondeterministic
    /// builtin, or another agent not itself marked
    /// `@deterministic`. `@deterministic` is strictly stronger
    /// than `@replayable` — it requires the agent body to be a
    /// pure function of its parameters, trace or no trace.
    NonDeterministicCall {
        agent: String,
        call: String,
        /// What the callee is: `"prompt"`, `"tool"`,
        /// `"approve"`, `"agent"` (for a non-`@deterministic`
        /// agent), or one of the catalog-source labels.
        call_kind: String,
    },

    /// An agent marked `@grounded_pure` (Provenance Propagation
    /// D6) launders a `Grounded<T>` value somewhere in its body
    /// — either via a silent `Grounded<T> -> T` coercion (slice 7
    /// recorded site), an explicit `.unwrap_discarding_sources()`
    /// call, or a transitive call into a non-`@grounded_pure`
    /// agent that might launder internally. The proof obligation
    /// composes through the call graph the same way
    /// `@deterministic` does.
    GroundedPureLaundering {
        agent: String,
        /// How the laundering surfaces: `"implicit_coercion"`
        /// (a silent `Grounded<T> -> T` at a slot-check site),
        /// `"explicit_unwrap"` (`.unwrap_discarding_sources()`),
        /// or `"non_grounded_pure_call"` (a call to an agent
        /// not itself marked `@grounded_pure`).
        kind: String,
        /// Human-readable target — the callee name for `_call`
        /// kinds, or the type that was coerced for `_coercion`
        /// / `_unwrap` kinds.
        target: String,
    },

    /// A custom dimension declared in `corvid.toml` under
    /// `[effect-system.dimensions.*]` failed validation. Rejects
    /// unknown composition rules, unknown value-types, malformed
    /// defaults, and collisions with built-in dimension names.
    InvalidCustomDimension { dimension: String, message: String },
    /// Slice 50i — UNTRUSTED content (a `data: untrusted` source or
    /// a prompt output derived from one) reached an
    /// approval-requiring call's arguments. The injection sink rule.
    TaintedDangerousArgument {
        tool: String,
        argument_index: usize,
        arg_type: String,
    },
    /// Slice 50j — a field refinement whose form does not fit the
    /// field's type (or whose bounds are inverted).
    RefinementInvalid {
        type_name: String,
        field: String,
        message: String,
    },
    /// B06 — a duration literal that names no exact length: a calendar
    /// unit, a fraction of a nanosecond, or a value out of range.
    DurationLiteralInvalid { message: String },
    /// B01 — a type's JSON keys (`@json(key:)` / `@json(case:)`) are
    /// empty or name one key twice.
    JsonKeyInvalid { type_name: String, message: String },
    /// B09 P2 — a query part the database cannot run.
    TableQueryNotTranslatable { message: String },
    /// B09 P3 — `renamed from` or `retired` contradicts the table's
    /// declared columns or another clause.
    TableColumnHistoryConflict { table: String, message: String },
    /// B09 P7a — something an `atomic:` block cannot hold.
    AtomicBlockRefused { message: String },
    /// B09 — a table declares no `key` field.
    TableKeyRequired { table: String },
    /// B09 — a table's key is neither `Int` nor `String`.
    TableKeyType { table: String, key: String, found: String },
    /// B09 — a table declares `revision`, which every row carries.
    TableFieldReserved { table: String, field: String },
    /// B09 — a `public` table; tables are private to their module.
    TableNotPublic { table: String },
    /// B09 — a table no `database` block lists.
    TableOutsideDatabase { table: String },
    /// B09 P11 — a table without `in` in a module with several databases.
    TableDatabaseAmbiguous { table: String, databases: String },
    /// B09 P11 — `in` names something that is not a database here.
    TableDatabaseUnknown { table: String, database: String },
    /// B09 P10 — a `unique` line that names a missing, repeated or
    /// JSON-stored column, or restates the key or another line.
    TableUniqueRule { table: String, message: String },
    /// B09 P8a — `order_by` on a column whose type has no order.
    TableOrderUnordered { column: String, type_name: String },
    /// B09 — a `database` block omits a setting that has no default.
    DatabaseSettingRequired { database: String, setting: String, allowed: String },
    /// B09 P4 — a database setting whose value this engine does not allow.
    DatabaseSettingInvalid { database: String, setting: String, message: String },
    /// B09 — an engine the runtime cannot run yet, or does not know.
    DatabaseEngineUnavailable { database: String, engine: String, available: String },

    /// A `route:` arm inside a prompt points at a name that is not a
    /// `model` declaration. The runtime can only dispatch to models,
    /// so the target must be a `Decl::Model`.
    RouteTargetNotModel {
        prompt: String,
        target: String,
        got_kind: String,
    },

    /// A `route:` arm's guard expression is not a Bool.
    RouteGuardNotBool { prompt: String, got: String },

    /// A backend server declared the same method/path pair more than once.
    DuplicateServerRoute {
        server: String,
        method: String,
        path: String,
    },

    /// A GET route declared a request body.
    GetRouteBody { server: String, path: String },

    /// A route declared with `HEAD` or `OPTIONS`, which the server
    /// answers itself (B05: the methods exist for connector operations).
    RouteMethodAnsweredByServer { server: String, method: String, path: String },

    /// A connector `Response` outside the program that made the call.
    ResponseOutsideCaller { place: String },

    ModelOutputFormatMismatch {
        prompt: String,
        model: String,
        required: String,
        got: Option<String>,
    },

    /// A `rollout N%` clause's percentage is outside `[0.0, 100.0]`.
    RolloutPercentOutOfRange { prompt: String, got: f64 },

    /// An `ensemble [..]` list names the same model more than once.
    /// Voting where two slots share a model degenerates to voting
    /// over fewer opinions than intended.
    EnsembleDuplicateModel { prompt: String, model: String },

    /// An `adversarial:` stage (propose / challenge / adjudicate)
    /// points at a name that is not a `prompt` declaration. Stages
    /// dispatch to prompts because the runtime chains stage outputs
    /// as positional arguments to the next stage.
    AdversarialStageNotPrompt {
        prompt: String,
        stage: String,
        target: String,
        got_kind: String,
    },

    /// An `adversarial:` stage's target prompt has the wrong number
    /// of parameters for its position in the pipeline.
    AdversarialStageArity {
        prompt: String,
        stage: String,
        target: String,
        expected: usize,
        got: usize,
    },

    /// An `adversarial:` stage's target prompt has a parameter whose
    /// type does not match the previous stage's return type (or, for
    /// the proposer, the outer prompt's parameter type).
    AdversarialStageParamType {
        prompt: String,
        stage: String,
        target: String,
        index: usize,
        expected: String,
        got: String,
    },

    /// The `adjudicate` stage's return type does not match the outer
    /// prompt's declared return type.
    AdversarialStageReturnType {
        prompt: String,
        stage: String,
        target: String,
        expected: String,
        got: String,
    },

    /// The `adjudicate` stage's return type is not a struct, or the
    /// struct it returns has no `contradiction: Bool` field. The
    /// runtime reads this field to decide whether to emit
    /// `TraceEvent::AdversarialContradiction`.
    AdversarialAdjudicatorMissingContradictionField {
        prompt: String,
        target: String,
        got: String,
    },

    /// The `<expr>` in `replay <expr>:` didn't evaluate to
    /// `TraceId` (or `String`, which coerces to `TraceId` inside a
    /// replay context).
    ReplayTraceNotATraceId { got: String },

    /// A replay arm body's type doesn't match the first arm's type,
    /// so the replay expression can't have a single result type.
    /// `context` points at whether this arm is a `when` or `else`.
    ReplayArmTypeMismatch {
        expected: String,
        got: String,
        context: String,
    },

    /// `pub extern "c"` currently supports only scalar ABI types.
    NonScalarInExternC {
        agent: String,
        offender_type: String,
        position: String,
    },

    /// The compiler cannot infer a sound ownership contract for an
    /// extern-visible boundary slot without an explicit annotation.
    AmbiguousExternOwnership { agent: String, position: String },

    /// A user-declared ownership annotation disagrees with the
    /// compiler's inference for the same extern-visible boundary slot.
    ExternOwnershipMismatch {
        agent: String,
        position: String,
        declared: String,
        inferred: String,
        reason: String,
    },
}

impl TypeErrorKind {
    pub fn message(&self) -> String {
        match self {
            Self::NonExhaustiveMatch { missing } => {
                format!("non-exhaustive match: missing {missing}")
            }
            Self::LiveDeclarationInvalid { path, problem } => {
                format!("`live \"{path}\"` {problem}")
            }
            Self::RefinementOutOfRange { type_name, field, refinement, value } => {
                format!("`{type_name}.{field}` must be {refinement}, but this value is {value}")
            }
            Self::ImpossibleConflictCase { case, possible } => {
                format!("this conflict is never `{case}`: it can only be {possible}")
            }
            Self::InvalidAssignTarget { reason } => {
                format!("invalid assignment target: {reason}")
            }
            Self::ArityMismatch {
                callee,
                expected,
                got,
            } => {
                format!("wrong number of arguments to `{callee}`: expected {expected}, got {got}")
            }
            Self::TypeMismatch {
                expected,
                got,
                context,
            } => {
                if context.is_empty() {
                    format!("type mismatch: expected `{expected}`, got `{got}`")
                } else {
                    format!("type mismatch in {context}: expected `{expected}`, got `{got}`")
                }
            }
            Self::UnknownField { struct_name, field } => {
                format!("no field named `{field}` on type `{struct_name}`")
            }
            Self::InvalidQualifiedApproval => "a qualified approval must name a declared Python import function".into(),
            Self::NotCallable { got } => {
                format!("cannot call a value of type `{got}`")
            }
            Self::NotAStruct { got } => {
                format!("field access requires a struct value, got `{got}`")
            }
            Self::TypeAsValue { name } => {
                format!("`{name}` is a type, not a value")
            }
            Self::NoJsonForm { method, ty, reason, .. } => {
                format!("`{method}` is not available for `{ty}`: {reason}")
            }
            Self::RetiredJsonDecoderTool { tool, .. } => format!(
                "tool `{tool}` follows the retired `decode_<Type>_from_json` convention; the runtime no longer decodes JSON for it"
            ),
            Self::RegexPatternNotLiteral { method } => format!(
                "the pattern passed to `{method}` must be a string literal, so the compiler can check it"
            ),
            Self::InvalidRegexPattern { method, problem } => {
                format!("invalid regular expression in `{method}`: {problem}")
            }
            Self::InvalidRegexReplacement { problem, .. } => {
                format!("invalid `replace_regex` replacement: {problem}")
            }
            Self::RegexSplitMatchesEmpty { pattern } => format!(
                "`split_regex` needs a pattern that cannot match empty text, but `{pattern}` can"
            ),
            Self::InvalidDecimalLiteral { literal, problem } => {
                format!("`{literal}` is not an exact `Decimal`: {problem}")
            }
            Self::DecimalFloatMix { context } => {
                format!("`Decimal` and `Float` cannot be mixed in {context}")
            }
            Self::DecimalDivisionNeedsRounding => {
                "dividing `Decimal` values must say how to round the result".into()
            }
            Self::DecimalPlacesOutOfRange { places } => {
                format!("a `Decimal` rounds to between 0 and 28 places, not {places}")
            }
            Self::DecimalDivisionByZero => "this `Decimal` division is by zero".into(),
            Self::UnknownCurrencyOrLocale { problem } => problem.clone(),
            Self::UnknownTimeZone { name, .. } => {
                format!("`{name}` is not a time zone in the IANA database")
            }
            Self::TimeOperatorMismatch { left, right, .. } => {
                format!("this operator does not combine `{left}` and `{right}`")
            }
            Self::BytesOperatorMismatch { left, right, .. } => {
                format!("this operator does not combine `{left}` and `{right}`")
            }
            Self::SecretOperatorMismatch { left, right, .. } => {
                format!("this operator does not combine `{left}` and `{right}`")
            }
            Self::SecretMethod { method } => {
                format!("a `Secret` has no method `{method}`: its text can never be read back or turned into other text")
            }
            Self::SecretKeyPrefixNotLiteral => {
                "the prefix passed to `decode_base64_after` must be a string literal".into()
            }
            Self::HmacKeyType { method, got } => {
                format!("the key of `{method}` must be `Bytes` or a `Secret`, not `{got}`")
            }
            Self::SecretAtBoundary { boundary, at, .. } => {
                format!("{boundary} cannot hold a `Secret`{at}: a secret's text never leaves the program")
            }
            Self::SecretTableColumn { column, at } => {
                format!("table column `{column}` cannot hold a `Secret`{at}: a stored secret could be read back and shown")
            }
            Self::SecretShownToPerson { builtin, at } => match builtin.as_str() {
                "ask" => format!("`ask` cannot answer with a `Secret`{at}: a person's answer is recorded in the trace"),
                _ => format!("`{builtin}` cannot offer a `Secret`{at}: its options are shown to a person"),
            },
            Self::BareFunctionReference { name } => {
                format!("`{name}` is a function; call it with `()` to use its result")
            }
            Self::InvalidTryPropagate { got } => {
                format!("`?` can only be used on `Result` or `Option`, got `{got}`")
            }
            Self::TryPropagateReturnMismatch { expected, got } => {
                format!(
                    "`?` return context mismatch: expected enclosing return type `{expected}`, got `{got}`"
                )
            }
            Self::InvalidRetryTarget { got } => {
                format!("`try ... on error retry ...` can only be used on `Result` or `Option`, got `{got}`")
            }
            Self::GenericArityMismatch {
                name,
                expected,
                got,
            } => {
                format!(
                    "wrong number of type arguments for `{name}`: expected {expected}, got {got}"
                )
            }
            Self::InvalidWeakTargetType { got } => {
                format!("`Weak<T>` requires a heap-backed target type, got `{got}`")
            }
            Self::InvalidWeakNewTarget { got } => {
                format!("`Weak::new(...)` requires a heap-backed strong value, got `{got}`")
            }
            Self::InvalidWeakUpgradeTarget { got } => {
                format!("`Weak::upgrade(...)` requires a `Weak<T>` value, got `{got}`")
            }
            Self::WeakUpgradeAcrossEffects { effects } => {
                format!(
                    "`Weak::upgrade(...)` is not provably valid here: {} may have invalidated this weak since its last refresh",
                    effects.join(", ")
                )
            }
            Self::ReturnTypeMismatch { expected, got } => {
                format!("return type mismatch: declared `{expected}`, but the body returns `{got}`")
            }
            Self::DirectPythonCallUnsupported { module, member } => {
                format!("Python function `{module}.{member}` has no declared signature")
            }
            Self::CorvidImportNotYetResolved { alias, name } => {
                format!(
                    "qualified type `{alias}.{name}` requires cross-file \
                     resolution, which is not yet implemented \
                     (lang-cor-imports-basic-resolve); the grammar parses \
                     cleanly but the module-loader + qualified-name lookup \
                     has not shipped"
                )
            }
            Self::UnknownImportAlias { alias } => {
                format!("no import alias `{alias}` in scope; check the `import` statements at the top of the file")
            }
            Self::ImportedDeclIsPrivate { alias, name } => {
                format!(
                    "declaration `{name}` in the module imported as `{alias}` is private; \
                     mark it with `public` in the imported file to make it importable"
                )
            }
            Self::UnknownImportMember { alias, name } => {
                format!("the module imported as `{alias}` has no declaration named `{name}`")
            }
            Self::YieldOutsideAgent => "`yield` is only allowed inside agent bodies".into(),
            Self::LoopFlowOutsideLoop { keyword } => {
                format!("`{keyword}` is only valid inside a `for` or `while` loop")
            }
            Self::ParallelArmInvalid { arm, message } => {
                format!("invalid parallel arm `{arm}`: {message}")
            }
            Self::PromptHistoryInvalid { prompt, message } => {
                format!("invalid history parameter on prompt `{prompt}`: {message}")
            }
            Self::ModelFieldInvalid {
                model,
                field,
                message,
            } => {
                format!("invalid field `{field}` on model `{model}`: {message}")
            }
            Self::FnBodyNotPure { name, what } => {
                format!("`fn {name}` must be effect-free, but its body contains {what}")
            }
            Self::AnnotationInvalid { annotation, message } => {
                format!("invalid `@{annotation}(...)`: {message}")
            }
            Self::IdentityConfigInvalid { identity, message } => {
                format!("invalid `identity {identity}`: {message}")
            }
            Self::FirstLoginPolicyRequired { identity } => {
                format!(
                    "E5210 First-login policy required: identity `{identity}` declares OAuth \
                     providers but does not state how an unknown verified subject is \
                     provisioned. Add: provisioning: first_login: open | invited"
                )
            }
            Self::RoutePolicyWithoutIdentity { server, path } => {
                format!(
                    "route `{path}` in server `{server}` uses `requires ...` but the program declares no `identity` block to authenticate against"
                )
            }
            Self::RoutePolicyUndeclaredAuthority { kind, name, path } => {
                format!(
                    "route `{path}` requires {kind} `{name}`, but no `{kind}` named `{name}` is declared in the identity block's `roles:` — the requirement could never be satisfied"
                )
            }
            Self::RouteApprovalRequiresAuthentication { path } => {
                format!(
                    "route `{path}` can request approval but does not require an authenticated caller"
                )
            }
            Self::RouteApprovalPolicyInvalid { path, message } => {
                format!("invalid approval policy on route `{path}`: {message}")
            }
            Self::InvalidLiveBinding { message } => format!("invalid live binding: {message}"),
            Self::InvalidForm { message } => format!("invalid form: {message}"),
            Self::RouteUploadPolicyInvalid { path, message } => {
                format!("invalid upload policy on route `{path}`: {message}")
            }
            Self::ActionPolicyWithoutIdentity { action } => format!(
                "action `{action}` requires a signed-in caller, but the program declares no `identity` block to authenticate against"
            ),
            Self::ActionPolicyUndeclaredAuthority { kind, name, action } => format!(
                "action `{action}` requires {kind} `{name}`, but no `{kind}` named `{name}` is declared in the identity block's `roles:` - the requirement could never be satisfied"
            ),
            Self::ActionPathTaken { action, path } => {
                format!("route `{path}` takes the path action `{action}` is served at")
            }
            Self::ActionCallerWithoutSignIn { action, param } => format!(
                "action `{action}` reads its caller as `{param}`, but `requires anyone` verifies no caller"
            ),
            Self::ActorFromInput { place, at } => {
                format!("an `Actor` cannot be part of {place}{at}: the sender could name any caller")
            }
            Self::RoutePathParamType { path, param, ty } => {
                format!("path parameter `{param}` of `{path}` is `{ty}`, which a path segment cannot be read as")
            }
            Self::TryElseFallbackType { expected, got } => {
                format!("the `else` fallback is `{got}`, but the `try` expression is `{expected}`")
            }
            Self::TryElseAroundOnFailure { operation, variant } => format!(
                "`try ... else` around `{operation}`, which declares `on failure -> {variant}`: its outages already arrive as `Err({variant}(...))`, so there is nothing to catch"
            ),
            Self::RouteQueryFieldType { path, field, ty } if field.is_empty() => {
                format!("the query of `{path}` is `{ty}`; a query is a record whose fields are the query parameters")
            }
            Self::RouteQueryFieldType { path, field, ty } => {
                format!("query field `{field}` of `{path}` is `{ty}`, which a query string cannot carry")
            }
            Self::ConnectorConfigInvalid { connector, message } => {
                format!("invalid `connector {connector}`: {message}")
            }
            Self::UnknownGenericHead { name, suggestion } => match suggestion {
                Some(s) => format!("`{name}` is not a generic type — did you mean `{s}`?"),
                None => format!("`{name}` is not a generic type"),
            },
            Self::AliasCycle { name } => {
                format!("type alias `{name}` never reaches a concrete type (alias cycle)")
            }
            Self::StructLiteralInvalid { type_name, message } => {
                format!("invalid `{type_name} {{ ... }}`: {message}")
            }
            Self::YieldRequiresStreamReturn { declared } => {
                format!(
                    "`yield` requires the enclosing agent to declare `Stream<T>`, got `{declared}`"
                )
            }
            Self::YieldReturnTypeMismatch { expected, got } => {
                format!("yield type mismatch: expected `{expected}`, got `{got}`")
            }
            Self::UnapprovedDangerousCall { tool, .. } => {
                format!("dangerous tool `{tool}` called without a prior `approve`")
            }
            Self::UnapprovedHighTrustCall {
                tool,
                deriving_effect,
                trust_tier,
                ..
            } => {
                format!(
                    "tool `{tool}` composes `trust: {trust_tier}` (from effect `{deriving_effect}`) and was called without a prior `approve` — high-trust tools require approval exactly like `dangerous` ones"
                )
            }
            Self::ApprovalReachabilityViolation {
                entrypoint, tool, ..
            } => {
                format!(
                    "`{entrypoint}` can reach dangerous tool `{tool}` without a reachable approval contract"
                )
            }
            Self::EffectConstraintViolation { agent, message, .. } => {
                format!("effect constraint violated in agent `{agent}`: {message}")
            }
            Self::AssertNotBool { got } => {
                format!("eval assertions must be `Bool`, got `{got}`")
            }
            Self::EvalUnknownTool { name } => {
                format!("eval trace assertion references unknown callable `{name}`")
            }
            Self::EvalUnknownApproval { label } => {
                format!("eval trace assertion references unknown approval label `{label}`")
            }
            Self::InvalidConfidence { value } => {
                format!("statistical assertion confidence must be in [0.0, 1.0], got `{value}`")
            }
            Self::UngroundedReturn { agent, message } => {
                format!("ungrounded return in agent `{agent}`: {message}")
            }
            Self::PromptCitationUnknownParam { prompt, param } => {
                format!(
                    "prompt `{prompt}` cites unknown parameter `{param}` in `cites {param} strictly`"
                )
            }
            Self::PromptCitationRequiresGrounded { prompt, param, got } => {
                format!(
                    "prompt `{prompt}` cites `{param}` strictly, but `{param}` has type `{got}` instead of `Grounded<T>`"
                )
            }
            Self::NonReplayableCall {
                agent,
                call,
                source_label,
            } => {
                format!(
                    "agent `{agent}` is marked `@replayable` but calls `{call}`, which introduces a {source_label} that the trace schema cannot capture"
                )
            }
            Self::NonDeterministicCall {
                agent,
                call,
                call_kind,
            } => {
                format!(
                    "agent `{agent}` is marked `@deterministic` but calls `{call}` ({call_kind}), which the compiler cannot prove is a pure function of the agent's inputs"
                )
            }
            Self::GroundedPureLaundering {
                agent,
                kind,
                target,
            } => {
                let phrase = match kind.as_str() {
                    "implicit_coercion" => format!(
                        "silently coerces a `Grounded<T>` into a non-grounded slot at `{target}`"
                    ),
                    "explicit_unwrap" => format!(
                        "calls `{target}.unwrap_discarding_sources()`, which explicitly drops the provenance chain"
                    ),
                    "non_grounded_pure_call" => format!(
                        "calls `{target}`, which is not itself marked `@grounded_pure` and could launder internally"
                    ),
                    _ => format!("launders provenance at `{target}` ({kind})"),
                };
                format!(
                    "agent `{agent}` is marked `@grounded_pure` but {phrase}"
                )
            }
            Self::TaintedDangerousArgument {
                tool,
                argument_index,
                arg_type,
            } => {
                format!(
                    "argument {argument_index} to `{tool}` is `{arg_type}` — untrusted content cannot parameterize an approval-requiring call"
                )
            }
            Self::RefinementInvalid {
                type_name,
                field,
                message,
            } => {
                format!("field `{field}` on `{type_name}`: {message}")
            }
            Self::JsonKeyInvalid { type_name, message } => format!("type `{type_name}`: {message}"),
            Self::DurationLiteralInvalid { message } => message.clone(),
            Self::TableQueryNotTranslatable { message } => message.clone(),
            Self::TableColumnHistoryConflict { table, message } => format!("table `{table}`: {message}"),
            Self::AtomicBlockRefused { message } => message.clone(),
            Self::TableKeyRequired { table } => format!("table `{table}` declares no key field"),
            Self::TableKeyType { table, key, found } => {
                format!("the key `{key}` of table `{table}` is `{found}`; a key is `Int` or `String`")
            }
            Self::TableFieldReserved { table, field } => {
                format!("table `{table}` declares `{field}`, which every row already carries")
            }
            Self::TableNotPublic { table } => {
                format!("table `{table}` is `public`, but tables are private to the module that declares them")
            }
            Self::TableOutsideDatabase { table } => {
                format!("table `{table}` has no database: this module declares none, so nothing says where its rows live")
            }
            Self::TableDatabaseAmbiguous { table, databases } => {
                format!("table `{table}` does not say which database holds it, and this module declares {databases}")
            }
            Self::TableDatabaseUnknown { table, database } => {
                format!("table `{table}` is `in {database}`, but no `database {database}` is declared here")
            }
            Self::TableUniqueRule { table, message } => format!("a `unique` line in table `{table}` cannot hold: {message}"),
            Self::TableOrderUnordered { column, type_name } => {
                format!("`order_by` reads column `{column}`, a `{type_name}`, which has no order")
            }
            Self::DatabaseSettingRequired { database, setting, allowed } => {
                format!("database `{database}` does not say `{setting}`; it must be one of {allowed}")
            }
            Self::DatabaseSettingInvalid { database, setting, message } => {
                format!("database `{database}`: `{setting}` {message}")
            }
            Self::DatabaseEngineUnavailable { database, engine, available } => {
                format!("database `{database}` names engine `{engine}`; the engines that run today are {available}")
            }

            Self::InvalidCustomDimension { dimension, message } => {
                format!("invalid custom dimension `{dimension}` in corvid.toml: {message}")
            }
            Self::RouteTargetNotModel {
                prompt,
                target,
                got_kind,
            } => {
                format!(
                    "route arm in prompt `{prompt}` points at `{target}`, which is a {got_kind}, not a `model`"
                )
            }
            Self::RouteGuardNotBool { prompt, got } => {
                format!("route arm guard in prompt `{prompt}` must evaluate to `Bool`, got `{got}`")
            }
            Self::DuplicateServerRoute {
                server,
                method,
                path,
            } => {
                format!("server `{server}` declares duplicate route `{method} {path}`")
            }
            Self::GetRouteBody { server, path } => {
                format!("GET route `{path}` in server `{server}` declares a request body")
            }
            Self::ResponseOutsideCaller { place } => {
                format!("{place} cannot hold a `Response`")
            }
            Self::RouteMethodAnsweredByServer { server, method, path } => {
                format!("route `{method} {path}` in server `{server}` declares a method the server answers itself")
            }
            Self::ModelOutputFormatMismatch {
                prompt,
                model,
                required,
                got,
            } => {
                format!(
                    "prompt `{prompt}` requires output format `{required}`, but model `{model}` declares {}",
                    got.as_deref()
                        .map(|format| format!("`{format}`"))
                        .unwrap_or_else(|| "no `output_format`".to_string())
                )
            }
            Self::RolloutPercentOutOfRange { prompt, got } => {
                format!(
                    "rollout percentage on prompt `{prompt}` must be in [0.0, 100.0], got `{got}`"
                )
            }
            Self::EnsembleDuplicateModel { prompt, model } => {
                format!("ensemble on prompt `{prompt}` lists model `{model}` more than once")
            }
            Self::AdversarialStageNotPrompt {
                prompt,
                stage,
                target,
                got_kind,
            } => {
                format!(
                    "adversarial `{stage}` stage in prompt `{prompt}` points at `{target}`, which is a {got_kind}, not a `prompt`"
                )
            }
            Self::AdversarialStageArity {
                prompt,
                stage,
                target,
                expected,
                got,
            } => {
                format!(
                    "adversarial `{stage}` stage `{target}` in prompt `{prompt}` takes {got} parameter{}; expected {expected}",
                    if *got == 1 { "" } else { "s" }
                )
            }
            Self::AdversarialStageParamType {
                prompt,
                stage,
                target,
                index,
                expected,
                got,
            } => {
                format!(
                    "adversarial `{stage}` stage `{target}` in prompt `{prompt}` parameter #{index} has type `{got}`, expected `{expected}`"
                )
            }
            Self::AdversarialStageReturnType {
                prompt,
                stage,
                target,
                expected,
                got,
            } => {
                format!(
                    "adversarial `{stage}` stage `{target}` in prompt `{prompt}` returns `{got}`, expected `{expected}` to match the outer prompt's return type"
                )
            }
            Self::AdversarialAdjudicatorMissingContradictionField {
                prompt,
                target,
                got,
            } => {
                format!(
                    "adversarial `adjudicate` stage `{target}` in prompt `{prompt}` returns `{got}`, which is not a struct with a `contradiction: Bool` field"
                )
            }
            Self::ReplayTraceNotATraceId { got } => {
                format!(
                    "`replay <expr>:` expects `TraceId` (or a `String` path literal), got `{got}`"
                )
            }
            Self::ReplayArmTypeMismatch {
                expected,
                got,
                context,
            } => {
                format!(
                    "replay arm type mismatch in {context}: expected `{expected}` (matching the first arm), got `{got}`"
                )
            }
            Self::NonScalarInExternC {
                agent,
                offender_type,
                position,
            } => {
                format!(
                    "extern \"c\" agent `{agent}` uses unsupported ABI type `{offender_type}` in {position}"
                )
            }
            Self::AmbiguousExternOwnership { agent, position } => {
                format!(
                    "extern \"c\" agent `{agent}` has ambiguous ownership in {position}; annotate it explicitly"
                )
            }
            Self::ExternOwnershipMismatch {
                agent,
                position,
                declared,
                inferred,
                reason,
            } => {
                format!(
                    "extern \"c\" agent `{agent}` declares {declared} in {position}, but the compiler inferred {inferred}: {reason}"
                )
            }
        }
    }

    pub fn hint(&self) -> Option<String> {
        match self {
            Self::NonExhaustiveMatch { .. } => Some(
                "add the missing arm(s), or a final `_ -> ...` catch-all".into(),
            ),
            Self::LiveDeclarationInvalid { .. } => Some(
                "a `live` body is one `return` of a query on the table it delivers, such as `return Card.where(fn (c) -> c.board == board)`; a subscriber gets those rows, then each committed change to them".into(),
            ),
            Self::RefinementOutOfRange { .. } => Some(
                "use a value the refinement admits; every value of a refined field holds its refinement, however it is built".into(),
            ),
            Self::ImpossibleConflictCase { .. } => Some(
                "remove the arm: each table write returns only the conflicts it can produce".into(),
            ),
            Self::InvalidAssignTarget { .. } => Some(
                "assign through a path rooted at a local variable, e.g. `x.field = v` or `xs[i] = v`".into(),
            ),
            Self::ArityMismatch { callee, expected, .. } => Some(format!(
                "`{callee}` takes {expected} argument{}",
                if *expected == 1 { "" } else { "s" }
            )),
            Self::TypeMismatch { expected, got, .. } => Some(match (expected.as_str(), got.as_str()) {
                ("Decimal", "Int") => "convert with `.to_decimal()`, which is exact".into(),
                ("Decimal", "Float") => "convert with `.to_decimal()`, which keeps the digits the Float shows, or declare the value as a `Decimal` from the start".into(),
                ("Float", "Decimal") => "convert with `.to_float()`, which gives the nearest Float".into(),
                _ => format!("change the value to produce a `{expected}`, or update the signature"),
            }),
            Self::InvalidDecimalLiteral { .. } => Some(
                "a `Decimal` holds at most 38 significant digits, at most 28 of them after the point".into(),
            ),
            Self::DecimalFloatMix { .. } => Some(
                "convert one side: `x.to_decimal()` makes a Float exact at the digits it shows, and `amount.to_float()` gives the nearest Float".into(),
            ),
            Self::DecimalDivisionNeedsRounding => Some(
                "round the quotient where you divide, such as `(total / 3).round_half_even(2)`; the modes are half_even, half_up, toward_zero, away_from_zero, floor and ceiling".into(),
            ),
            Self::DecimalPlacesOutOfRange { .. } => Some(
                "places is the number of digits after the point, from 0 to 28".into(),
            ),
            Self::DecimalDivisionByZero => Some("divide by a non-zero value".into()),
            Self::UnknownTimeZone { suggestion: Some(suggestion), .. } => Some(format!("did you mean `{suggestion}`?")),
            Self::UnknownTimeZone { suggestion: None, .. } => Some(
                "use an IANA zone name such as `Europe/Berlin` or `America/New_York`; for a name chosen at run time, use `parse_time_zone()`".into(),
            ),
            Self::TimeOperatorMismatch { hint, .. } => Some(hint.clone()),
            Self::BytesOperatorMismatch { hint, .. } => Some(hint.clone()),
            Self::SecretOperatorMismatch { hint, .. } => Some(hint.clone()),
            Self::SecretMethod { .. } => Some(
                "compare two secrets with `==`, key an HMAC with one (`body.utf8().hmac_sha256(key)`), check one against a stored hash (`password.verify_password(hash)`), or pass the secret to the tool that needs it; the tool receives the text and its trace records `<redacted>`".into(),
            ),
            Self::SecretKeyPrefixNotLiteral => Some(
                "write the provider's prefix inline, such as `key.decode_base64_after(\"whsec_\")`; a prefix computed at run time could test the secret's text one guess at a time".into(),
            ),
            Self::HmacKeyType { .. } => Some(
                "read a signing key with `secret(name)` from std/secrets, so it never reaches a trace; a key the program holds as bytes, such as `Bytes.from_hex(text)?`, works too".into(),
            ),
            Self::SecretAtBoundary { hint, .. } => Some(hint.clone()),
            Self::SecretTableColumn { .. } => Some(
                "never store a secret's text. To check a value later, store its hash, `hash_password(value, owasp_2023())` from std/crypto, in a `String` column and check it with `value.verify_password(hash)`; to use the value again it needs an encrypted column, which is not built yet (tracked in B09)".into(),
            ),
            Self::SecretShownToPerson { .. } => Some(
                "read secrets from the environment with `secret(name)` from std/secrets; never ask a person to type one or show them one".into(),
            ),
            Self::UnknownCurrencyOrLocale { .. } => Some(
                "currencies are ISO 4217 codes such as `USD` and `EUR`, and locales are BCP 47 tags such as `en-US` and `de-DE`; for values chosen at run time, use `parse_currency()` or `parse_locale()`".into(),
            ),
            Self::UnknownField { struct_name, field } => Some(format!(
                "check the declaration of `{struct_name}` for the correct field name (you wrote `{field}`)"
            )),
            Self::InvalidQualifiedApproval => Some("use `approve alias.member(args)` with the same import alias and function as the call".into()),
            Self::NotCallable { .. } => Some(
                "only tools, agents, prompts, and imported functions can be called".into(),
            ),
            Self::NotAStruct { .. } => {
                Some("use `.field` only on values of a user-declared `type`".into())
            }
            Self::NoJsonForm { hint, .. } => Some(hint.clone()),
            Self::RetiredJsonDecoderTool { replacement, .. } => Some(replacement.clone()),
            Self::RegexPatternNotLiteral { .. } => Some(
                "write the pattern inline, such as `text.find_regex(\"[0-9]+\")`; to search for text computed at run time, use `contains`, which needs no escaping".into(),
            ),
            Self::InvalidRegexPattern { .. } => Some(
                "patterns use Rust `regex` syntax; backslashes are doubled inside Corvid strings, as in `\"\\\\d+\"`".into(),
            ),
            Self::InvalidRegexReplacement { hint, .. } => Some(hint.clone()),
            Self::RegexSplitMatchesEmpty { .. } => Some(
                "require at least one character, for example `\",\\\\s*\"` instead of `\"\\\\s*\"`".into(),
            ),
            Self::TypeAsValue { name } => Some(format!(
                "to create a value of type `{name}`, call a tool or prompt that returns one"
            )),
            Self::BareFunctionReference { name } => {
                Some(format!("did you mean `{name}(...)` ?"))
            }
            Self::InvalidTryPropagate { .. } => Some(
                "apply `?` only to `Result<T, E>` or `Option<T>` values".into(),
            ),
            Self::TryPropagateReturnMismatch { expected, .. } => Some(format!(
                "change the enclosing return type to `{expected}`, or remove `?`"
            )),
            Self::InvalidRetryTarget { .. } => Some(
                "apply retry only to `Result<T, E>` or `Option<T>` expressions".into(),
            ),
            Self::GenericArityMismatch { name, expected, .. } => Some(format!(
                "`{name}` requires {expected} type argument{}",
                if *expected == 1 { "" } else { "s" }
            )),
            Self::InvalidWeakTargetType { .. } => Some(
                "use `Weak<T>` only with heap-backed types like String, user-declared types, or List<T>".into(),
            ),
            Self::InvalidWeakNewTarget { .. } => Some(
                "pass a String, user-declared type, or List<T> value to `Weak::new(...)`".into(),
            ),
            Self::InvalidWeakUpgradeTarget { .. } => Some(
                "call `Weak::upgrade(...)` only on a value whose type is `Weak<T>`".into(),
            ),
            Self::WeakUpgradeAcrossEffects { effects } => Some(format!(
                "refresh the weak with a new `Weak::new(...)` or an earlier `Weak::upgrade(...)`, and avoid `{}` on every path before this call",
                effects.join(", ")
            )),
            Self::ReturnTypeMismatch { expected, .. } => Some(format!(
                "change the final `return` to produce a `{expected}`, or update the declared return type"
            )),
            Self::DirectPythonCallUnsupported { .. } => Some(
                "declare a bodyless `fn` signature in the indented Python import block".into(),
            ),
            Self::CorvidImportNotYetResolved { .. } => Some(
                "for now, use an unqualified local type or wait for cross-file Corvid import resolution to land"
                    .into(),
            ),
            Self::UnknownImportAlias { alias } => Some(format!(
                "add `import \"./path\" as {alias}` at the top of the file (or fix the alias spelling)"
            )),
            Self::ImportedDeclIsPrivate { name, .. } => Some(format!(
                "in the imported file, add `public` before `{name}`'s declaration to export it"
            )),
            Self::UnknownImportMember { .. } => Some(
                "check the imported file's top-level declarations for the name you meant".into(),
            ),
            Self::LoopFlowOutsideLoop { keyword } => Some(format!(
                "move the `{keyword}` inside a loop body, or remove it"
            )),
            Self::ParallelArmInvalid { .. } => Some(
                "a parallel block runs two or more `name = call(...)` arms concurrently and joins when all complete".into(),
            ),
            Self::PromptHistoryInvalid { .. } => Some(
                "declare one `history: List<AiMessage>` parameter; its messages splice between the system blocks and the current turn".into(),
            ),
            Self::ModelFieldInvalid { .. } => Some(
                "sampling ranges: temperature 0..=2, top_p 0..=1, max_tokens a positive integer".into(),
            ),
            Self::FnBodyNotPure { .. } => Some(
                "move the effectful call into an agent, or pass its result into the fn as a parameter".into(),
            ),
            Self::AnnotationInvalid { .. } => Some(
                "see `@retry(max_attempts: N, backoff: linear|exponential MS)` and `@idempotency(key: param_name)`".into(),
            ),
            Self::UnknownGenericHead { .. } => Some(
                "the generic heads are List, Map, Option, Result, Stream, Weak, Grounded, Partial, and ResumeToken; user-defined generics are post-v1.0".into(),
            ),
            Self::AliasCycle { .. } => Some(
                "make the alias chain end at a concrete type (a record, sum, or builtin)".into(),
            ),
            Self::StructLiteralInvalid { .. } => Some(
                "named literals need every declared field (or `..base` to fill the rest); destructuring needs bare names and `..` only".into(),
            ),
            Self::YieldOutsideAgent => Some(
                "move `yield` into an `agent ... -> Stream<T>` body, or replace it with `return`".into(),
            ),
            Self::YieldRequiresStreamReturn { .. } => Some(
                "change the agent return type to `Stream<T>` matching the yielded value type".into(),
            ),
            Self::YieldReturnTypeMismatch { .. } => Some(
                "yield values whose type matches the declared `Stream<T>` element type".into(),
            ),
            Self::UnapprovedDangerousCall {
                expected_approve_label,
                arity,
                ..
            } => {
                let args = (0..*arity)
                    .map(|i| format!("arg{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(format!(
                    "add `approve {expected_approve_label}({args})` on the line before this call"
                ))
            }
            Self::UnapprovedHighTrustCall {
                expected_approve_label,
                arity,
                deriving_effect,
                trust_tier,
                ..
            } => {
                let args = (0..*arity)
                    .map(|i| format!("arg{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(format!(
                    "add `approve {expected_approve_label}({args})` on the line before this call, or lower `trust:` on effect `{deriving_effect}` if `{trust_tier}` overstates the tool's real authority requirement"
                ))
            }
            Self::ApprovalReachabilityViolation {
                expected_approve_label,
                arity,
                ..
            } => {
                let args = (0..*arity)
                    .map(|i| format!("arg{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(", ");
                Some(format!(
                    "add `approve {expected_approve_label}({args})` in the same lexical block as the dangerous call, or move the dangerous action behind a checked approval helper"
                ))
            }
            Self::EffectConstraintViolation { dimension, .. } => Some(format!(
                "relax the `@{dimension}` constraint, or remove the call that violates it"
            )),
            Self::AssertNotBool { .. } => Some(
                "make the asserted expression evaluate to `Bool`, for example by adding a comparison".into(),
            ),
            Self::EvalUnknownTool { .. } => Some(
                "declare the referenced tool, prompt, or agent before using it in `assert called ...`".into(),
            ),
            Self::EvalUnknownApproval { .. } => Some(
                "use the PascalCase approval label for a dangerous tool declared in this file".into(),
            ),
            Self::InvalidConfidence { .. } => Some(
                "use `with confidence P over N runs` with 0.0 <= P <= 1.0 and N > 0".into(),
            ),
            Self::UngroundedReturn { .. } => Some(
                "derive each return value from a grounded parameter or a tool or prompt with a grounded effect such as `uses retrieval`"
                    .into(),
            ),
            Self::PromptCitationUnknownParam { .. } => Some(
                "cite one of the prompt parameters, for example `cites ctx strictly`".into(),
            ),
            Self::PromptCitationRequiresGrounded { .. } => Some(
                "make the cited parameter `Grounded<T>` or pass a retrieval-backed value into the prompt"
                    .into(),
            ),
            Self::NonReplayableCall { call, .. } => Some(format!(
                "route `{call}` through a recorded interface (a tool, prompt, or captured builtin) \
                 so replay can substitute the recorded value, or drop the `@replayable` attribute"
            )),
            Self::NonDeterministicCall { call, call_kind, .. } => Some(format!(
                "`@deterministic` requires a pure function of the agent's inputs. Either remove \
                 the call to `{call}` ({call_kind}), mark the callee `@deterministic` if it is \
                 an agent you control, or drop the `@deterministic` attribute and use \
                 `@replayable` if replay reproducibility is enough"
            )),
            Self::GroundedPureLaundering { kind, target, .. } => Some(match kind.as_str() {
                "implicit_coercion" => format!(
                    "preserve the `Grounded<T>` wrapper at `{target}` — return / parameter / \
                     field types should match the source's grounding, not strip it. The \
                     typechecker recorded this site because the legacy `Grounded<T> -> T` \
                     rule fired silently; under `@grounded_pure` it has to be visible"
                ),
                "explicit_unwrap" => format!(
                    "remove the `{target}.unwrap_discarding_sources()` call, or drop \
                     `@grounded_pure` if the laundering is genuinely intended"
                ),
                "non_grounded_pure_call" => format!(
                    "mark `{target}` `@grounded_pure` so the compiler can prove it does not \
                     launder, or refactor to avoid calling it"
                ),
                _ => format!("see docs/meta/grounded-propagation-design.md §D6 for the moat contract"),
            }),
            Self::TaintedDangerousArgument { .. } => Some(
                "this value derives from untrusted content (a `data: untrusted` source, or a prompt that read one) — the prompt-injection sink rule refuses it here. Either constrain the value and assert the boundary explicitly with `trusted(expr)` (greppable, reviewable), or restructure so the dangerous call's arguments never touch untrusted content"
                    .into(),
            ),
            Self::TableQueryNotTranslatable { .. } => Some(
                "a query runs in the database, so its parts are limited to what SQL can do: filter with comparisons of a column and an agent value, order by one column, take a count; combine further in the agent after the query".into(),
            ),
            Self::AtomicBlockRefused { .. } => Some(
                "an `atomic:` block holds only what the database can undo: table operations, queries and pure code. Do anything else (a tool, a model call, an approval) before or after the block, and finish the block before returning".into(),
            ),
            Self::TableColumnHistoryConflict { .. } => Some(
                "`renamed from` names the column a field was stored as before, and `retired` names stored columns the table no longer has; each stored name appears once, and never as a declared field".into(),
            ),
            Self::TableKeyRequired { .. } => Some(
                "mark the field that identifies a row, for example `key shelf: Int`; rows are saved by key with a revision check".into(),
            ),
            Self::TableKeyType { .. } => Some("declare the key as `Int` or `String`".into()),
            Self::TableFieldReserved { .. } => Some(
                "the database maintains `revision` and advances it on every write; give the field another name".into(),
            ),
            Self::TableNotPublic { .. } => Some(
                "remove `public`; another module reaches the rows through an agent this module exports".into(),
            ),
            Self::TableOutsideDatabase { .. } => Some(
                "declare a `database` block in this module, and its tables live there".into(),
            ),
            Self::TableDatabaseAmbiguous { .. } => Some("write `table Name in <database>:` with one of them".into()),
            Self::TableDatabaseUnknown { .. } => Some("declare the database in this module, or name one that is".into()),
            Self::TableUniqueRule { .. } => Some(
                "write `unique email` for one column, or `unique tenant, slug` for columns that must not repeat together".into(),
            ),
            Self::TableOrderUnordered { type_name, .. } if type_name == "ZonedTime" => {
                Some("order by an `Instant` column instead: two zoned times in different zones can be the same instant".into())
            }
            Self::TableOrderUnordered { .. } => Some("order by a column of a type with an order, such as a number, text or an `Instant`".into()),
            Self::DatabaseSettingRequired { .. } => Some(
                "where data lives is a decision the source states: write the setting in the `database` block".into(),
            ),
            Self::DatabaseSettingInvalid { .. } => Some(
                "a Postgres connection string usually holds a password, so it comes from `env(\"NAME\")`, and its encryption is stated with `tls: required` or `tls: disabled`; a SQLite database is a file and has no connection to encrypt".into(),
            ),
            Self::DatabaseEngineUnavailable { .. } => Some(
                "use `engine: sqlite`; Postgres and MySQL are on the roadmap (B09 P4 and P5)".into(),
            ),
            Self::DurationLiteralInvalid { .. } => Some(
                "a duration is a number and one of nanoseconds, microseconds, milliseconds, seconds, minutes or hours, such as `30 seconds` or `1.5 hours`".into(),
            ),
            Self::JsonKeyInvalid { .. } => Some(
                "give each field its own non-empty key with `@json(key: \"...\")`; a field's key replaces the one its type's `@json(case:)` derives".into(),
            ),
            Self::RefinementInvalid { .. } => Some(
                "`between(min, max)` refines Int fields; `len_between(min, max)` refines                  String fields; bounds are inclusive and min must not exceed max"
                    .into(),
            ),
            Self::InvalidCustomDimension { .. } => Some(
                "see docs/internals/effect-spec/01-dimensional-syntax.md §4 for the supported \
                 composition rules, value types, and default-value shapes"
                    .into(),
            ),
            Self::RouteTargetNotModel { target, .. } => Some(format!(
                "declare `{target}` as a `model ...:` block, or route to an existing model"
            )),
            Self::RouteGuardNotBool { .. } => Some(
                "use a comparison or boolean expression for the guard, e.g. `length(q) > 1000`"
                    .into(),
            ),
            Self::DuplicateServerRoute { method, path, .. } => Some(format!(
                "keep exactly one handler for `{method} {path}`, or change one route's method/path"
            )),
            Self::GetRouteBody { .. } => Some(
                "move request data for GET into a `query Type` clause, or change the method to POST/PUT/PATCH".into(),
            ),
            Self::ResponseOutsideCaller { .. } => Some(
                "a `Response` is what a connector call answered: read `status()`, `header(name)` and `body()` where the call is made, and pass on the values you need".into(),
            ),
            Self::RouteMethodAnsweredByServer { .. } => Some(
                "`corvid serve` answers HEAD from the route's GET and OPTIONS from its routes and CORS policy; declare the GET route. HEAD and OPTIONS are for connector operations".into(),
            ),
            Self::ModelOutputFormatMismatch { required, .. } => Some(format!(
                "add `output_format: {required}` to the target `model` declaration, or route this prompt to a model that supports that response format"
            )),
            Self::RolloutPercentOutOfRange { .. } => Some(
                "use a percentage between 0 and 100, e.g. `rollout 10% new_v2, else old_v1`"
                    .into(),
            ),
            Self::EnsembleDuplicateModel { .. } => Some(
                "list each ensemble model at most once; dispatch to distinct providers to get independent votes"
                    .into(),
            ),
            Self::AdversarialStageNotPrompt { target, .. } => Some(format!(
                "declare `{target}` as a `prompt ...` with the right signature, or point the stage at an existing prompt"
            )),
            Self::AdversarialStageArity { stage, expected, .. } => Some(match stage.as_str() {
                "propose" => format!(
                    "the `propose` stage runs first; it must accept the same {expected} parameter(s) as the outer prompt"
                ),
                "challenge" => "the `challenge` stage must accept exactly 1 parameter: the proposer's return value".into(),
                "adjudicate" => "the `adjudicate` stage must accept exactly 2 parameters: the proposer's return value followed by the challenger's return value".into(),
                _ => format!("change the stage's arity to {expected}"),
            }),
            Self::AdversarialStageParamType { stage, index, expected, .. } => Some(match stage.as_str() {
                "propose" => format!(
                    "parameter #{index} must match the outer prompt's parameter at the same position; expected `{expected}`"
                ),
                "challenge" => format!(
                    "the `challenge` stage's parameter must accept the proposer's return type `{expected}`"
                ),
                "adjudicate" if *index == 0 => format!(
                    "the `adjudicate` stage's first parameter must accept the proposer's return type `{expected}`"
                ),
                "adjudicate" => format!(
                    "the `adjudicate` stage's second parameter must accept the challenger's return type `{expected}`"
                ),
                _ => format!("change the parameter type to `{expected}`"),
            }),
            Self::AdversarialStageReturnType { expected, .. } => Some(format!(
                "change the `adjudicate` stage's return type to `{expected}`, or update the outer prompt's return type to match"
            )),
            Self::AdversarialAdjudicatorMissingContradictionField { .. } => Some(
                "declare a `type` with at least `contradiction: Bool` and return it from the adjudicator — the runtime reads this field to decide whether to emit an adversarial contradiction trace event"
                    .into(),
            ),
            Self::ReplayTraceNotATraceId { .. } => Some(
                "pass either a `String` path literal like `\"run.jsonl\"` or a value of type `TraceId`"
                    .into(),
            ),
            Self::ReplayArmTypeMismatch { expected, .. } => Some(format!(
                "every arm (including `else`) of a replay block must produce the same type; adjust the arm body to return `{expected}`"
            )),
            Self::NonScalarInExternC { .. } => Some(
                "extern \"c\" accepts Int/Float/Bool/String parameters, scalar / `Grounded<scalar>` / `Nothing` returns, and user-declared structs whose fields are themselves all scalars (Int/Float/Bool/String). Nested structs, Lists, Options, and other rich types still wait for follow-up FFI slices. Boundary structs travel the C ABI as JSON-encoded strings; see docs/reference/exported-abi.md".into(),
            ),
            Self::AmbiguousExternOwnership { .. } => Some(
                "add an explicit ownership annotation such as `@owned`, `@borrowed`, `@shared`, or `@static` after the boundary type".into(),
            ),
            Self::ExternOwnershipMismatch { inferred, .. } => Some(format!(
                "either change the annotation to `{inferred}` or change the implementation so the inferred ownership matches the declared contract"
            )),
            Self::IdentityConfigInvalid { .. } => Some(
                "declare at least one `provider`; keep cookies `secure`+`http_only` and `same_site: strict|lax`. An unsafe cookie choice requires an explicit `insecure_opt_out: true`.".into(),
            ),
            Self::RoutePolicyWithoutIdentity { .. } => Some(
                "add an `identity <name>:` block declaring the sign-in providers; the auth routes and the typed `actor` derive from it".into(),
            ),
            Self::RoutePolicyUndeclaredAuthority { kind, name, .. } => Some(format!(
                "declare it in the identity block's `roles:` — e.g. a role granting `{name}`, or a role named `{name}` (for a `requires {kind}`)"
            )),
            Self::FirstLoginPolicyRequired { .. } => Some(
                "add a `provisioning:` sub-block, e.g. `provisioning: first_login: open` with `tenant: fixed(\"public\")` for public signup, or `first_login: invited` with `tenant: from_invitation` for invite-gated access. There is no default — the posture must be stated.".into(),
            ),
            Self::RouteApprovalRequiresAuthentication { .. } => Some(
                "add `requires authenticated` to the route; Corvid records the verified actor and tenant as the approval requester and never substitutes an anonymous global identity".into(),
            ),
            Self::RouteApprovalPolicyInvalid { .. } => Some(
                "add a complete `@approval(role: \"...\", risk: \"...\", data: \"...\", expires_ms: N, max_cost_usd: $N, irreversible: true|false)` before the route; the role must be declared and able to decide approvals".into(),
            ),
            Self::InvalidForm { .. } | Self::InvalidLiveBinding { .. } => None,
            Self::TryElseFallbackType { .. } => Some(
                "give the fallback the expression's type: the form evaluates to one or the other".into(),
            ),
            Self::TryElseAroundOnFailure { variant, .. } => Some(format!(
                "drop the `try` and match the result: `Err({variant}(reason)) -> ...`"
            )),
            Self::ActionPolicyWithoutIdentity { .. } => Some(
                "add an `identity <name>:` block declaring the sign-in providers, or write `requires anyone` for an action every caller may use".into(),
            ),
            Self::ActionPolicyUndeclaredAuthority { kind, name, .. } => Some(format!(
                "declare it in the identity block's `roles:`, e.g. a role granting `{name}`, or a role named `{name}` (for a `requires {kind}`)"
            )),
            Self::ActionCallerWithoutSignIn { .. } => Some(
                "require a signed-in caller, such as `requires authenticated`, or remove the `Actor` parameter".into(),
            ),
            Self::ActorFromInput { .. } => Some(
                "an `Actor` comes only from the verified session: give an action an `Actor` parameter of its own, or read `actor` in a route that requires a sign-in".into(),
            ),
            Self::ActionPathTaken { .. } => Some(
                "every action is served at `POST /actions/<name>`; give the route another path, or call the action from it".into(),
            ),
            Self::RoutePathParamType { .. } => Some(format!(
                "a path parameter is one of {}",
                crate::text_parameters::TEXT_TYPES
            )),
            Self::RouteQueryFieldType { .. } => Some(format!(
                "a query field is one of {}, an `Option` of one (left out when absent) or a `List` of one (the name repeated)",
                crate::text_parameters::TEXT_TYPES
            )),
            Self::RouteUploadPolicyInvalid { .. } => Some(
                "put `@upload(max_mb: N)` immediately before a route whose body is `Upload<Format>`; the runtime enforces that exact declared limit and never supplies a hidden default".into(),
            ),
            Self::ConnectorConfigInvalid { .. } => Some(
                "`base_url` must be an absolute `http(s)://` URL; `timeout: <n>s` (1 to 3600) and `redirects: never | same_host | allowed_hosts` are required; each operation path must start with `/` and its `{placeholders}` must name declared parameters; reliability values (`retry`, `rate_limit`, `circuit_breaker`) must be non-zero.".into(),
            ),
        }
    }
}
