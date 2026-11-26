//! Structural types the checker assigns to expressions.
//!
//! Distinct from `corvid_ast::TypeRef`, which is what the user *wrote*.
//! `Type` is what the compiler *resolved*.

use corvid_ast::{Effect, WeakEffectRow};
use corvid_resolve::DefId;

/// Stable identity for a struct imported from another `.cor` module.
/// The module path is part of the type identity so two modules can
/// both export `Receipt` without becoming accidentally assignable.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ImportedStructType {
    pub module_path: String,
    pub def_id: DefId,
    pub name: String,
}

/// The cases a `Conflict<Row>` value can be. A narrower set is
/// assignable to a wider one, so the result of `insert` (only `Taken`)
/// can be returned as a written `Conflict<Row>` (every case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConflictCases(u8);

/// One case of a [`Type::Conflict`], in declaration order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictCase {
    /// `Changed(current)`: the row changed since it was read.
    Changed,
    /// `Deleted`: the row is gone.
    Deleted,
    /// `Taken(holder)`: another row holds its key or a unique value.
    Taken,
}

impl ConflictCase {
    /// Every case, in declaration order; a case's position is its
    /// variant index at run time.
    pub const ALL: [ConflictCase; 3] = [ConflictCase::Changed, ConflictCase::Deleted, ConflictCase::Taken];

    pub fn name(self) -> &'static str {
        match self {
            ConflictCase::Changed => "Changed",
            ConflictCase::Deleted => "Deleted",
            ConflictCase::Taken => "Taken",
        }
    }

    /// The case's payload field, if it has one.
    pub fn field(self) -> Option<&'static str> {
        match self {
            ConflictCase::Changed => Some("current"),
            ConflictCase::Deleted => None,
            ConflictCase::Taken => Some("holder"),
        }
    }

    pub fn index(self) -> u32 {
        self as u32
    }

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|case| case.name() == name)
    }
}

impl ConflictCases {
    /// Every case: what a written `Conflict<Row>` means.
    pub const ALL: ConflictCases = ConflictCases(0b111);

    pub fn of(cases: &[ConflictCase]) -> Self {
        ConflictCases(cases.iter().fold(0, |bits, case| bits | 1 << case.index()))
    }

    pub fn contains(self, case: ConflictCase) -> bool {
        self.0 & 1 << case.index() != 0
    }

    pub fn is_within(self, other: ConflictCases) -> bool {
        self.0 & !other.0 == 0
    }

    pub fn union(self, other: ConflictCases) -> Self {
        ConflictCases(self.0 | other.0)
    }

    /// The cases in declaration order.
    pub fn cases(self) -> impl Iterator<Item = ConflictCase> {
        ConflictCase::ALL.into_iter().filter(move |case| self.contains(*case))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Type {
    // Primitives
    Int,
    Float,
    /// B06 — an exact base-10 number (`corvid_decimal::Decimal`). Mixing
    /// it with `Float` is a compile error; `Int` mixes in arithmetic
    /// because the conversion is exact.
    Decimal,
    /// B06 — an exact length of elapsed time (`corvid_time::Duration`).
    Duration,
    /// B06 — an exact point on the UTC timeline (`corvid_time::Instant`).
    Instant,
    /// B06 — an IANA time zone (`corvid_time::TimeZone`).
    TimeZone,
    /// B06 — an instant seen in a time zone (`corvid_time::ZonedTime`).
    ZonedTime,
    /// B06 — an ISO 4217 currency (`corvid_currency::Currency`).
    Currency,
    /// B06 — a BCP 47 locale with CLDR data (`corvid_currency::Locale`).
    Locale,
    /// An immutable byte string (`corvid_crypto::Bytes`): digests, MAC
    /// tags, keys and binary payloads. `==` is constant-time; there is no
    /// ordering.
    Bytes,
    /// Sensitive text (`corvid_crypto::Secret`): a key, a password or a
    /// token the program can use but never reveal. It has no `String`
    /// form, no JSON form and no display; `==` is constant-time.
    Secret,
    String,
    Bool,
    Nothing,

    /// A user-declared `type` (struct-like).
    Struct(DefId),

    /// A public `type` imported through `alias.Name`.
    ImportedStruct(ImportedStructType),

    /// A tool/prompt/agent, considered as a first-class value.
    Function {
        params: Vec<Type>,
        ret: Box<Type>,
        effect: Effect,
    },

    /// A list of homogeneous elements.
    List(Box<Type>),

    /// A key->value map with homogeneous keys and values
    /// (slice 45g). Insertion-order iteration; structural key
    /// equality; reads return `Option<V>`.
    Map(Box<Type>, Box<Type>),

    /// Compiler-known `Stream<T>`.
    Stream(Box<Type>),

    /// Compiler-known `Result<T, E>`.
    Result(Box<Type>, Box<Type>),

    /// Compiler-known `Option<T>`.
    Option(Box<Type>),

    /// Compiler-known `Weak<T>` / `Weak<T, {effects}>`.
    Weak(Box<Type>, WeakEffectRow),

    /// Compiler-known `Tainted<T>` (slice 50i) — a value derived from
    /// UNTRUSTED content (a `data: untrusted` effect source, or the
    /// output of a prompt that consumed one). Never assignable to
    /// `T`: taint must not launder silently. Refused as an argument
    /// to approval-requiring calls; unwrapped only by the explicit
    /// `trusted(expr)` boundary. Compile-time only — at runtime a
    /// `Tainted<String>` IS a `String`.
    Tainted(Box<Type>),
    /// Compiler-known `Grounded<T>` — a value whose provenance chain
    /// includes at least one `data: grounded` source. The compiler
    /// verifies this statically by tracing data flow from retrieval
    /// tools through prompts to return types.
    Grounded(Box<Type>),

    /// Compiler-known `Partial<T>` for progressive structured streams.
    /// Field access on `Partial<Struct>` returns `Option<FieldType>`.
    Partial(Box<Type>),

    /// Compiler-known `ResumeToken<T>` for resuming interrupted streams.
    ResumeToken(Box<Type>),

    /// Compiler-known `Upload<Format>` (slice 51f) — a file upload
    /// crossing the HTTP boundary, where `Format` is a tag type
    /// (`Pdf`, `Image`, `Csv`, ...) that supplies default accepted
    /// MIME. An HTTP-boundary type: the application contract and its
    /// OpenAPI projection describe it (accepted MIME / max size /
    /// retention from an `@upload(...)` field attribute), and
    /// `corvid serve` receives it as multipart. Native codegen
    /// backends refuse to lower it, the same interpreter/serve-tier
    /// stance `DbHandle` takes.
    Upload(Box<Type>),

    /// Compiler-known `Page<Item>` (slice 51f) — one cursor-paginated
    /// page of `Item`s. Structurally `{ items: List<Item>,
    /// next_cursor: Option<String>, has_more: Bool }`; a route or
    /// agent returning `Page<Item>` advertises cursor pagination in
    /// the contract and accepts a `cursor` query parameter. Another
    /// HTTP-boundary type — codegen backends refuse to lower it.
    Page(Box<Type>),

    /// Compiler-known `Response<Body>` (B05): a connector response with
    /// its status and headers. Returned only by a connector operation
    /// that declares it; read with `status()`, `body()` and
    /// `header(name)`. It has no JSON form and never crosses a route,
    /// prompt, store or table boundary.
    Response(Box<Type>),

    /// Compiler-known `Conflict<Row>` (B09): why a table write did not
    /// happen — `Changed(current)`, `Deleted` or `Taken(holder)`. The
    /// cases say which of them this value can be: a written
    /// `Conflict<Row>` can be any, and each table operation returns only
    /// the ones it produces, so a `match` is complete when it covers
    /// those. [`ConflictCases`] has the rules.
    Conflict(Box<Type>, ConflictCases),

    /// Compiler-known `TraceId` — an opaque handle to a recorded
    /// JSONL trace, used as the subject of a `replay <expr>:`
    /// expression. String literals coerce to `TraceId` inside a
    /// replay context so `replay "run.jsonl": ...` parses
    /// naturally; richer producers (`Trace::load(...)`) can land
    /// later without breaking the surface syntax. Phase 21 slice
    /// 21-inv-E-3.
    TraceId,

    /// Compiler-known `DbHandle` — an opaque, refcounted handle to
    /// a SQLite connection, returned by `std/db.cor`'s executing
    /// `db_open` tool and threaded through `db_query` /
    /// `db_execute`. Phase 33S3a introduces the type as a
    /// load-bearing language primitive: it can ONLY be constructed
    /// by the runtime's `db_open` dispatch path (mapped at the
    /// vm-value layer to `Value::DbHandle(Arc<DbHandleInner>)`),
    /// which means user code structurally cannot fabricate a
    /// connection. The opacity guarantee is what makes
    /// "executing SQLite is typed and tamper-proof" true at the
    /// language level rather than at the documentation level.
    /// Codegen backends (CL / Py / WASM) refuse to lower this
    /// type until cdylib codegen lands in a future slice;
    /// interpreter-tier execution is fully supported.
    DbHandle,

    /// Compiler-known `JsonValue` — an opaque, refcounted parsed
    /// JSON value, returned by `std/json.cor`'s executing
    /// `json_parse` tool and threaded through the typed accessor
    /// tools (`json_get_int` / `json_get_string` / etc.). Phase
    /// 33R5b-a introduces the type as a load-bearing language
    /// primitive: the value is the parsed JSON shape (a wrapper
    /// around `Arc<serde_json::Value>`), and the typed accessors
    /// return `Result<T, String>` so field-type mismatches surface
    /// as recoverable errors rather than panics.
    ///
    /// Unlike `DbHandle`, JsonValue has NO opacity gate at the
    /// `json_to_value` boundary because the payload IS the JSON
    /// shape — there is no underlying registry the value indexes
    /// into. Constructing `Value::JsonValue` from JSON is the
    /// natural conversion path; the JSON `null` / numbers /
    /// strings / arrays / objects all map directly.
    ///
    /// Codegen backends refuse to lower this type until cdylib
    /// codegen lands in a follow-up slice (the C-ABI exports
    /// `corvid_json_parse` / `corvid_json_get_field_*` already
    /// exist in `corvid-runtime::ffi_bridge::json_exports`, so
    /// the cdylib bridging is plumbing rather than primitives).
    JsonValue,

    /// Compiler-known `JsonBuilder` — an opaque, mutable builder
    /// for assembling JSON objects field-by-field. Returned by
    /// `std/json.cor`'s `json_object_new` and consumed by
    /// `json_object_set_*` (fluent — mutates the inner
    /// `Arc<Mutex<...>>` and returns the same builder) and
    /// `json_object_finish` (snapshots the current state and
    /// serialises to a `String`; the builder remains usable for
    /// further set+finish cycles).
    ///
    /// The Arc-of-Mutex design lets multiple references to the
    /// same builder all see each other's mutations — useful when
    /// passing a builder through a chain of agent calls. The
    /// snapshot semantics of `json_object_finish` means there is
    /// no "consumed builder" lifecycle to track; calling finish
    /// twice yields two independent strings reflecting the
    /// builder's state at each call.
    ///
    /// Codegen backends refuse to lower this type for the same
    /// reason as `JsonValue` — interpreter-only in 33R5b.
    JsonBuilder,

    /// Compiler-known `JsonError` — why `T.from_json(text)` failed.
    /// A read-only record whose fields are listed by
    /// [`crate::builtin_methods::json_error_fields`]: `path` (such as
    /// `items[1].quantity`, empty at the root), `expected`, `found`
    /// and `message`. At runtime it is an ordinary record value.
    JsonError,

    /// Compiler-known `RegexMatch` — one match of a regular
    /// expression (B03). A read-only record whose fields are listed by
    /// [`crate::builtin_methods::regex_match_fields`]: `text`, `start`
    /// and `end` in characters, and `groups`. At runtime it is an
    /// ordinary record value.
    RegexMatch,

    /// Synthetic struct-like value for backend route path captures.
    RouteParams(Vec<(String, Type)>),

    /// Placeholder when the checker can't determine a precise type.
    /// Propagates without cascading errors.
    Unknown,
}

impl Type {
    /// The fields of a `Page<Item>` envelope in declaration order:
    /// the items, the cursor of the next page if there is one, and
    /// whether there is one.
    pub fn page_fields(item: &Type) -> [(&'static str, Type); 3] {
        [
            ("items", Type::List(Box::new(item.clone()))),
            ("next_cursor", Type::Option(Box::new(Type::String))),
            ("has_more", Type::Bool),
        ]
    }

    /// The type a bare compiler-known name denotes in source (`Int`,
    /// `DbHandle`, `JsonValue`, ...), or `None` for a name that must be
    /// looked up among declarations. The checker, the IR lowerer and
    /// the ABI emitter all resolve names through this one table.
    pub fn compiler_known(name: &str) -> Option<Type> {
        Some(match name {
            "Int" => Type::Int,
            "Float" => Type::Float,
            "Decimal" => Type::Decimal,
            "Duration" => Type::Duration,
            "Instant" => Type::Instant,
            "TimeZone" => Type::TimeZone,
            "ZonedTime" => Type::ZonedTime,
            "Currency" => Type::Currency,
            "Locale" => Type::Locale,
            "Bytes" => Type::Bytes,
            "Secret" => Type::Secret,
            "String" => Type::String,
            "Bool" => Type::Bool,
            "Nothing" => Type::Nothing,
            // Opaque runtime handles: only the executing stdlib tools
            // (`db_open`, `json_parse`, `json_object_new`) produce them.
            "DbHandle" => Type::DbHandle,
            "TraceId" => Type::TraceId,
            "JsonValue" => Type::JsonValue,
            "JsonBuilder" => Type::JsonBuilder,
            "JsonError" => Type::JsonError,
            "RegexMatch" => Type::RegexMatch,
            _ => return None,
        })
    }

    /// Human-readable name used in diagnostic messages. A declared
    /// struct reads `struct`; [`Self::display_name_with`] names it.
    pub fn display_name(&self) -> String {
        self.display_name_with(&|_| None)
    }

    /// The type's name, with each declared struct named by `struct_name`
    /// (`struct` when it gives none). The one renderer of a resolved
    /// type: diagnostics use it without names, and the Application
    /// Contract uses it to name the type an alias stands for.
    pub fn display_name_with(&self, struct_name: &dyn Fn(DefId) -> Option<String>) -> String {
        let name = |ty: &Type| ty.display_name_with(struct_name);
        match self {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::Decimal => "Decimal".into(),
            Type::Duration => "Duration".into(),
            Type::Instant => "Instant".into(),
            Type::TimeZone => "TimeZone".into(),
            Type::ZonedTime => "ZonedTime".into(),
            Type::Currency => "Currency".into(),
            Type::Locale => "Locale".into(),
            Type::Bytes => "Bytes".into(),
            Type::Secret => "Secret".into(),
            Type::String => "String".into(),
            Type::Bool => "Bool".into(),
            Type::Nothing => "Nothing".into(),
            Type::Struct(id) => struct_name(*id).unwrap_or_else(|| "struct".into()),
            Type::ImportedStruct(imported) => imported.name.clone(),
            Type::Function { .. } => "function".into(),
            Type::List(inner) => format!("List<{}>", name(inner)),
            Type::Map(k, v) => format!("Map<{}, {}>", name(k), name(v)),
            Type::Stream(inner) => format!("Stream<{}>", name(inner)),
            Type::Result(ok, err) => {
                format!("Result<{}, {}>", name(ok), name(err))
            }
            Type::Option(inner) => format!("Option<{}>", name(inner)),
            Type::Weak(inner, effects) => {
                if effects.is_any() {
                    format!("Weak<{}>", name(inner))
                } else {
                    let names = effects
                        .effects()
                        .into_iter()
                        .map(|effect| match effect {
                            corvid_ast::WeakEffect::ToolCall => "tool_call",
                            corvid_ast::WeakEffect::Llm => "llm",
                            corvid_ast::WeakEffect::Approve => "approve",
                            corvid_ast::WeakEffect::Human => "human",
                        })
                        .collect::<Vec<_>>()
                        .join(", ");
                    format!("Weak<{}, {{{names}}}>", name(inner))
                }
            }
            Type::Grounded(inner) => format!("Grounded<{}>", name(inner)),
            Type::Tainted(inner) => format!("Tainted<{}>", name(inner)),
            Type::Partial(inner) => format!("Partial<{}>", name(inner)),
            Type::ResumeToken(inner) => format!("ResumeToken<{}>", name(inner)),
            Type::Upload(inner) => format!("Upload<{}>", name(inner)),
            Type::Page(inner) => format!("Page<{}>", name(inner)),
            Type::Response(inner) => format!("Response<{}>", name(inner)),
            Type::Conflict(row, _) => format!("Conflict<{}>", name(row)),
            Type::TraceId => "TraceId".into(),
            Type::DbHandle => "DbHandle".into(),
            Type::JsonValue => "JsonValue".into(),
            Type::JsonBuilder => "JsonBuilder".into(),
            Type::JsonError => "JsonError".into(),
            Type::RegexMatch => "RegexMatch".into(),
            Type::RouteParams(_) => "route path params".into(),
            Type::Unknown => "<unknown>".into(),
        }
    }

    /// Strip every `Grounded<>` wrapper, returning the inner
    /// non-grounded type. A (degenerate) `Grounded<Grounded<T>>`
    /// normalises to `T`.
    ///
    /// The single source of truth for "see through grounding." A
    /// `Grounded<T>` value is operationally a `T` carrying provenance;
    /// any site that routes on the *shape* of a type — the operator
    /// checks (`checker/ops.rs`), the wrapping-int IR-lowering test
    /// (`corvid-ir`), the native operand-routing decisions
    /// (`corvid-codegen-cl`) — must see through the wrapper so a
    /// grounded operand routes exactly as its inner type would.
    pub fn ungrounded(&self) -> &Type {
        match self {
            Type::Grounded(inner) => inner.ungrounded(),
            other => other,
        }
    }

    /// Slice 50i — strip `Tainted<>` wrapper(s) for operator
    /// contagion, mirroring [`Self::ungrounded`].
    pub fn untainted(&self) -> &Type {
        match self {
            Type::Tainted(inner) => inner.untainted(),
            other => other,
        }
    }

    /// Whether this type is or names `Secret` among its type arguments:
    /// `Secret`, `List<Secret>`, `Result<Secret, String>`, a function
    /// taking one. A declared record holding a `Secret` field is not
    /// seen here, because its fields live in its declaration; callers
    /// that can see declarations walk the fields too.
    /// Whether a `Response` appears anywhere inside this type.
    pub fn mentions_response(&self) -> bool {
        self.mentions(&|ty| matches!(ty, Type::Response(_)))
    }

    pub fn mentions_secret(&self) -> bool {
        self.mentions(&|ty| matches!(ty, Type::Secret))
    }

    /// Whether this type, or any type inside it, satisfies `found`. The
    /// one walk over a type's parts; it does not enter a declared
    /// record's fields, which live in its declaration.
    pub fn mentions(&self, found: &dyn Fn(&Type) -> bool) -> bool {
        if found(self) {
            return true;
        }
        match self {
            Type::List(inner)
            | Type::Option(inner)
            | Type::Tainted(inner)
            | Type::Grounded(inner)
            | Type::Stream(inner)
            | Type::Partial(inner)
            | Type::ResumeToken(inner)
            | Type::Upload(inner)
            | Type::Page(inner)
            | Type::Response(inner)
            | Type::Conflict(inner, _)
            | Type::Weak(inner, _) => inner.mentions(found),
            Type::Map(a, b) | Type::Result(a, b) => a.mentions(found) || b.mentions(found),
            Type::Function { params, ret, .. } => ret.mentions(found) || params.iter().any(|p| p.mentions(found)),
            Type::RouteParams(params) => params.iter().any(|(_, param)| param.mentions(found)),
            _ => false,
        }
    }

    /// Offer this type, and then each type inside it, to `replace`, the
    /// mutable twin of [`Type::mentions`]. When `replace` rewrites a type
    /// it returns true and its parts are not visited. Returns whether
    /// anything was replaced. Like `mentions`, it does not enter a
    /// declared record's fields.
    pub fn replace_within(&mut self, replace: &mut dyn FnMut(&mut Type) -> bool) -> bool {
        if replace(self) {
            return true;
        }
        match self {
            Type::List(inner)
            | Type::Option(inner)
            | Type::Tainted(inner)
            | Type::Grounded(inner)
            | Type::Stream(inner)
            | Type::Partial(inner)
            | Type::ResumeToken(inner)
            | Type::Upload(inner)
            | Type::Page(inner)
            | Type::Response(inner)
            | Type::Conflict(inner, _)
            | Type::Weak(inner, _) => inner.replace_within(replace),
            Type::Map(a, b) | Type::Result(a, b) => {
                let a = a.replace_within(replace);
                b.replace_within(replace) | a
            }
            Type::Function { params, ret, .. } => {
                let mut found = ret.replace_within(replace);
                for param in params {
                    found |= param.replace_within(replace);
                }
                found
            }
            Type::RouteParams(params) => {
                let mut found = false;
                for (_, param) in params {
                    found |= param.replace_within(replace);
                }
                found
            }
            _ => false,
        }
    }

    /// Is this type compatible with `other` in a value-assignment position?
    ///
    /// v0.1 is intentionally lenient: structurally identical types match,
    /// `Unknown` matches anything (to avoid error cascades), and `Int`
    /// implicitly coerces to `Float` in typing-friendly contexts.
    pub fn is_assignable_to(&self, other: &Type) -> bool {
        match (self, other) {
            (Type::Unknown, _) | (_, Type::Unknown) => true,
            (Type::Int, Type::Float) => true, // widening
            (Type::List(a), Type::List(b)) => a.is_assignable_to(b),
            (Type::Map(ka, va), Type::Map(kb, vb)) => {
                ka.is_assignable_to(kb) && va.is_assignable_to(vb)
            }
            (Type::Stream(a), Type::Stream(b)) => a.is_assignable_to(b),
            (Type::Option(a), Type::Option(b)) => a.is_assignable_to(b),
            (Type::Result(ok_a, err_a), Type::Result(ok_b, err_b)) => {
                ok_a.is_assignable_to(ok_b) && err_a.is_assignable_to(err_b)
            }
            (Type::Weak(inner_a, effects_a), Type::Weak(inner_b, effects_b)) => {
                inner_a.is_assignable_to(inner_b) && effects_a == effects_b
            }
            (Type::Grounded(a), Type::Grounded(b)) => a.is_assignable_to(b),
            (Type::Tainted(a), Type::Tainted(b)) => a.is_assignable_to(b),
            // Deliberately NO `Tainted<T>` → `T` coercion (contrast
            // Grounded's legacy rule below): taint never launders
            // silently — `trusted(expr)` is the only exit.
            (Type::Partial(a), Type::Partial(b)) => a.is_assignable_to(b),
            (Type::ResumeToken(a), Type::ResumeToken(b)) => a.is_assignable_to(b),
            (Type::Conflict(a, cases_a), Type::Conflict(b, cases_b)) => {
                a.is_assignable_to(b) && cases_a.is_within(*cases_b)
            }
            (Type::RouteParams(a), Type::RouteParams(b)) => a == b,
            // Function types (45j): parameters are contravariant,
            // the return type covariant. The legacy binary effect is
            // ignored for assignability — lambdas are always Safe.
            (
                Type::Function {
                    params: pa,
                    ret: ra,
                    ..
                },
                Type::Function {
                    params: pb,
                    ret: rb,
                    ..
                },
            ) => {
                pa.len() == pb.len()
                    && pb.iter().zip(pa.iter()).all(|(b, a)| b.is_assignable_to(a))
                    && ra.is_assignable_to(rb)
            }
            // Legacy compatibility: Grounded<T> remains assignable to T.
            // New code should prefer `.unwrap_discarding_sources()` so the
            // provenance drop is visible in source and IR.
            (Type::Grounded(inner), other) => inner.is_assignable_to(other),
            (a, b) => a == b,
        }
    }

    /// The narrowest type both `self` and `other` are assignable to,
    /// when there is one: the wider of the two, or, for conflicts with
    /// different cases (an `insert`'s beside a `delete`'s), a conflict
    /// holding the cases of both. Match arms and branches join with it.
    pub fn join(&self, other: &Type) -> Option<Type> {
        if other.is_assignable_to(self) {
            return Some(self.clone());
        }
        if self.is_assignable_to(other) {
            return Some(other.clone());
        }
        Some(match (self, other) {
            (Type::Conflict(a, cases_a), Type::Conflict(b, cases_b)) => {
                Type::Conflict(Box::new(a.join(b)?), cases_a.union(*cases_b))
            }
            (Type::Result(ok_a, err_a), Type::Result(ok_b, err_b)) => {
                Type::Result(Box::new(ok_a.join(ok_b)?), Box::new(err_a.join(err_b)?))
            }
            (Type::Option(a), Type::Option(b)) => Type::Option(Box::new(a.join(b)?)),
            (Type::List(a), Type::List(b)) => Type::List(Box::new(a.join(b)?)),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod conflict_tests {
    use super::{ConflictCase, ConflictCases, Type};

    fn conflict(cases: &[ConflictCase]) -> Type {
        Type::Conflict(Box::new(Type::String), ConflictCases::of(cases))
    }

    /// An `insert`'s conflict (only `Taken`) goes where any conflict is
    /// expected, never the other way round.
    #[test]
    fn a_narrower_conflict_is_assignable_to_a_wider_one() {
        let taken = conflict(&[ConflictCase::Taken]);
        let any = Type::Conflict(Box::new(Type::String), ConflictCases::ALL);
        assert!(taken.is_assignable_to(&any));
        assert!(!any.is_assignable_to(&taken));
        assert_eq!(taken.display_name(), "Conflict<String>");
    }

    /// An `insert`'s conflict beside a `delete`'s joins into one holding
    /// the cases of both, inside a `Result` too.
    #[test]
    fn conflicts_join_into_the_union_of_their_cases() {
        let taken = conflict(&[ConflictCase::Taken]);
        let stale = conflict(&[ConflictCase::Changed, ConflictCase::Deleted]);
        assert_eq!(taken.join(&stale), Some(Type::Conflict(Box::new(Type::String), ConflictCases::ALL)));
        let result = |err: Type| Type::Result(Box::new(Type::Int), Box::new(err));
        assert_eq!(
            result(taken.clone()).join(&result(stale)),
            Some(result(Type::Conflict(Box::new(Type::String), ConflictCases::ALL)))
        );
        assert_eq!(taken.join(&Type::Int), None);
        let cases: Vec<&str> = ConflictCases::of(&[ConflictCase::Taken, ConflictCase::Changed]).cases().map(ConflictCase::name).collect();
        assert_eq!(cases, ["Changed", "Taken"]);
    }
}

#[cfg(test)]
mod tests {
    use super::Type;

    #[test]
    fn stream_display_name_and_assignability_follow_inner_type() {
        let stream = Type::Stream(Box::new(Type::String));
        assert_eq!(stream.display_name(), "Stream<String>");
        assert!(stream.is_assignable_to(&Type::Stream(Box::new(Type::String))));
        assert!(!stream.is_assignable_to(&Type::Stream(Box::new(Type::Int))));
    }
}
