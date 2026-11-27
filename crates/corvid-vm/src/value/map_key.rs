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
//! - **Every other key** hashes to one fixed value and is found by `==`
//!   alone, as every key was before. That covers records, lists, maps,
//!   options, results and variants, whose contents can change in place
//!   after insertion and would leave a stale hash. It also covers
//!   `Bytes` and `Secret`, whose `==` is constant-time, so a lookup reveals
//!   nothing through the hash, grounded values, whose content sits in a
//!   cell, and identity-compared values. A map of
//!   such keys is as fast as before, and no slower.

use super::Value;
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
        // Found by `==` alone (module docs).
        _ => 0u8.hash(state),
    }
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

    /// Keys whose contents can change, and constant-time keys, share one
    /// hash, so they are found by `==` alone.
    #[test]
    fn compound_and_constant_time_keys_are_found_by_equality_alone() {
        let list = |items: Vec<Value>| Value::List(crate::value::ListValue::new(items));
        assert_eq!(hash_of(list(vec![Value::Int(1)])), hash_of(list(vec![Value::Int(2)])));
        let bytes = |b: &[u8]| Value::Bytes(corvid_crypto::Bytes::new(b));
        assert_eq!(hash_of(bytes(b"a")), hash_of(bytes(b"b")));
    }
}
