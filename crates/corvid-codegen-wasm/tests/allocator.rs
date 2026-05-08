//! Integration tests for the WASM allocator emitted by 20n-B
//! commit 1.
//!
//! Strategy: build a trivial Corvid module (just a scalar agent so
//! the existing emit pipeline runs), instantiate it via wasmtime,
//! exercise the exported `corvid_alloc` and `corvid_free` functions
//! directly. The agent itself is a no-op for these tests; we only
//! care about the allocator behaviour.
//!
//! What we verify:
//! - alloc returns a non-zero, payload-pointer-aligned address.
//! - alloc-then-free-then-alloc reuses the freed slot (free list
//!   walk works).
//! - Two adjacent frees coalesce into a single larger free block —
//!   we detect this by checking that a subsequent alloc whose size
//!   exceeds either individual freed block's size still succeeds
//!   without bumping past the original heap-top watermark.
//! - Heap can grow past the initial 64 KiB page when allocations
//!   exceed initial memory.
//! - `corvid_alloc(0)` returns a valid 4-byte slot (rounding to
//!   `MIN_PAYLOAD`).

use corvid_codegen_wasm::emit_wasm_artifacts;
use corvid_ir::lower;
use corvid_resolve::resolve;
use corvid_syntax::{lex, parse_file};
use corvid_types::typecheck;
use wasmtime::{Engine, Instance, Linker, Module, Store, TypedFunc};

const NOOP_SRC: &str = "agent noop(x: Int) -> Int:\n    return x\n";

struct Allocator {
    store: Store<()>,
    alloc: TypedFunc<i32, i32>,
    free: TypedFunc<(i32, i32), ()>,
    memory: wasmtime::Memory,
}

impl Allocator {
    fn alloc(&mut self, size: i32) -> i32 {
        self.alloc
            .call(&mut self.store, size)
            .expect("corvid_alloc call")
    }

    fn free(&mut self, ptr: i32, size: i32) {
        self.free
            .call(&mut self.store, (ptr, size))
            .expect("corvid_free call");
    }

    fn write_byte(&mut self, ptr: i32, value: u8) {
        let data = self.memory.data_mut(&mut self.store);
        data[ptr as usize] = value;
    }

    fn read_byte(&mut self, ptr: i32) -> u8 {
        let data = self.memory.data(&mut self.store);
        data[ptr as usize]
    }

    fn read_i32(&mut self, ptr: i32) -> i32 {
        let data = self.memory.data(&mut self.store);
        let bytes = &data[ptr as usize..(ptr + 4) as usize];
        i32::from_le_bytes(bytes.try_into().unwrap())
    }

    fn memory_pages(&mut self) -> u64 {
        self.memory.size(&mut self.store)
    }
}

fn build_allocator() -> Allocator {
    let tokens = lex(NOOP_SRC).expect("lex");
    let (parsed, parse_errors) = parse_file(&tokens);
    assert!(parse_errors.is_empty(), "parse: {parse_errors:?}");
    let resolved = resolve(&parsed);
    assert!(resolved.errors.is_empty(), "resolve: {:?}", resolved.errors);
    let checked = typecheck(&parsed, &resolved);
    assert!(checked.errors.is_empty(), "typecheck: {:?}", checked.errors);
    let ir = lower(&parsed, &resolved, &checked);
    let artifacts = emit_wasm_artifacts(&ir, "alloc_test").expect("emit");
    wasmparser::Validator::new()
        .validate_all(&artifacts.wasm)
        .expect("module must validate");

    let engine = Engine::default();
    let module = Module::new(&engine, &artifacts.wasm).expect("compile");
    let linker: Linker<()> = Linker::new(&engine);
    let mut store = Store::new(&engine, ());
    let instance: Instance = linker.instantiate(&mut store, &module).expect("instantiate");

    let alloc: TypedFunc<i32, i32> = instance
        .get_typed_func(&mut store, "corvid_alloc")
        .expect("corvid_alloc must be exported");
    let free: TypedFunc<(i32, i32), ()> = instance
        .get_typed_func(&mut store, "corvid_free")
        .expect("corvid_free must be exported");
    let memory = instance
        .get_memory(&mut store, "memory")
        .expect("memory must be exported");

    Allocator {
        store,
        alloc,
        free,
        memory,
    }
}

#[test]
fn alloc_returns_non_zero_payload_pointer() {
    let mut a = build_allocator();
    let p = a.alloc(8);
    assert!(p >= 12, "payload should be past header + sentinel; got {p}");
}

#[test]
fn alloc_zero_rounds_up_to_min_payload() {
    let mut a = build_allocator();
    let p = a.alloc(0);
    assert!(p > 0, "alloc(0) must still return a usable pointer");
    // The header before the payload should record at least 4 bytes
    // (MIN_PAYLOAD).
    let header = a.read_i32(p - 4);
    assert!(
        header >= 4,
        "alloc(0) must materialise at least 4-byte payload; header was {header}"
    );
}

#[test]
fn write_then_read_round_trips() {
    let mut a = build_allocator();
    let p = a.alloc(16);
    for i in 0..16 {
        a.write_byte(p + i, (i as u8).wrapping_add(0xA0));
    }
    for i in 0..16 {
        assert_eq!(a.read_byte(p + i), (i as u8).wrapping_add(0xA0));
    }
}

#[test]
fn free_then_alloc_reuses_slot() {
    let mut a = build_allocator();
    let p1 = a.alloc(32);
    a.free(p1, 32);
    let p2 = a.alloc(32);
    assert_eq!(
        p1, p2,
        "free-then-alloc(same size) should reuse the original block"
    );
}

#[test]
fn forward_coalesce_lets_a_larger_alloc_fit_into_two_freed_blocks() {
    let mut a = build_allocator();
    // Two consecutive small allocations.
    let p1 = a.alloc(16);
    let p2 = a.alloc(16);
    assert!(p2 > p1, "second alloc should be at a higher address");
    let initial_top_marker = p2 + 16; // approximate heap-top watermark

    // Free in order such that the second free can forward-coalesce
    // with the first. p1 freed first, then p2: pass-2 (backward
    // coalesce) when freeing p2 should see p1's free block ending
    // exactly at p2's header, and merge.
    a.free(p1, 16);
    a.free(p2, 16);

    // A 28-byte allocation should fit into the coalesced free block
    // (roughly 16 + 4 + 16 = 36 bytes payload available after merge,
    // minus the size header that's now part of the coalesced
    // payload). It should NOT need to bump the heap further.
    let p3 = a.alloc(28);
    assert!(
        p3 <= initial_top_marker,
        "after coalesce, a 28-byte alloc should reuse the merged free block (got {p3}, watermark was {initial_top_marker})",
    );
}

#[test]
fn backward_coalesce_runs_when_predecessor_already_free() {
    // This exercises pass-2 of the coalescer (the just-freed block's
    // predecessor in address order is already free). Free p2 first,
    // then p1 — when p1 is freed, the coalescer's forward-pass sees
    // a free block at p1.end == p2.header and merges p2 into the
    // newly-inserted p1.
    let mut a = build_allocator();
    let p1 = a.alloc(16);
    let p2 = a.alloc(16);
    assert!(p2 > p1);

    a.free(p2, 16);
    a.free(p1, 16);

    // A 30-byte alloc should fit. Same reasoning as the
    // forward-coalesce test.
    let p3 = a.alloc(30);
    assert!(
        p3 < p2 + 32,
        "after backward coalesce, a 30-byte alloc should reuse the merged block; got {p3}"
    );
}

#[test]
fn many_alloc_free_cycles_do_not_leak_pages() {
    // Long-running churn pattern: allocate, free, allocate, free,
    // ... If the allocator failed to reclaim freed slots it would
    // grow memory. With proper free-list reuse, page count should
    // stay at the initial 1 page.
    let mut a = build_allocator();
    let initial_pages = a.memory_pages();
    for _ in 0..1000 {
        let p = a.alloc(64);
        a.free(p, 64);
    }
    let final_pages = a.memory_pages();
    assert_eq!(
        initial_pages, final_pages,
        "alloc/free churn must reuse the same block; pages went {initial_pages} -> {final_pages}"
    );
}

#[test]
fn memory_grows_when_allocation_exceeds_initial_page() {
    // 64 KiB initial memory minus the heap base. A 100 KiB
    // allocation must trigger memory.grow.
    let mut a = build_allocator();
    let initial_pages = a.memory_pages();
    let p = a.alloc(100_000);
    assert!(p > 0, "large alloc should succeed");
    let after_pages = a.memory_pages();
    assert!(
        after_pages > initial_pages,
        "memory should have grown past initial {initial_pages} pages; ended at {after_pages}"
    );
}
