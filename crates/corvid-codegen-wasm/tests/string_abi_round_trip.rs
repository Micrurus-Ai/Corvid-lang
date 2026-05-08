//! End-to-end round-trip tests for the WASM String ABI, exercising
//! the full chain from generated module through wasmtime
//! instantiation to the user-visible boundary.
//!
//! Each test simulates exactly what the generated JS loader does
//! at runtime:
//!
//!   1. Allocate input bytes via `corvid_alloc(len)` — this hands
//!      back a payload pointer into linear memory.
//!   2. Write the encoded UTF-8 bytes into linear memory at that
//!      offset (the JS side does `Uint8Array(memory.buffer, ptr,
//!      len).set(bytes)`; here we write through `Memory::data_mut`).
//!   3. Call the agent's exported function with `(ptr, len)` as
//!      the two `i32` arguments — wasmtime hands the multi-value
//!      `(i32, i32)` return back as a Rust tuple.
//!   4. Read the returned bytes out of linear memory and decode
//!      as UTF-8 (TextDecoder-equivalent).
//!   5. Free the input via `corvid_free(ptr, len)` — the agent's
//!      return may alias the input, but the decoded string is
//!      already a separate copy in the Rust stack so freeing is
//!      safe.
//!
//! The tests cover the four scope axes: ASCII pass-through,
//! multi-byte UTF-8 pass-through, string-literal return (no
//! parameters), and alloc/free non-leak across many iterations
//! (the same property the allocator integration tests verify
//! one level down, but here through the full agent surface).

use corvid_codegen_wasm::emit_wasm_artifacts;
use corvid_ir::lower;
use corvid_resolve::resolve;
use corvid_syntax::{lex, parse_file};
use corvid_types::typecheck;
use wasmtime::{Engine, Linker, Module, Store, TypedFunc};

fn build_module_bytes(src: &str, module_name: &str) -> Vec<u8> {
    let tokens = lex(src).expect("lex");
    let (parsed, parse_errors) = parse_file(&tokens);
    assert!(parse_errors.is_empty(), "parse: {parse_errors:?}");
    let resolved = resolve(&parsed);
    assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
    let checked = typecheck(&parsed, &resolved);
    assert!(checked.errors.is_empty(), "typecheck: {:?}", checked.errors);
    let ir = lower(&parsed, &resolved, &checked);
    let artifacts = emit_wasm_artifacts(&ir, module_name).expect("emit");
    wasmparser::Validator::new()
        .validate_all(&artifacts.wasm)
        .expect("module must validate");
    artifacts.wasm
}

#[test]
fn shout_round_trips_ascii() {
    let src = "agent shout(msg: String) -> String:\n    return msg\n";
    let bytes = build_module_bytes(src, "shout");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes).expect("compile");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &module).expect("instantiate");

    let memory = instance.get_memory(&mut store, "memory").expect("memory");
    let alloc: TypedFunc<i32, i32> = instance
        .get_typed_func(&mut store, "corvid_alloc")
        .expect("corvid_alloc");
    let free: TypedFunc<(i32, i32), ()> = instance
        .get_typed_func(&mut store, "corvid_free")
        .expect("corvid_free");
    let shout: TypedFunc<(i32, i32), (i32, i32)> = instance
        .get_typed_func(&mut store, "shout")
        .expect("shout export");

    let input = "hello, world";
    let bytes_in = input.as_bytes();
    let ptr_in = alloc
        .call(&mut store, bytes_in.len() as i32)
        .expect("alloc input");
    memory.data_mut(&mut store)[ptr_in as usize..ptr_in as usize + bytes_in.len()]
        .copy_from_slice(bytes_in);

    let (ptr_out, len_out) = shout
        .call(&mut store, (ptr_in, bytes_in.len() as i32))
        .expect("call shout");

    let bytes_out =
        &memory.data(&mut store)[ptr_out as usize..ptr_out as usize + len_out as usize];
    let decoded = std::str::from_utf8(bytes_out).expect("UTF-8 decode");
    assert_eq!(decoded, input);

    free.call(&mut store, (ptr_in, bytes_in.len() as i32))
        .expect("free input");
}

#[test]
fn shout_round_trips_multi_byte_utf8() {
    let src = "agent shout(msg: String) -> String:\n    return msg\n";
    let bytes = build_module_bytes(src, "shout_utf8");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes).expect("compile");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &module).expect("instantiate");

    let memory = instance.get_memory(&mut store, "memory").expect("memory");
    let alloc: TypedFunc<i32, i32> = instance
        .get_typed_func(&mut store, "corvid_alloc")
        .expect("corvid_alloc");
    let free: TypedFunc<(i32, i32), ()> = instance
        .get_typed_func(&mut store, "corvid_free")
        .expect("corvid_free");
    let shout: TypedFunc<(i32, i32), (i32, i32)> = instance
        .get_typed_func(&mut store, "shout")
        .expect("shout export");

    // Multi-byte UTF-8: 'é' is 2 bytes (0xC3 0xA9), '🦀' is 4 bytes
    // (0xF0 0x9F 0xA6 0x80). Pass-through must preserve byte-exact
    // content end to end.
    let input = "héllo 🦀";
    let bytes_in = input.as_bytes();
    assert!(bytes_in.len() > input.chars().count());

    let ptr_in = alloc
        .call(&mut store, bytes_in.len() as i32)
        .expect("alloc input");
    memory.data_mut(&mut store)[ptr_in as usize..ptr_in as usize + bytes_in.len()]
        .copy_from_slice(bytes_in);

    let (ptr_out, len_out) = shout
        .call(&mut store, (ptr_in, bytes_in.len() as i32))
        .expect("call shout");

    assert_eq!(len_out as usize, bytes_in.len(), "byte length must match");

    let bytes_out =
        &memory.data(&mut store)[ptr_out as usize..ptr_out as usize + len_out as usize];
    let decoded = std::str::from_utf8(bytes_out).expect("UTF-8 decode");
    assert_eq!(decoded, input);

    free.call(&mut store, (ptr_in, bytes_in.len() as i32))
        .expect("free input");
}

#[test]
fn agent_returns_string_literal() {
    // Slice 20n-B-2b end-to-end check: an agent that returns a
    // string literal places the bytes in the data section, the
    // agent body emits `(I32Const(addr), I32Const(len))`, and the
    // return is a multi-value pair pointing into linear memory.
    let src = "agent greet() -> String:\n    return \"hello\"\n";
    let bytes = build_module_bytes(src, "greet");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes).expect("compile");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &module).expect("instantiate");

    let memory = instance.get_memory(&mut store, "memory").expect("memory");
    let greet: TypedFunc<(), (i32, i32)> = instance
        .get_typed_func(&mut store, "greet")
        .expect("greet export");

    let (ptr, len) = greet.call(&mut store, ()).expect("call greet");
    let bytes_out = &memory.data(&mut store)[ptr as usize..ptr as usize + len as usize];
    let decoded = std::str::from_utf8(bytes_out).expect("UTF-8 decode");
    assert_eq!(decoded, "hello");
}

#[test]
fn many_round_trips_do_not_grow_memory() {
    // Long-running churn: repeatedly call a String pass-through
    // agent, each iteration alloc+write+call+decode+free. With the
    // free-list allocator from commit 1, the memory page count
    // should stay at the initial 1 page across hundreds of
    // iterations. Without coalescing or with a leak, page count
    // would grow.
    let src = "agent shout(msg: String) -> String:\n    return msg\n";
    let bytes = build_module_bytes(src, "shout_churn");
    let engine = Engine::default();
    let module = Module::new(&engine, &bytes).expect("compile");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance = linker.instantiate(&mut store, &module).expect("instantiate");

    let memory = instance.get_memory(&mut store, "memory").expect("memory");
    let alloc: TypedFunc<i32, i32> = instance
        .get_typed_func(&mut store, "corvid_alloc")
        .expect("corvid_alloc");
    let free: TypedFunc<(i32, i32), ()> = instance
        .get_typed_func(&mut store, "corvid_free")
        .expect("corvid_free");
    let shout: TypedFunc<(i32, i32), (i32, i32)> = instance
        .get_typed_func(&mut store, "shout")
        .expect("shout export");

    let initial_pages = memory.size(&mut store);
    let input = "moderately-sized payload to exercise the allocator's reuse path";

    for _ in 0..200 {
        let bytes_in = input.as_bytes();
        let ptr_in = alloc
            .call(&mut store, bytes_in.len() as i32)
            .expect("alloc");
        memory.data_mut(&mut store)[ptr_in as usize..ptr_in as usize + bytes_in.len()]
            .copy_from_slice(bytes_in);
        let (_, _) = shout
            .call(&mut store, (ptr_in, bytes_in.len() as i32))
            .expect("call");
        free.call(&mut store, (ptr_in, bytes_in.len() as i32))
            .expect("free");
    }

    let final_pages = memory.size(&mut store);
    assert_eq!(
        initial_pages, final_pages,
        "200-iteration churn must reuse memory; pages went {initial_pages} -> {final_pages}",
    );
}
