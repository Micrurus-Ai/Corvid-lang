//! A map key: a `Value` whose hash agrees with Corvid's `==`, so a map
//! finds a key in constant time and still finds exactly the keys `==`
//! finds.
//!
//! Equal values hash alike:
//!
//! - **Numbers** hash by value. An `Int` hashes as the `Decimal` of the
//!   same value, because `Int` and `Decimal` compare by value, and a
//!   `Float` holding a whole number hashes as that number, because `Int`
//!   and `Float` compare by value too. `Decimal` hashes its normalized
//!   form (`1.0` and `1.00` alike), as its `==` does.
//! - **Text, booleans and the B06 scalars** hash their own value.
//! - **Records, lists, maps, options, results and variants** hash
//!   structurally, by the parts their `==` compares. This is sound because
//!   a map holds a snapshot of each such key (`snapshot_key`), which no
//!   other part of the program can change (proposal: map key snapshots).
//! - **Every other key** hashes to one fixed value and is found by `==`
//!   alone: `Bytes` and `Secret`, whose `==` is constant-time, so a lookup
//!   reveals nothing through the hash; grounded values, whose content sits
//!   in a cell; and identity-compared values.

use super::{BoxedValue, EnumValue, ListValue, MapValue, StructValue, Value};
use std::collections::HashSet;
use std::hash::{Hash, Hasher};

/// One key of a `MapValue`.
#[derive(Debug, Clone)]
pub(crate) struct MapKey(pub(crate) Value);

impl PartialEq for MapKey {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

/// `==` is an equivalence on the keys a map holds, apart from a `NaN`,
/// which equals nothing and so is stored once per insertion, as before.
impl Eq for MapKey {}

impl Hash for MapKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        hash_key(&self.0, state);
    }
}

/// The hash of a key whose equality is by value; see the module docs.
fn hash_key<H: Hasher>(value: &Value, state: &mut H) {
    match value {
        Value::Int(n) => corvid_decimal::Decimal::from_int(*n).hash(state),
        Value::Decimal(d) => d.hash(state),
        Value::Float(f) => match whole_number(*f) {
            Some(n) => corvid_decimal::Decimal::from_int(n).hash(state),
            None => {
                1u8.hash(state);
                f.to_bits().hash(state);
            }
        },
        Value::String(s) => {
            2u8.hash(state);
            s.hash(state);
        }
        Value::Bool(b) => {
            3u8.hash(state);
            b.hash(state);
        }
        Value::Nothing => 4u8.hash(state),
        Value::Duration(d) => {
            5u8.hash(state);
            d.hash(state);
        }
        Value::Instant(i) => {
            6u8.hash(state);
            i.hash(state);
        }
        Value::TimeZone(z) => {
            7u8.hash(state);
            z.hash(state);
        }
        Value::ZonedTime(z) => {
            8u8.hash(state);
            z.hash(state);
        }
        Value::Currency(c) => {
            9u8.hash(state);
            c.hash(state);
        }
        Value::Locale(l) => {
            10u8.hash(state);
            l.hash(state);
        }
        Value::Struct(record) => {
            11u8.hash(state);
            record.type_id().hash(state);
            let mut fields = record.with_fields(|fields| fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>());
            fields.sort_by(|a, b| a.0.cmp(&b.0));
            for (name, value) in fields {
                name.hash(state);
                hash_key(&value, state);
            }
        }
        Value::List(items) => {
            12u8.hash(state);
            let items = items.iter_cloned();
            items.len().hash(state);
            for item in &items {
                hash_key(item, state);
            }
        }
        Value::Enum(variant) => {
            13u8.hash(state);
            variant.type_id().hash(state);
            variant.variant_index().hash(state);
            for field in variant.fields_cloned() {
                hash_key(&field, state);
            }
        }
        Value::OptionSome(inner) => {
            14u8.hash(state);
            hash_key(&inner.get(), state);
        }
        Value::OptionNone => 15u8.hash(state),
        Value::ResultOk(inner) => {
            16u8.hash(state);
            hash_key(&inner.get(), state);
        }
        Value::ResultErr(inner) => {
            17u8.hash(state);
            hash_key(&inner.get(), state);
        }
        // A map's `==` ignores order, so its hash does too: the sum of each
        // entry's own hash.
        Value::Map(map) => {
            18u8.hash(state);
            let entries = map.entries_cloned();
            entries.len().hash(state);
            let mut sum: u64 = 0;
            for (k, v) in &entries {
                let mut entry = std::collections::hash_map::DefaultHasher::new();
                hash_key(k, &mut entry);
                hash_key(v, &mut entry);
                sum = sum.wrapping_add(entry.finish());
            }
            sum.hash(state);
        }
        // Found by `==` alone (module docs).
        _ => 0u8.hash(state),
    }
}

/// A copy of `value` that shares no record, list, map or variant with it,
/// so a key a map stores (or hands out) cannot be changed by the rest of
/// the program. Scalars are themselves values and are returned as they
/// are. A cycle (a record that reaches itself) shares the record where it
/// closes, which only a self-referential key can contain.
pub(crate) fn snapshot_key(value: &Value) -> Value {
    snapshot(value, &mut HashSet::new())
}

fn snapshot(value: &Value, open: &mut HashSet<usize>) -> Value {
    let guarded = |key: usize, open: &mut HashSet<usize>, copy: &dyn Fn(&mut HashSet<usize>) -> Value| {
        if !open.insert(key) {
            return value.clone();
        }
        let copied = copy(open);
        open.remove(&key);
        copied
    };
    match value {
        Value::Struct(record) => guarded(record.ptr_key(), open, &|open| {
            let fields = record.with_fields(|fields| fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect::<Vec<_>>());
            Value::Struct(StructValue::new(
                record.type_id(),
                record.type_name().to_string(),
                fields.into_iter().map(|(name, field)| (name, snapshot(&field, open))),
            ))
        }),
        Value::List(items) => guarded(items.ptr_key(), open, &|open| {
            Value::List(ListValue::new(items.iter_cloned().iter().map(|item| snapshot(item, open))))
        }),
        Value::Enum(variant) => guarded(variant.ptr_key(), open, &|open| {
            Value::Enum(EnumValue::new(
                variant.type_id(),
                variant.type_name().to_string(),
                variant.variant_index(),
                variant.variant_name().to_string(),
                variant.fields_cloned().iter().map(|field| snapshot(field, open)).collect(),
            ))
        }),
        Value::Map(map) => guarded(map.ptr_key(), open, &|open| {
            Value::Map(MapValue::new(map.entries_cloned().iter().map(|(k, v)| (snapshot(k, open), snapshot(v, open)))))
        }),
        Value::OptionSome(inner) => Value::OptionSome(BoxedValue::new(snapshot(&inner.get(), open))),
        Value::ResultOk(inner) => Value::ResultOk(BoxedValue::new(snapshot(&inner.get(), open))),
        Value::ResultErr(inner) => Value::ResultErr(BoxedValue::new(snapshot(&inner.get(), open))),
        other => other.clone(),
    }
}

/// Whether `value` is a key the map copies on the way in and out.
pub(crate) fn is_compound_key(value: &Value) -> bool {
    matches!(
        value,
        Value::Struct(_) | Value::List(_) | Value::Enum(_) | Value::Map(_) | Value::OptionSome(_) | Value::ResultOk(_) | Value::ResultErr(_)
    )
}

/// The whole number a `Float` holds exactly, which `==` equates with the
/// `Int` of that value.
fn whole_number(f: f64) -> Option<i64> {
    // `-0.0 == 0.0`, and both are 0.
    if f.fract() == 0.0 && f >= -9_223_372_036_854_775_808.0 && f < 9_223_372_036_854_775_808.0 {
        Some(f as i64)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::hash_map::DefaultHasher;
    use std::sync::Arc;

    fn hash_of(value: Value) -> u64 {
        let mut state = DefaultHasher::new();
        MapKey(value).hash(&mut state);
        state.finish()
    }

    /// Values `==` equates hash alike, for every pair of key types `==`
    /// relates, so a map never misses a key `==` would find.
    #[test]
    fn equal_keys_hash_alike() {
        let decimal = |text: &str| Value::Decimal(corvid_decimal::Decimal::parse(text).unwrap());
        let pairs = [
            (Value::Int(3), Value::Decimal(corvid_decimal::Decimal::from_int(3))),
            (Value::Int(3), Value::Float(3.0)),
            (decimal("1.0"), decimal("1.00")),
            (Value::Float(0.0), Value::Float(-0.0)),
            (Value::Int(-7), Value::Float(-7.0)),
            (Value::String(Arc::from("a")), Value::String(Arc::from("a"))),
            (Value::Bool(true), Value::Bool(true)),
            (Value::Nothing, Value::Nothing),
        ];
        for (a, b) in pairs {
            assert!(a == b, "{a:?} == {b:?}");
            assert_eq!(hash_of(a.clone()), hash_of(b.clone()), "{a:?} and {b:?} are equal, so they hash alike");
        }
    }

    /// Compound keys hash by their contents, so equal ones hash alike and
    /// different ones spread out; constant-time keys share one hash and are
    /// found by `==` alone.
    #[test]
    fn compound_keys_hash_by_contents_and_constant_time_keys_share_one_hash() {
        let list = |items: Vec<Value>| Value::List(crate::value::ListValue::new(items));
        assert_eq!(hash_of(list(vec![Value::Int(1)])), hash_of(list(vec![Value::Int(1)])));
        assert_ne!(hash_of(list(vec![Value::Int(1)])), hash_of(list(vec![Value::Int(2)])));
        let bytes = |b: &[u8]| Value::Bytes(corvid_crypto::Bytes::new(b));
        assert_eq!(hash_of(bytes(b"a")), hash_of(bytes(b"b")));
    }

    /// A key a map stores is a snapshot: changing the program's record
    /// afterwards leaves the map's key as it was, and a key read back out is
    /// the program's own copy.
    #[test]
    fn a_stored_key_is_a_snapshot() {
        use crate::value::{MapValue, StructValue};
        let point = StructValue::new(corvid_resolve::DefId(1), "P", [("x".to_string(), Value::Int(1))]);
        let map = MapValue::new([(Value::Struct(point.clone()), Value::Int(10))]);
        point.set_field("x", Value::Int(2));
        let one = Value::Struct(StructValue::new(corvid_resolve::DefId(1), "P", [("x".to_string(), Value::Int(1))]));
        assert_eq!(map.get_by_key(&one), Some(Value::Int(10)), "the map kept P(1)");
        assert_eq!(map.get_by_key(&Value::Struct(point)), None, "P(2) was never a key");
        let Value::Struct(read) = &map.keys_cloned()[0] else { panic!("a record key") };
        read.set_field("x", Value::Int(3));
        assert_eq!(map.get_by_key(&one), Some(Value::Int(10)), "changing a key read out does not reach the map");
    }
}
