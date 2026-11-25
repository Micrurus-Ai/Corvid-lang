//! `T.from_json(text)` and `value.to_json()` for native programs.
//!
//! Compiled code passes the JSON text or the value, the plan the
//! compiler wrote for the type, and a table of typeinfo pointers the
//! plan indexes. Both directions run the shared public JSON mapping
//! (`corvid-public-json`), so a native program accepts and produces
//! exactly the JSON the interpreter does. This module only builds and
//! reads native heap values for it, laid out as
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

/// A runtime-owned call from native code (native dispatch N1): run
/// `Runtime::call_tool(name, args)`, the path the interpreter takes, and
/// decode its answer into the plan's root type, writing it to `payload`.
/// The arguments are public JSON texts the compiled code encoded. A
/// connector answer that does not decode becomes the operation's
/// `on failure` variant, as in the interpreter; any other failure stops
/// the program with the runtime's message, as the interpreter does.
///
/// # Safety
/// `name` and the `arg_count` strings at `args` must be live Corvid
/// strings; `plan`, `plan_len` and `typeinfos` as for
/// [`corvid_json_decode_typed`]; `payload` a writable, aligned slot.
#[no_mangle]
pub unsafe extern "C" fn corvid_runtime_call(
    name: CorvidString,
    args: *const CorvidString,
    arg_count: i64,
    plan: *const u8,
    plan_len: i64,
    typeinfos: *const *const c_void,
    payload: *mut i64,
) -> i64 {
    let name = unsafe { name.as_str() }.to_string();
    let mut values = Vec::with_capacity(arg_count.max(0) as usize);
    for index in 0..arg_count.max(0) as usize {
        let text = unsafe { (*args.add(index)).as_str() };
        values.push(serde_json::from_str::<J>(text).unwrap_or_else(|err| {
            panic!("internal compiler error: native code encoded an argument of `{name}` that is not JSON: {err}")
        }));
    }
    let state = crate::ffi_bridge::bridge();
    let runtime = state.corvid_runtime();
    let call_name = name.clone();
    let call_runtime = runtime.clone();
    let answer = state
        .tokio_handle()
        .block_on(async move { call_runtime.call_tool(&call_name, values).await });
    let answer = match answer {
        Ok(answer) => answer,
        Err(err) => {
            crate::ffi_bridge::panic_if_replay_runtime_error(&format!("corvid: `{name}`"), &err);
            eprintln!("corvid: `{name}`: {err}");
            std::process::exit(1);
        }
    };
    let plan = unsafe { cached_plan(plan, plan_len) };
    let types_by_id = plan.types_by_id();
    let mut builder = NativeValues { plan: &plan, typeinfos };
    let decoded = match corvid_public_json::decode(&answer, &plan.root, &types_by_id, UnknownFields::Ignore, &mut builder) {
        Ok(value) => value,
        Err(error) => match runtime.connector_failure_variant(&name) {
            Some((variant, takes_reason)) => {
                let reason = format!("the response does not match the declared type {}", error.message());
                let failure = crate::connectors::failure_result(&variant, takes_reason, &reason);
                corvid_public_json::decode(&failure, &plan.root, &types_by_id, UnknownFields::Ignore, &mut builder)
                    .unwrap_or_else(|e| panic!("internal error: `{name}`'s failure value does not decode: {}", e.message()))
            }
            None => {
                eprintln!("corvid: connector `{name}`: the response does not match the declared type {}", error.message());
                std::process::exit(1);
            }
        },
    };
    unsafe { payload.write(decoded.bits) };
    RESULT_TAG_OK
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
    /// B06 — every text-scalar type is a native String holding its
    /// canonical text.
    fn text_scalar(&mut self, value: corvid_public_json::text_scalar::TextScalar) -> Slot {
        self.native_string(&value.to_text())
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

/// Encode `value`, whose type is the plan's root, as compact public JSON
/// text with sorted keys. The value is borrowed; the caller owns the
/// returned string. A value with no JSON form, which the checker leaves
/// only for a NaN or infinite `Float`, stops the program with the
/// interpreter's `to_json:` message.
///
/// # Safety
/// `value` must be a live native value of the plan's root type, and
/// `plan`, `plan_len` and `typeinfos` must be as for
/// [`corvid_json_decode_typed`].
#[no_mangle]
pub unsafe extern "C" fn corvid_json_encode_typed(
    value: i64,
    plan: *const u8,
    plan_len: i64,
    typeinfos: *const *const c_void,
) -> CorvidString {
    let plan = unsafe { cached_plan(plan, plan_len) };
    let types_by_id = plan.types_by_id();
    let mut reader = NativeValues { plan: &plan, typeinfos };
    match corvid_public_json::encode_text(&Slot::scalar(value), &plan.root, &types_by_id, &mut reader) {
        Ok(text) => crate::ffi_bridge::string_from_rust(text),
        Err(reason) => {
            eprintln!("corvid: to_json: {reason}");
            std::process::exit(1);
        }
    }
}

/// Read the `index`th 8-byte slot of the payload at `pointer`, starting
/// `first_offset` bytes in.
unsafe fn slot_at(pointer: i64, first_offset: i32, index: usize) -> i64 {
    let address = pointer as usize + first_offset as usize + index * SLOT_BYTES as usize;
    unsafe { (address as *const i64).read() }
}

impl corvid_public_json::ValueReader for NativeValues<'_> {
    type Value = Slot;

    fn int(&mut self, value: &Slot) -> Result<i64, String> {
        Ok(value.bits)
    }
    fn float(&mut self, value: &Slot) -> Result<f64, String> {
        Ok(f64::from_bits(value.bits as u64))
    }
    /// A Bool occupies the low byte of its slot.
    fn bool(&mut self, value: &Slot) -> Result<bool, String> {
        Ok(value.bits & 0xff != 0)
    }
    fn string(&mut self, value: &Slot) -> Result<String, String> {
        // SAFETY: a native String slot holds a live descriptor pointer;
        // `CorvidString` has no destructor, so viewing it releases nothing.
        let text: CorvidString = unsafe { std::mem::transmute(value.bits as *const u8) };
        Ok(unsafe { text.as_str() }.to_string())
    }
    fn option(&mut self, option: &Type, value: &Slot) -> Result<Option<Slot>, String> {
        if value.bits == 0 {
            return Ok(None);
        }
        let boxed = self.typeinfo(&Shape::Type(option.clone())).is_some();
        Ok(Some(if boxed { Slot::scalar(unsafe { slot_at(value.bits, OPTION_PAYLOAD_OFFSET, 0) }) } else { *value }))
    }
    /// A list payload is `{length, data, capacity}`.
    fn list(&mut self, _list: &Type, value: &Slot) -> Result<Vec<Slot>, String> {
        let length = unsafe { slot_at(value.bits, 0, 0) } as usize;
        let data = unsafe { slot_at(value.bits, 0, 1) };
        Ok((0..length).map(|index| Slot::scalar(unsafe { slot_at(data, 0, index) })).collect())
    }
    /// A map payload points at `{count, key, value, key, value, ...}`.
    fn map(&mut self, _map: &Type, value: &Slot) -> Result<Vec<(Slot, Slot)>, String> {
        let entries = unsafe { slot_at(value.bits, 0, 0) };
        let count = unsafe { slot_at(entries, 0, 0) } as usize;
        Ok((0..count)
            .map(|index| unsafe {
                (Slot::scalar(slot_at(entries, SLOT_BYTES, 2 * index)), Slot::scalar(slot_at(entries, SLOT_BYTES, 2 * index + 1)))
            })
            .collect())
    }
    fn result(&mut self, _result: &Type, value: &Slot) -> Result<(bool, Slot), String> {
        let tag = unsafe { slot_at(value.bits, RESULT_TAG_OFFSET, 0) };
        Ok((tag == RESULT_TAG_OK, Slot::scalar(unsafe { slot_at(value.bits, RESULT_PAYLOAD_OFFSET, 0) })))
    }
    fn record(&mut self, record: &IrType, value: &Slot) -> Result<Vec<Slot>, String> {
        Ok((0..record.fields.len()).map(|index| Slot::scalar(unsafe { slot_at(value.bits, 0, index) })).collect())
    }
    /// A variant payload is the tag slot, then the variant's fields.
    fn variant(&mut self, sum: &IrType, value: &Slot) -> Result<(usize, Vec<Slot>), String> {
        let index = unsafe { slot_at(value.bits, 0, 0) } as usize;
        let variant = sum.variants.get(index).ok_or_else(|| format!("`{}` has no variant {index}", sum.name))?;
        let fields = (0..variant.fields.len()).map(|field| Slot::scalar(unsafe { slot_at(value.bits, 0, field + 1) })).collect();
        Ok((index, fields))
    }
    fn grounded(&mut self, _grounded: &Type, _value: &Slot) -> Result<(Slot, Vec<J>), String> {
        Err("native code keeps no provenance for a `Grounded` value; encode it on the interpreter".into())
    }
    fn json(&mut self, _value: &Slot) -> Result<J, String> {
        Err("`JsonValue` has no native form".into())
    }
    /// B06 — a native text-scalar slot holds its canonical text, read
    /// back with the parser the JSON form uses.
    fn text_scalar(&mut self, ty: &corvid_types::Type, value: &Slot) -> Result<corvid_public_json::text_scalar::TextScalar, String> {
        // SAFETY: a native text-scalar slot holds a live descriptor
        // pointer; `CorvidString` has no destructor.
        let text: CorvidString = unsafe { std::mem::transmute(value.bits as *const u8) };
        let text = unsafe { super::strings::borrow_corvid_string(&text) };
        corvid_public_json::text_scalar::parse(ty, text).map_err(|error| {
            format!("internal compiler error: a native `{}` holds `{text}`: {error}", ty.display_name())
        })
    }
}
