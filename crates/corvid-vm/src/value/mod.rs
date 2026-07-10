//! Runtime value representation.
//!
//! Primitives copy by value. Cycle-capable composites (`Struct`, `List`,
//! `Result*`, `OptionSome`) ride on VM-owned heap cells with explicit
//! retain/release bookkeeping so the interpreter can own its refcount
//! semantics instead of delegating them to `Arc`.
//!
//! `String` intentionally stays `Arc<str>` for now because it is a leaf
//! payload with no outgoing refcounted edges. If a future string-like type
//! ever gains outgoing refcounted edges (rope fragments, parent-backed
//! string views, lazy concat nodes), it must migrate to `HeapHandle` style
//! ownership and participate in the VM collector.

use crate::errors::InterpError;
use corvid_resolve::DefId;
use corvid_runtime::{DbHandleInner, ProvenanceChain, ProvenanceEntry, ProvenanceKind};
use std::collections::HashMap;
use std::sync::Arc;

mod cells;
mod display;
mod heap;
mod object_ref;
mod stream;
mod weak;
pub use cells::{BoxedValue, EnumValue, ListValue, MapValue, StructValue};
pub use display::value_confidence;
pub(crate) use heap::Color;
pub(crate) use object_ref::{ObjectRef, WeakObjectRef};
pub(crate) use stream::{StreamChunk, StreamSender};
pub use stream::StreamValue;
pub use weak::{ListWeakValue, StructWeakValue, WeakValue};

/// A runtime value.
#[derive(Debug)]
pub enum Value {
    Int(i64),
    Float(f64),
    String(Arc<str>),
    Bool(bool),
    Nothing,
    Struct(StructValue),
    List(ListValue),
    /// Map cell (slice 45g): insertion-ordered, structurally-keyed.
    Map(MapValue),
    /// Sum-type value (slice 45h): owning type + variant + positional
    /// payload.
    Enum(EnumValue),
    Weak(WeakValue),
    ResultOk(BoxedValue),
    ResultErr(BoxedValue),
    OptionSome(BoxedValue),
    OptionNone,
    Grounded(GroundedValue),
    Partial(PartialValue),
    ResumeToken(ResumeTokenValue),
    Stream(StreamValue),
    /// Phase 33S3a — opaque, refcounted handle to a SQLite
    /// connection owned by the runtime. The inner `Arc` carries the
    /// handle's identity (the `handle_id` in `corvid-runtime`'s
    /// `DbRuntime` slotmap) plus diagnostic metadata; the actual
    /// `rusqlite::Connection` lives behind a mutex in the runtime
    /// and is reached only through `Runtime::call_tool` dispatch
    /// for `db_query` / `db_execute`. The opacity is structural:
    /// `DbHandleInner` is not `Default` or `From<u64>`, it has no
    /// public constructor outside the runtime crate's dispatch
    /// path, and `Value::DbHandle` cannot be produced via the
    /// JSON marshalling path (`json_to_value` rejects the handle
    /// shape with a structured error). This is what makes
    /// "you cannot fabricate a SQLite connection in user code" a
    /// load-bearing language property rather than a documentation
    /// claim.
    DbHandle(Arc<DbHandleInner>),

    /// Phase 33R5b-a — opaque, refcounted parsed JSON value.
    /// Produced by `std/json.cor`'s `json_parse` tool and threaded
    /// through the typed accessor tools (`json_get_int` /
    /// `json_get_string` / etc.). Unlike `DbHandle`, the payload
    /// IS the JSON shape — there is no registry the value indexes
    /// into. `json_to_value` against `Type::JsonValue` is the
    /// natural conversion path (the JSON `null` / numbers /
    /// strings / arrays / objects all map directly).
    ///
    /// Multiple references share the same Arc; the underlying
    /// `serde_json::Value` is immutable so no synchronization is
    /// needed.
    JsonValue(Arc<serde_json::Value>),

    /// Phase 33R5b-a — opaque, mutable JSON object builder.
    /// Returned by `std/json.cor`'s `json_object_new` and mutated
    /// by `json_object_set_*` (fluent — mutates the inner
    /// `serde_json::Map` and returns the same builder for
    /// chaining). `json_object_finish` snapshots the current
    /// state and serialises to a `String`; the builder remains
    /// usable for further set+finish cycles.
    ///
    /// The `Arc<Mutex<...>>` design lets multiple references to
    /// the same builder all see each other's mutations — useful
    /// when passing a builder through a chain of agent calls.
    JsonBuilder(Arc<std::sync::Mutex<serde_json::Map<String, serde_json::Value>>>),
}

// Phase 33S3b — `DbHandleInner` moved to `corvid-runtime::db`
// so the runtime's `DbHandleRegistry` can mint Arcs directly
// (the dispatch path that returns a `Value::DbHandle` produces
// the Arc inside corvid-runtime and hands it back to the
// interpreter, which wraps it in the Value variant). The type
// is re-exported by `corvid-vm` at the crate root so existing
// consumers' import paths continue to resolve.

pub(super) const UNBOUNDED_STREAM_WARN_THRESHOLD: usize = 1024;

/// A value with a provenance chain proving it derives from a grounded source.
#[derive(Debug, Clone)]
pub struct GroundedValue {
    pub inner: BoxedValue,
    pub provenance: ProvenanceChain,
    /// LLM-reported or deterministic confidence, composed via Min
    /// through the call graph. Defaults to 1.0 for deterministic tool
    /// results; prompts can set lower values from self-reported
    /// confidence or logprobs.
    pub confidence: f64,
}

impl GroundedValue {
    pub fn new(inner: Value, provenance: ProvenanceChain) -> Self {
        Self {
            inner: BoxedValue::new(inner),
            provenance,
            confidence: 1.0,
        }
    }

    pub fn with_confidence(inner: Value, provenance: ProvenanceChain, confidence: f64) -> Self {
        Self {
            inner: BoxedValue::new(inner),
            provenance,
            confidence: confidence.clamp(0.0, 1.0),
        }
    }

    pub fn sources(&self) -> &[ProvenanceEntry] {
        &self.provenance.entries
    }

    pub fn unwrap_with_reason(self, reason: &str) -> (Value, ProvenanceEntry) {
        let severed = ProvenanceEntry {
            kind: ProvenanceKind::Severed { reason: reason.to_string() },
            name: "unwrap".to_string(),
            timestamp_ms: 0,
        };
        (self.inner.get(), severed)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum PartialFieldValue {
    Complete(Value),
    Streaming,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PartialValue {
    type_id: DefId,
    type_name: String,
    fields: HashMap<String, PartialFieldValue>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ResumeTokenValue {
    pub(crate) prompt_name: String,
    pub(crate) args: Vec<Value>,
    pub(crate) delivered: Vec<StreamChunk>,
    pub(crate) provider_session: Option<String>,
}

#[derive(Debug, Clone)]
pub(crate) struct StreamResumeContext {
    pub prompt_name: String,
    pub args: Vec<Value>,
    pub provider_session: Option<String>,
}

impl PartialValue {
    pub fn new(
        type_id: DefId,
        type_name: impl Into<String>,
        fields: impl IntoIterator<Item = (String, PartialFieldValue)>,
    ) -> Self {
        Self {
            type_id,
            type_name: type_name.into(),
            fields: fields.into_iter().collect(),
        }
    }

    pub fn type_name(&self) -> &str {
        &self.type_name
    }

    pub fn get_field(&self, field: &str) -> Option<Value> {
        match self.fields.get(field)? {
            PartialFieldValue::Complete(value) => {
                Some(Value::OptionSome(BoxedValue::new(value.clone())))
            }
            PartialFieldValue::Streaming => Some(Value::OptionNone),
        }
    }

    pub fn with_fields<R>(&self, f: impl FnOnce(&HashMap<String, PartialFieldValue>) -> R) -> R {
        f(&self.fields)
    }
}


impl Clone for Value {
    fn clone(&self) -> Self {
        match self {
            Value::Int(n) => Value::Int(*n),
            Value::Float(x) => Value::Float(*x),
            Value::String(s) => Value::String(s.clone()),
            Value::Bool(b) => Value::Bool(*b),
            Value::Nothing => Value::Nothing,
            Value::Struct(s) => Value::Struct(s.clone()),
            Value::List(items) => Value::List(items.clone()),
            Value::Map(m) => Value::Map(m.clone()),
            Value::Enum(e) => Value::Enum(e.clone()),
            Value::Weak(w) => Value::Weak(w.clone()),
            Value::ResultOk(v) => Value::ResultOk(v.clone()),
            Value::ResultErr(v) => Value::ResultErr(v.clone()),
            Value::OptionSome(v) => Value::OptionSome(v.clone()),
            Value::OptionNone => Value::OptionNone,
            Value::Grounded(g) => Value::Grounded(g.clone()),
            Value::Partial(p) => Value::Partial(p.clone()),
            Value::ResumeToken(token) => Value::ResumeToken(token.clone()),
            Value::Stream(stream) => Value::Stream(stream.clone()),
            // Phase 33S3a — cloning a DbHandle increments the
            // inner Arc's strong count. This is the "refcounted"
            // half of the brief's "opaque, refcounted" promise:
            // multiple agents can hold the same handle and the
            // underlying connection lives until the last clone
            // drops (33S3b wires the actual runtime callback).
            Value::DbHandle(inner) => Value::DbHandle(inner.clone()),
            // Phase 33R5b-a — JSON value/builder clones are just
            // Arc increments. JsonValue's payload is immutable so
            // clones share read-only access; JsonBuilder's payload
            // is behind a Mutex so clones share mutable access
            // (set_* on one clone is visible through another).
            Value::JsonValue(value) => Value::JsonValue(value.clone()),
            Value::JsonBuilder(builder) => Value::JsonBuilder(builder.clone()),
        }
    }
}

impl Value {
    pub fn type_name(&self) -> String {
        match self {
            Value::Int(_) => "Int".into(),
            Value::Float(_) => "Float".into(),
            Value::String(_) => "String".into(),
            Value::Bool(_) => "Bool".into(),
            Value::Nothing => "Nothing".into(),
            Value::Struct(s) => s.type_name().to_string(),
            Value::List(_) => "List".into(),
            Value::Map(_) => "Map".into(),
            Value::Enum(e) => e.type_name().to_string(),
            Value::Weak(_) => "Weak".into(),
            Value::ResultOk(_) | Value::ResultErr(_) => "Result".into(),
            Value::OptionSome(_) | Value::OptionNone => "Option".into(),
            Value::Grounded(g) => format!("Grounded<{}>", g.inner.get().type_name()),
            Value::Partial(p) => format!("Partial<{}>", p.type_name()),
            Value::ResumeToken(_) => "ResumeToken".into(),
            Value::Stream(stream) => {
                format!("Stream<{}>", stream.backpressure_label())
            }
            Value::DbHandle(_) => "DbHandle".into(),
            Value::JsonValue(_) => "JsonValue".into(),
            Value::JsonBuilder(_) => "JsonBuilder".into(),
        }
    }

    pub fn new_struct(
        type_id: DefId,
        type_name: impl Into<String>,
        fields: impl IntoIterator<Item = (String, Value)>,
    ) -> Value {
        Value::Struct(StructValue::new(type_id, type_name, fields))
    }

    pub fn downgrade(&self) -> Option<WeakValue> {
        match self {
            Value::String(s) => Some(WeakValue::String(Arc::downgrade(s))),
            Value::Struct(s) => Some(WeakValue::Struct(StructWeakValue(Arc::downgrade(&s.0)))),
            Value::List(items) => Some(WeakValue::List(ListWeakValue(Arc::downgrade(&items.0)))),
            _ => None,
        }
    }

    pub(crate) fn as_object_ref(&self) -> Option<ObjectRef> {
        match self {
            Value::Struct(s) => Some(ObjectRef::Struct(s.0.clone())),
            Value::List(items) => Some(ObjectRef::List(items.0.clone())),
            Value::Map(m) => Some(ObjectRef::Map(m.0.clone())),
            Value::Enum(e) => Some(ObjectRef::Enum(e.0.clone())),
            Value::ResultOk(v) | Value::ResultErr(v) | Value::OptionSome(v) => {
                Some(ObjectRef::Boxed(v.0.clone()))
            }
            Value::Grounded(g) => Some(ObjectRef::Boxed(g.inner.0.clone())),
            Value::Partial(_) => None,
            Value::ResumeToken(_) => None,
            _ => None,
        }
    }
}



impl PartialEq for Value {
    fn eq(&self, other: &Value) -> bool {
        match (self, other) {
            (Value::Int(a), Value::Int(b)) => a == b,
            (Value::Float(a), Value::Float(b)) => a == b,
            (Value::Int(a), Value::Float(b)) => (*a as f64) == *b,
            (Value::Float(a), Value::Int(b)) => *a == (*b as f64),
            (Value::String(a), Value::String(b)) => a == b,
            (Value::Bool(a), Value::Bool(b)) => a == b,
            (Value::Nothing, Value::Nothing) => true,
            (Value::Struct(a), Value::Struct(b)) => a == b,
            (Value::List(a), Value::List(b)) => a == b,
            (Value::Map(a), Value::Map(b)) => a == b,
            (Value::Enum(a), Value::Enum(b)) => a == b,
            (Value::Weak(a), Value::Weak(b)) => a.ptr_eq(b),
            (Value::ResultOk(a), Value::ResultOk(b)) => a == b,
            (Value::ResultErr(a), Value::ResultErr(b)) => a == b,
            (Value::OptionSome(a), Value::OptionSome(b)) => a == b,
            (Value::OptionNone, Value::OptionNone) => true,
            (Value::Grounded(a), Value::Grounded(b)) => a.inner == b.inner,
            (Value::Partial(a), Value::Partial(b)) => a == b,
            (Value::ResumeToken(a), Value::ResumeToken(b)) => a == b,
            (Value::Stream(a), Value::Stream(b)) => a == b,
            // Phase 33S3a — two DbHandles compare equal when
            // they reference the SAME underlying connection
            // (same Arc pointer). This is identity equality, not
            // structural: cloning a handle yields an equal handle;
            // opening the same database file twice produces two
            // *different* handles (consistent with how rusqlite
            // treats independent connections).
            (Value::DbHandle(a), Value::DbHandle(b)) => Arc::ptr_eq(a, b),
            // Phase 33R5b-a — JsonValue equality is STRUCTURAL:
            // two JSON values with the same shape are equal even
            // if they were parsed from different sources. This
            // matches the natural mental model and matches
            // serde_json::Value's own PartialEq. JsonBuilder
            // equality is IDENTITY (Arc ptr_eq) because the
            // builder is mutable and structural equality would
            // race against concurrent mutations.
            (Value::JsonValue(a), Value::JsonValue(b)) => **a == **b,
            (Value::JsonBuilder(a), Value::JsonBuilder(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}




#[cfg(test)]
mod tests {
    use super::{DbHandleInner, StructValue, Value};
    use corvid_resolve::DefId;
    use std::sync::Arc;

    /// Phase 33S3a — `Value::DbHandle` cloning is just an `Arc`
    /// increment. Many clones of the same handle all observe the
    /// same inner pointer; dropping them returns the strong count
    /// to 1. This is the load-bearing "refcounted" half of the
    /// brief's promise — multiple agents can hold the same handle
    /// without copying the underlying connection.
    #[test]
    fn db_handle_clones_share_inner_arc_and_refcount_returns_to_one_on_drop() {
        let value = Value::DbHandle(Arc::new(DbHandleInner::new(42, ":memory:")));
        let strong = match &value {
            Value::DbHandle(inner) => Arc::strong_count(inner),
            _ => unreachable!(),
        };
        assert_eq!(strong, 1);

        let mut clones = Vec::new();
        for _ in 0..1000 {
            clones.push(value.clone());
        }

        let strong = match &value {
            Value::DbHandle(inner) => Arc::strong_count(inner),
            _ => unreachable!(),
        };
        assert_eq!(strong, 1001);

        drop(clones);

        let strong = match &value {
            Value::DbHandle(inner) => Arc::strong_count(inner),
            _ => unreachable!(),
        };
        assert_eq!(strong, 1);
    }

    /// Phase 33S3a — `Value::DbHandle` equality is Arc-pointer
    /// identity. Two clones of the same handle are equal; two
    /// independently-constructed handles with the same handle_id
    /// are NOT equal because they represent independent connection
    /// references (`rusqlite::Connection::open` semantics).
    #[test]
    fn db_handle_equality_is_arc_pointer_identity_not_structural() {
        let a = Value::DbHandle(Arc::new(DbHandleInner::new(7, ":memory:")));
        let a_clone = a.clone();
        let b = Value::DbHandle(Arc::new(DbHandleInner::new(7, ":memory:")));

        assert_eq!(a, a_clone, "clones share the same Arc and must compare equal");
        assert_ne!(
            a, b,
            "independently-constructed handles with the same handle_id must NOT compare equal"
        );
    }

    /// Phase 33S3a — `Value::DbHandle.type_name()` returns the
    /// language-level name "DbHandle" (not "DbHandleInner" or any
    /// implementation detail). This is what user-facing diagnostics
    /// surface when a typecheck or marshal error mentions the type.
    #[test]
    fn db_handle_type_name_is_the_language_level_name() {
        let value = Value::DbHandle(Arc::new(DbHandleInner::new(0, "./data/app.sqlite")));
        assert_eq!(value.type_name(), "DbHandle");
    }

    /// Phase 33S3a — the opacity guarantee at the JSON boundary.
    /// `json_to_value` REFUSES to mint a `Value::DbHandle` from a
    /// JSON payload, even when the payload exactly matches the
    /// shape `value_to_json` emits for trace-debug visibility. This
    /// is what makes "you cannot fabricate a SQLite connection in
    /// user code" a load-bearing language property — there is no
    /// JSON round-trip a malicious tool could exploit to forge a
    /// handle.
    #[test]
    fn json_to_value_refuses_to_mint_a_db_handle_even_when_shape_matches_sentinel() {
        use crate::conv::{json_to_value, value_to_json};
        use corvid_types::Type;
        use std::collections::HashMap;

        // Round-trip an authentic handle through value_to_json to
        // obtain the EXACT sentinel shape an attacker would forge.
        let authentic =
            Value::DbHandle(Arc::new(DbHandleInner::new(99, "./data/app.sqlite")));
        let sentinel_json = value_to_json(&authentic);
        // Sentinel shape carries the tag for trace-debug visibility.
        assert_eq!(
            sentinel_json.get("tag").and_then(|t| t.as_str()),
            Some("db_handle_opaque_sentinel"),
            "value_to_json must tag the sentinel so traces can render it"
        );

        // Now attempt the round-trip: feeding the sentinel back
        // through json_to_value with expected: Type::DbHandle must
        // be refused. This is the OPACITY GATE.
        let empty: HashMap<DefId, &corvid_ir::IrType> = HashMap::new();
        let result = json_to_value(sentinel_json, &Type::DbHandle, &empty);
        assert!(
            result.is_err(),
            "json_to_value must refuse to mint a DbHandle from the sentinel; got {:?}",
            result.map(|v| v.type_name())
        );
        let err = result.unwrap_err().to_string();
        assert!(
            err.contains("DbHandle") && err.contains("opaque"),
            "diagnostic must name the opacity property; got: {err}"
        );
    }

    #[test]
    fn struct_handle_refcount_tracks_value_clones() {
        let value = Value::Struct(StructValue::new(
            DefId(1),
            "Node",
            [("label".to_string(), Value::String(Arc::from("root")))],
        ));
        let strong = match &value {
            Value::Struct(s) => s.strong_count_for_tests(),
            _ => unreachable!(),
        };
        assert_eq!(strong, 1);

        let mut clones = Vec::new();
        for _ in 0..1000 {
            clones.push(value.clone());
        }

        let strong = match &value {
            Value::Struct(s) => s.strong_count_for_tests(),
            _ => unreachable!(),
        };
        assert_eq!(strong, 1001);

        drop(clones);

        let strong = match &value {
            Value::Struct(s) => s.strong_count_for_tests(),
            _ => unreachable!(),
        };
        assert_eq!(strong, 1);
    }
}
