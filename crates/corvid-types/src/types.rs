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

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Type {
    // Primitives
    Int,
    Float,
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

    /// Synthetic struct-like value for backend route path captures.
    RouteParams(Vec<(String, Type)>),

    /// Placeholder when the checker can't determine a precise type.
    /// Propagates without cascading errors.
    Unknown,
}

impl Type {
    /// Human-readable name used in diagnostic messages.
    pub fn display_name(&self) -> String {
        match self {
            Type::Int => "Int".into(),
            Type::Float => "Float".into(),
            Type::String => "String".into(),
            Type::Bool => "Bool".into(),
            Type::Nothing => "Nothing".into(),
            Type::Struct(_) => "struct".into(),
            Type::ImportedStruct(imported) => imported.name.clone(),
            Type::Function { .. } => "function".into(),
            Type::List(inner) => format!("List<{}>", inner.display_name()),
            Type::Map(k, v) => format!("Map<{}, {}>", k.display_name(), v.display_name()),
            Type::Stream(inner) => format!("Stream<{}>", inner.display_name()),
            Type::Result(ok, err) => {
                format!("Result<{}, {}>", ok.display_name(), err.display_name())
            }
            Type::Option(inner) => format!("Option<{}>", inner.display_name()),
            Type::Weak(inner, effects) => {
                if effects.is_any() {
                    format!("Weak<{}>", inner.display_name())
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
                    format!("Weak<{}, {{{names}}}>", inner.display_name())
                }
            }
            Type::Grounded(inner) => format!("Grounded<{}>", inner.display_name()),
            Type::Tainted(inner) => format!("Tainted<{}>", inner.display_name()),
            Type::Partial(inner) => format!("Partial<{}>", inner.display_name()),
            Type::ResumeToken(inner) => format!("ResumeToken<{}>", inner.display_name()),
            Type::Upload(inner) => format!("Upload<{}>", inner.display_name()),
            Type::Page(inner) => format!("Page<{}>", inner.display_name()),
            Type::TraceId => "TraceId".into(),
            Type::DbHandle => "DbHandle".into(),
            Type::JsonValue => "JsonValue".into(),
            Type::JsonBuilder => "JsonBuilder".into(),
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
