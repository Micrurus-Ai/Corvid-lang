//! `T.from_json(text)` for native programs.
//!
//! Compiled code passes the JSON text, the decode plan the compiler
//! wrote for `T`, and a table of typeinfo pointers the plan indexes.
//! Decoding runs the shared public JSON decoder (`corvid-public-json`),
//! so a native program accepts exactly the JSON the interpreter accepts
//! and fails with the same path, expectation and finding. This module
//! only builds native heap values for it, laid out as
//! [`crate::native_layout`] describes.

use crate::abi::CorvidString;
use crate::native_layout::{
    record_payload_bytes, variant_payload_bytes, OPTION_PAYLOAD_BYTES, OPTION_PAYLOAD_OFFSET,
    RESULT_PAYLOAD_BYTES, RESULT_PAYLOAD_OFFSET, RESULT_TAG_ERR, RESULT_TAG_OFFSET, RESULT_TAG_OK, SLOT_BYTES,
};
use corvid_ir::IrType;
use corvid_public_json::{DecodePlan, JsonDecodeError, Shape, UnknownFields, ValueBuilder};
use corvid_types::Type;
use serde_json::Value as J;
use std::collections::HashMap;
use std::ffi::c_void;
use std::sync::{Arc, Mutex, OnceLock};

extern "C" {
    fn corvid_alloc_typed(payload_bytes: i64, typeinfo: *const c_void) -> *mut u8;
    fn corvid_release(payload: *const u8);
    fn corvid_string_from_bytes(bytes: *const u8, length: i64) -> *const u8;
    fn corvid_list_new(capacity: i64, typeinfo: *const c_void) -> *mut c_void;
    fn corvid_list_append(list: *mut c_void, slot: i64);
    fn corvid_ordered_map_new_typed(capacity: i64, owns_values: i64) -> *mut c_void;
    fn corvid_ordered_map_set(map: *mut c_void, key: *mut c_void, value: *mut c_void);
}

/// Decode `text` into the plan's root type. On success the value is
/// written to `payload` and the Ok tag is returned; on failure a
/// `JsonError` record is written there and the Err tag is returned.
/// Either way the caller owns what `payload` now holds.
///
/// # Safety
/// `text` must be a live Corvid string. `plan` must point to `plan_len`
/// bytes of a decode plan written by the compiler, which stay valid for
/// the program's lifetime, and `typeinfos` to the table that plan
/// indexes. `payload` must be a writable, aligned slot.
#[no_mangle]
pub unsafe extern "C" fn corvid_json_decode_typed(
    text: CorvidString,
    plan: *const u8,
    plan_len: i64,
    typeinfos: *const *const c_void,
    strict: i64,
    payload: *mut i64,
) -> i64 {
    let plan = unsafe { cached_plan(plan, plan_len) };
    let types_by_id = plan.types_by_id();
    let unknown_fields = if strict != 0 { UnknownFields::Reject } else { UnknownFields::Ignore };
    let mut builder = NativeValues { plan: &plan, typeinfos };
    let text = unsafe { text.as_str() };
    match corvid_public_json::decode_text(text, &plan.root, &types_by_id, unknown_fields, &mut builder) {
        Ok(value) => {
            unsafe { payload.write(value.bits) };
            RESULT_TAG_OK
        }
        Err(error) => {
            let record = builder.json_error(&error);
            unsafe { payload.write(record.bits) };
            RESULT_TAG_ERR
        }
    }
}

/// Plans live in the program's read-only data, so their address
/// identifies them; each is parsed once.
unsafe fn cached_plan(plan: *const u8, plan_len: i64) -> Arc<DecodePlan> {
    static PLANS: OnceLock<Mutex<HashMap<usize, Arc<DecodePlan>>>> = OnceLock::new();
    let plans = PLANS.get_or_init(|| Mutex::new(HashMap::new()));
    let mut plans = plans.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    plans
        .entry(plan as usize)
        .or_insert_with(|| {
            let bytes = unsafe { std::slice::from_raw_parts(plan, plan_len as usize) };
            let text = std::str::from_utf8(bytes).expect("the compiler writes decode plans as UTF-8");
            Arc::new(DecodePlan::from_text(text).unwrap_or_else(|reason| {
                panic!("internal compiler error: a native decode plan does not read back: {reason}")
            }))
        })
        .clone()
}

/// One native value: its slot bits, and whether it is a reference the
/// builder owns (and must release if the decode fails).
#[derive(Clone, Copy)]
struct Slot {
    bits: i64,
    owned: bool,
}

impl Slot {
    fn scalar(bits: i64) -> Self {
        Self { bits, owned: false }
    }
    fn reference(pointer: *const c_void) -> Self {
        Self { bits: pointer as i64, owned: true }
    }
}

struct NativeValues<'a> {
    plan: &'a DecodePlan,
    typeinfos: *const *const c_void,
}

impl NativeValues<'_> {
    fn typeinfo(&self, shape: &Shape) -> Option<*const c_void> {
        let index = *self.plan.shapes.get(shape)?;
        Some(unsafe { *self.typeinfos.add(index as usize) })
    }

    fn required_typeinfo(&self, shape: &Shape) -> *const c_void {
        self.typeinfo(shape).unwrap_or_else(|| {
            panic!("internal compiler error: the native decode plan has no typeinfo for {shape:?}")
        })
    }

    /// Allocate a payload of `bytes` and write `slots` from `first_offset`.
    fn allocate(&self, shape: &Shape, bytes: i64, first_offset: i32, slots: &[i64]) -> Slot {
        let payload = unsafe { corvid_alloc_typed(bytes, self.required_typeinfo(shape)) };
        for (index, bits) in slots.iter().enumerate() {
            let offset = first_offset as usize + index * SLOT_BYTES as usize;
            unsafe { payload.add(offset).cast::<i64>().write(*bits) };
        }
        Slot::reference(payload.cast())
    }

    fn native_string(&self, text: &str) -> Slot {
        Slot::reference(unsafe { corvid_string_from_bytes(text.as_ptr(), text.len() as i64) }.cast())
    }

    /// The `JsonError` record for a failed decode, fields in the order
    /// the checker declares them.
    fn json_error(&mut self, error: &JsonDecodeError) -> Slot {
        let message = error.message();
        let fields: Vec<i64> = corvid_types::json_error_fields()
            .iter()
            .map(|(name, _)| {
                let text = match *name {
                    "path" => error.path.as_str(),
                    "expected" => error.expected.as_str(),
                    "found" => error.found.as_str(),
                    "message" => message.as_str(),
                    other => unreachable!("JsonError field `{other}` has no runtime value"),
                };
                self.native_string(text).bits
            })
            .collect();
        let shape = Shape::Record(corvid_types::JSON_ERROR_TYPE_ID);
        self.allocate(&shape, record_payload_bytes(fields.len()), 0, &fields)
    }
}

impl ValueBuilder for NativeValues<'_> {
    type Value = Slot;

    fn int(&mut self, value: i64) -> Slot {
        Slot::scalar(value)
    }
    fn float(&mut self, value: f64) -> Slot {
        Slot::scalar(value.to_bits() as i64)
    }
    fn string(&mut self, value: &str) -> Slot {
        self.native_string(value)
    }
    fn bool(&mut self, value: bool) -> Slot {
        Slot::scalar(i64::from(value))
    }
    fn nothing(&mut self) -> Slot {
        Slot::scalar(0)
    }
    /// `None` is the null pointer in both native option forms.
    fn none(&mut self, _option: &Type) -> Slot {
        Slot::scalar(0)
    }
    /// The plan gives an option a typeinfo exactly when the native
    /// backend boxes it; otherwise the payload itself is the option.
    fn some(&mut self, option: &Type, value: Slot) -> Slot {
        let shape = Shape::Type(option.clone());
        if self.typeinfo(&shape).is_some() {
            self.allocate(&shape, OPTION_PAYLOAD_BYTES, OPTION_PAYLOAD_OFFSET, &[value.bits])
        } else {
            value
        }
    }
    fn list(&mut self, list: &Type, items: Vec<Slot>) -> Slot {
        let info = self.required_typeinfo(&Shape::Type(list.clone()));
        let native = unsafe { corvid_list_new(items.len() as i64, info) };
        for item in items {
            unsafe { corvid_list_append(native, item.bits) };
        }
        Slot::reference(native)
    }
    fn map(&mut self, map: &Type, entries: Vec<(Slot, Slot)>) -> Slot {
        let owns_values = self.plan.owning_maps.contains(map);
        let native = unsafe { corvid_ordered_map_new_typed(entries.len() as i64, i64::from(owns_values)) };
        for (key, value) in entries {
            unsafe { corvid_ordered_map_set(native, key.bits as *mut c_void, value.bits as *mut c_void) };
        }
        Slot::reference(native)
    }
    fn ok(&mut self, result: &Type, value: Slot) -> Slot {
        self.result(result, RESULT_TAG_OK, value)
    }
    fn err(&mut self, result: &Type, value: Slot) -> Slot {
        self.result(result, RESULT_TAG_ERR, value)
    }
    fn record(&mut self, record: &IrType, fields: Vec<Slot>) -> Slot {
        let bits: Vec<i64> = fields.iter().map(|f| f.bits).collect();
        self.allocate(&Shape::Record(record.id), record_payload_bytes(bits.len()), 0, &bits)
    }
    fn variant(&mut self, sum: &IrType, index: usize, fields: Vec<Slot>) -> Slot {
        let bits: Vec<i64> = std::iter::once(index as i64).chain(fields.iter().map(|f| f.bits)).collect();
        let shape = Shape::Variant(sum.id, index as u32);
        self.allocate(&shape, variant_payload_bytes(fields.len()), 0, &bits)
    }
    fn json(&mut self, _json: &J) -> Slot {
        panic!("internal compiler error: a native decode plan targets `JsonValue`, which has no native form")
    }
    fn discard(&mut self, value: Slot) {
        if value.owned && value.bits != 0 {
            unsafe { corvid_release(value.bits as *const u8) };
        }
    }
}

impl NativeValues<'_> {
    fn result(&mut self, result: &Type, tag: i64, value: Slot) -> Slot {
        let shape = Shape::Type(result.clone());
        let payload = unsafe { corvid_alloc_typed(RESULT_PAYLOAD_BYTES, self.required_typeinfo(&shape)) };
        unsafe {
            payload.add(RESULT_TAG_OFFSET as usize).cast::<i64>().write(tag);
            payload.add(RESULT_PAYLOAD_OFFSET as usize).cast::<i64>().write(value.bits);
        }
        Slot::reference(payload.cast())
    }
}
