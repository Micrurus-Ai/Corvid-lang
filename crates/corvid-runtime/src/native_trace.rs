#![allow(unsafe_code)]

use crate::abi::CorvidString;
use crate::ffi_bridge::{borrow_corvid_string, bridge};
use crate::runtime::Runtime;
use crate::tracing::now_ms;
use corvid_trace_schema::TraceEvent;
use serde_json::Value;

fn runtime() -> std::sync::Arc<Runtime> {
    bridge().corvid_runtime()
}

pub(crate) unsafe fn decode_trace_values(
    type_tags: &str,
    value_count: i64,
    values_ptr: i64,
) -> Vec<Value> {
    if value_count <= 0 {
        return Vec::new();
    }
    let count = value_count as usize;
    assert_eq!(
        type_tags.chars().count(),
        count,
        "native trace arg tag/value count mismatch: tags=`{type_tags}` count={count}"
    );
    assert_ne!(
        values_ptr, 0,
        "native trace values pointer was null for non-empty payload"
    );

    let base = values_ptr as usize as *const u8;
    type_tags
        .chars()
        .enumerate()
        .map(|(idx, tag)| unsafe { decode_slot_json(base.add(idx * 8), tag) })
        .collect()
}

unsafe fn decode_slot_json(slot_ptr: *const u8, tag: char) -> Value {
    match tag {
        'i' => Value::from(unsafe { *(slot_ptr as *const i64) }),
        'b' => Value::from(unsafe { *(slot_ptr as *const i64) != 0 }),
        'f' => {
            let value = unsafe { *(slot_ptr as *const f64) };
            serde_json::Number::from_f64(value)
                .map(Value::Number)
                .unwrap_or_else(|| Value::String(value.to_string()))
        }
        's' => {
            let descriptor = unsafe { *(slot_ptr as *const i64) };
            Value::String(unsafe { borrow_descriptor_string(descriptor) })
        }
        'j' => {
            // Native code encoded the value in the public JSON form
            // by its declared type (`corvid_json_encode_recorded`)
            // and stored the String descriptor pointer in the slot.
            // Parse it back into the JSON value the interpreter
            // records.
            let descriptor = unsafe { *(slot_ptr as *const i64) };
            let json_text = unsafe { borrow_descriptor_string(descriptor) };
            serde_json::from_str::<Value>(&json_text).unwrap_or_else(|err| {
                panic!(
                    "native trace 'j'-tagged slot held malformed JSON `{json_text}`: {err}"
                )
            })
        }
        other => panic!("unsupported native trace value tag `{other}`"),
    }
}

unsafe fn borrow_descriptor_string(descriptor: i64) -> String {
    let corvid: CorvidString = unsafe { std::mem::transmute((descriptor as usize) as *const u8) };
    unsafe { borrow_corvid_string(&corvid) }.to_owned()
}

fn emit(event: TraceEvent) {
    let runtime = runtime();
    let tracer = runtime.tracer();
    if tracer.is_enabled() {
        tracer.emit(event);
    }
}

#[no_mangle]
pub unsafe extern "C" fn corvid_trace_run_started(
    agent: CorvidString,
    arg_types: CorvidString,
    argc: i64,
    args_ptr: i64,
) {
    let agent_name = unsafe { borrow_corvid_string(&agent) }.to_owned();
    let arg_tags = unsafe { borrow_corvid_string(&arg_types) };
    let args = unsafe { decode_trace_values(arg_tags, argc, args_ptr) };
    let runtime = runtime();
    runtime
        .prepare_run(&agent_name, &args)
        .unwrap_or_else(|err| panic!("corvid_trace_run_started failed: {err}"));
    emit(TraceEvent::RunStarted {
        ts_ms: now_ms(),
        run_id: runtime.tracer().run_id().to_string(),
        agent: agent_name,
        args,
    });
}

/// A native run's result, as the public JSON text the compiled code
/// encoded through its plan: what the interpreter records for the run.
///
/// # Safety
/// `text` must be a live Corvid string.
#[no_mangle]
pub unsafe extern "C" fn corvid_trace_run_completed_json(text: CorvidString) {
    let text = unsafe { borrow_corvid_string(&text) };
    let result: Value = serde_json::from_str(text)
        .unwrap_or_else(|err| panic!("internal compiler error: a run's result is not JSON: {err}"));
    emit_run_completed(result);
}

fn emit_run_completed(result: Value) {
    let runtime = runtime();
    runtime
        .complete_run(true, Some(&result), None)
        .unwrap_or_else(|err| panic!("corvid_trace_run_completed failed: {err}"));
    emit(TraceEvent::RunCompleted {
        ts_ms: now_ms(),
        run_id: runtime.tracer().run_id().to_string(),
        ok: true,
        result: Some(result),
        error: None,
    });
}

#[cfg(test)]
mod tests {
    use super::decode_trace_values;
    use serde_json::json;

    /// Drop a Corvid String descriptor obtained through these tests.
    /// Matches `ffi_bridge.rs`'s extern signature byte-for-byte (the
    /// pointer width of `*const u8` and `*mut c_void` is identical on
    /// every supported target) so rustc does not warn about a
    /// duplicate `corvid_release` extern with a different signature.
    unsafe fn corvid_release(payload: *mut std::ffi::c_void) {
        extern "C" {
            fn corvid_release(descriptor: *const u8);
        }
        corvid_release(payload as *const u8);
    }

    /// Same trick for `corvid_string_from_bytes` — `ffi_bridge.rs`
    /// declares it returning `*const u8`; cast to `*mut c_void` here
    /// so the test code reads as one consistent pointer kind.
    unsafe fn corvid_string_from_bytes(bytes: *const u8, len: i64) -> *mut std::ffi::c_void {
        extern "C" {
            fn corvid_string_from_bytes(bytes: *const u8, length: i64) -> *const u8;
        }
        corvid_string_from_bytes(bytes, len) as *mut std::ffi::c_void
    }

    /// Allocate a fresh Corvid String via the runtime's regular
    /// allocator, the descriptor layout native code stores in a slot.
    unsafe fn make_string(s: &str) -> *mut std::ffi::c_void {
        corvid_string_from_bytes(s.as_ptr(), s.len() as i64)
    }

    #[test]
    fn decode_trace_values_handles_scalar_tags() {
        let ints = [7_i64, 1_i64];
        let values = unsafe { decode_trace_values("ib", 2, ints.as_ptr() as usize as i64) };
        assert_eq!(values, vec![json!(7), json!(true)]);
    }

    #[test]
    fn decode_trace_values_handles_float_bits() {
        let floats = [3.5_f64];
        let values =
            unsafe { decode_trace_values("f", 1, floats.as_ptr() as usize as i64) };
        assert_eq!(values, vec![json!(3.5)]);
    }

    #[test]
    fn decode_trace_values_handles_j_tag_via_descriptor_pointer() {
        // A `j` slot holds a String descriptor of public JSON text, as
        // native code writes it; the decoder parses it back into the
        // JSON value.
        unsafe {
            let json_desc = make_string(r#"{"id":"r-001"}"#);
            let slot: i64 = json_desc as usize as i64;
            let values = decode_trace_values("j", 1, &slot as *const i64 as usize as i64);
            corvid_release(json_desc);
            assert_eq!(values, vec![json!({"id": "r-001"})]);
        }
    }
}
