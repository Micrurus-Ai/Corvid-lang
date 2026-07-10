//! Refcounted heap-cell value types — `StructValue`, `ListValue`,
//! and `BoxedValue` along with their `*Inner` Arc-payloads.
//!
//! Each public handle owns one strong refcount tracked through
//! `HeapMeta` rather than the `Arc`'s internal counter, so the
//! cycle collector can re-target `strong` during reclamation
//! while leaving the `Arc` to manage the underlying allocation.
//! `Clone` retains, `Drop` hands the cell to
//! `cycle_collector::release_object` which decides between an
//! immediate free-zero-path and adding the cell to the candidate
//! buffer for trial deletion.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use corvid_resolve::DefId;

use crate::cycle_collector;

use super::heap::HeapMeta;
use super::{ObjectRef, Value};

#[derive(Debug)]
pub(crate) struct StructInner {
    pub(super) meta: HeapMeta,
    pub(super) type_id: DefId,
    pub(super) type_name: String,
    pub(super) fields: Mutex<Option<HashMap<String, Value>>>,
}

#[derive(Debug)]
pub(crate) struct ListInner {
    pub(super) meta: HeapMeta,
    pub(super) items: Mutex<Option<Vec<Value>>>,
}

#[derive(Debug)]
pub(crate) struct BoxedInner {
    pub(super) meta: HeapMeta,
    pub(super) value: Mutex<Option<Value>>,
}

#[derive(Debug)]
pub struct StructValue(pub(super) Arc<StructInner>);

#[derive(Debug)]
pub struct ListValue(pub(super) Arc<ListInner>);

#[derive(Debug)]
pub struct BoxedValue(pub(super) Arc<BoxedInner>);

/// Sum-type value cell (slice 45h): owning type id/name, variant
/// index/name, and POSITIONAL payload fields. Immutable after
/// construction at the language level, but stored behind the same
/// Mutex-cell shape so the cycle collector's clear-payload path
/// works uniformly.
#[derive(Debug)]
pub struct EnumValue(pub(super) Arc<EnumInner>);

#[derive(Debug)]
pub(crate) struct EnumInner {
    pub(super) meta: HeapMeta,
    pub(super) type_id: DefId,
    pub(super) type_name: String,
    pub(super) variant_index: u32,
    pub(super) variant_name: String,
    pub(super) fields: Mutex<Option<Vec<Value>>>,
}

/// Map cell (slice 45g): insertion-ordered `(key, value)` pairs with
/// structural key equality. Vec-backed — O(n) lookup is fine for v1
/// map sizes; the representation can move to an index map later
/// without changing any semantics.
#[derive(Debug)]
pub struct MapValue(pub(super) Arc<MapInner>);

#[derive(Debug)]
pub(crate) struct MapInner {
    pub(super) meta: HeapMeta,
    pub(super) entries: Mutex<Option<Vec<(Value, Value)>>>,
}

impl StructValue {
    pub fn new(
        type_id: DefId,
        type_name: impl Into<String>,
        fields: impl IntoIterator<Item = (String, Value)>,
    ) -> Self {
        Self(Arc::new(StructInner {
            meta: HeapMeta::new(),
            type_id,
            type_name: type_name.into(),
            fields: Mutex::new(Some(fields.into_iter().collect())),
        }))
    }

    pub fn type_id(&self) -> DefId {
        self.0.type_id
    }

    pub fn type_name(&self) -> &str {
        &self.0.type_name
    }

    pub fn get_field(&self, field: &str) -> Option<Value> {
        self.0
            .fields
            .lock()
            .expect("struct fields lock poisoned")
            .as_ref()
            .and_then(|fields| fields.get(field).cloned())
    }

    pub fn with_fields<R>(&self, f: impl FnOnce(&HashMap<String, Value>) -> R) -> R {
        let guard = self.0.fields.lock().expect("struct fields lock poisoned");
        let fields = guard.as_ref().expect("struct payload already freed");
        f(fields)
    }

    pub fn ptr_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }

    #[doc(hidden)]
    pub fn set_field(&self, field: impl Into<String>, value: Value) {
        let mut guard = self.0.fields.lock().expect("struct fields lock poisoned");
        let fields = guard.as_mut().expect("struct payload already freed");
        fields.insert(field.into(), value);
    }

    #[doc(hidden)]
    pub fn strong_count_for_tests(&self) -> usize {
        self.0.meta.strong_count()
    }
}

impl Clone for StructValue {
    fn clone(&self) -> Self {
        self.0.meta.retain();
        Self(self.0.clone())
    }
}

impl Drop for StructValue {
    fn drop(&mut self) {
        cycle_collector::release_object(ObjectRef::Struct(self.0.clone()));
    }
}

impl PartialEq for StructValue {
    fn eq(&self, other: &Self) -> bool {
        self.type_id() == other.type_id()
            && self.with_fields(|a| other.with_fields(|b| a == b))
    }
}

impl ListValue {
    pub fn new(items: impl IntoIterator<Item = Value>) -> Self {
        Self(Arc::new(ListInner {
            meta: HeapMeta::new(),
            items: Mutex::new(Some(items.into_iter().collect())),
        }))
    }

    pub fn len(&self) -> usize {
        self.0
            .items
            .lock()
            .expect("list items lock poisoned")
            .as_ref()
            .expect("list payload already freed")
            .len()
    }

    pub fn iter_cloned(&self) -> Vec<Value> {
        self.0
            .items
            .lock()
            .expect("list items lock poisoned")
            .as_ref()
            .expect("list payload already freed")
            .clone()
    }

    pub fn get(&self, idx: usize) -> Option<Value> {
        self.0
            .items
            .lock()
            .expect("list items lock poisoned")
            .as_ref()
            .and_then(|items| items.get(idx).cloned())
    }

    /// Replace the element at `idx` in place (slice 45b place
    /// assignment). Callers bounds-check first; out-of-range is a
    /// no-op here to keep the lock scope minimal.
    pub fn set(&self, idx: usize, value: Value) {
        if let Some(items) = self.0.items.lock().expect("list lock").as_mut() {
            if idx < items.len() {
                items[idx] = value;
            }
        }
    }

    /// Append an element in place (slice 45f `append`).
    pub fn push(&self, value: Value) {
        if let Some(items) = self.0.items.lock().expect("list lock").as_mut() {
            items.push(value);
        }
    }

    /// Reverse in place (slice 45f `reverse`).
    pub fn reverse_in_place(&self) {
        if let Some(items) = self.0.items.lock().expect("list lock").as_mut() {
            items.reverse();
        }
    }

    /// Sort in place with the provided comparator (slice 45f
    /// `sort`; the checker gates element types to Int/Float/String).
    pub fn sort_in_place_by(
        &self,
        cmp: impl FnMut(&Value, &Value) -> std::cmp::Ordering,
    ) {
        if let Some(items) = self.0.items.lock().expect("list lock").as_mut() {
            items.sort_by(cmp);
        }
    }

    pub fn ptr_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

impl Clone for ListValue {
    fn clone(&self) -> Self {
        self.0.meta.retain();
        Self(self.0.clone())
    }
}

impl Drop for ListValue {
    fn drop(&mut self) {
        cycle_collector::release_object(ObjectRef::List(self.0.clone()));
    }
}

impl PartialEq for ListValue {
    fn eq(&self, other: &Self) -> bool {
        self.iter_cloned() == other.iter_cloned()
    }
}

impl BoxedValue {
    pub fn new(value: Value) -> Self {
        Self(Arc::new(BoxedInner {
            meta: HeapMeta::new(),
            value: Mutex::new(Some(value)),
        }))
    }

    pub fn get(&self) -> Value {
        self.0
            .value
            .lock()
            .expect("boxed value lock poisoned")
            .as_ref()
            .expect("boxed payload already freed")
            .clone()
    }

    pub fn ptr_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

impl Clone for BoxedValue {
    fn clone(&self) -> Self {
        self.0.meta.retain();
        Self(self.0.clone())
    }
}

impl Drop for BoxedValue {
    fn drop(&mut self) {
        cycle_collector::release_object(ObjectRef::Boxed(self.0.clone()));
    }
}

impl EnumValue {
    pub fn new(
        type_id: DefId,
        type_name: impl Into<String>,
        variant_index: u32,
        variant_name: impl Into<String>,
        fields: Vec<Value>,
    ) -> Self {
        Self(Arc::new(EnumInner {
            meta: HeapMeta::new(),
            type_id,
            type_name: type_name.into(),
            variant_index,
            variant_name: variant_name.into(),
            fields: Mutex::new(Some(fields)),
        }))
    }

    pub fn type_id(&self) -> DefId {
        self.0.type_id
    }

    pub fn type_name(&self) -> &str {
        &self.0.type_name
    }

    pub fn variant_index(&self) -> u32 {
        self.0.variant_index
    }

    pub fn variant_name(&self) -> &str {
        &self.0.variant_name
    }

    pub fn fields_cloned(&self) -> Vec<Value> {
        self.0
            .fields
            .lock()
            .expect("enum lock")
            .as_ref()
            .cloned()
            .unwrap_or_default()
    }

    pub fn ptr_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

impl Clone for EnumValue {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Structural equality: same owning type, same variant, equal
/// payload fields.
impl PartialEq for EnumValue {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        self.0.type_id == other.0.type_id
            && self.0.variant_index == other.0.variant_index
            && self.fields_cloned() == other.fields_cloned()
    }
}

impl MapValue {
    /// Build from entries; a later duplicate key WINS (Python dict
    /// literal semantics).
    pub fn new(entries: impl IntoIterator<Item = (Value, Value)>) -> Self {
        let mut out: Vec<(Value, Value)> = Vec::new();
        for (k, v) in entries {
            if let Some(slot) = out.iter_mut().find(|(ek, _)| *ek == k) {
                slot.1 = v;
            } else {
                out.push((k, v));
            }
        }
        Self(Arc::new(MapInner {
            meta: HeapMeta::new(),
            entries: Mutex::new(Some(out)),
        }))
    }

    pub fn len(&self) -> usize {
        self.0
            .entries
            .lock()
            .expect("map lock")
            .as_ref()
            .map(|e| e.len())
            .unwrap_or(0)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn get_by_key(&self, key: &Value) -> Option<Value> {
        self.0
            .entries
            .lock()
            .expect("map lock")
            .as_ref()
            .and_then(|e| e.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()))
    }

    pub fn insert_or_update(&self, key: Value, value: Value) {
        if let Some(entries) = self.0.entries.lock().expect("map lock").as_mut() {
            if let Some(slot) = entries.iter_mut().find(|(k, _)| *k == key) {
                slot.1 = value;
            } else {
                entries.push((key, value));
            }
        }
    }

    pub fn remove(&self, key: &Value) -> Option<Value> {
        let mut guard = self.0.entries.lock().expect("map lock");
        let entries = guard.as_mut()?;
        let idx = entries.iter().position(|(k, _)| k == key)?;
        Some(entries.remove(idx).1)
    }

    pub fn keys_cloned(&self) -> Vec<Value> {
        self.0
            .entries
            .lock()
            .expect("map lock")
            .as_ref()
            .map(|e| e.iter().map(|(k, _)| k.clone()).collect())
            .unwrap_or_default()
    }

    pub fn values_cloned(&self) -> Vec<Value> {
        self.0
            .entries
            .lock()
            .expect("map lock")
            .as_ref()
            .map(|e| e.iter().map(|(_, v)| v.clone()).collect())
            .unwrap_or_default()
    }

    pub fn entries_cloned(&self) -> Vec<(Value, Value)> {
        self.0
            .entries
            .lock()
            .expect("map lock")
            .as_ref()
            .cloned()
            .unwrap_or_default()
    }

    pub fn ptr_key(&self) -> usize {
        Arc::as_ptr(&self.0) as usize
    }
}

impl Clone for MapValue {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

/// Structural, ORDER-INSENSITIVE equality (Python dict ==): same
/// length and every key maps to an equal value.
impl PartialEq for MapValue {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.0, &other.0) {
            return true;
        }
        let a = self.entries_cloned();
        if a.len() != other.len() {
            return false;
        }
        a.iter()
            .all(|(k, v)| other.get_by_key(k).is_some_and(|ov| ov == *v))
    }
}

impl PartialEq for BoxedValue {
    fn eq(&self, other: &Self) -> bool {
        self.get() == other.get()
    }
}
