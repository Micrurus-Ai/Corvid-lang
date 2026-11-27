//! Phase 17d — cycle collector end-to-end test.
//!
//! Current Corvid syntax has no field mutation, so users can't
//! construct refcounted cycles from Corvid source today. To exercise
//! the collector we build the cycle directly via FFI and call
//! `corvid_gc_from_roots` (the deterministic, no-stack-walk variant).
//!
//! `corvid_gc`, the collection the allocation threshold triggers, finds
//! its roots by trial deletion: the references a block holds to another
//! are subtracted from that block's count, and whatever remains is held
//! from outside the heap. It does not walk the stack, so it is tested
//! here from Rust too: a cycle is collected, and a block only a Rust local
//! holds (as a runtime function holds one mid-call) survives. It used to
//! mark from compiled frames' stack maps alone, which freed exactly such
//! blocks, including the one an allocation was about to return.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, MutexGuard, OnceLock};

#[repr(C)]
struct CorvidTypeinfo {
    size: u32,
    flags: u32,
    destroy_fn: Option<unsafe extern "C" fn(*mut u8)>,
    trace_fn: Option<
        unsafe extern "C" fn(
            *mut u8,
            Option<unsafe extern "C" fn(*mut u8, *mut u8)>,
            *mut u8,
        ),
    >,
    weak_fn: Option<unsafe extern "C" fn(*mut u8)>,
    elem_typeinfo: *const CorvidTypeinfo,
    name: *const u8,
}

unsafe impl Sync for CorvidTypeinfo {}

#[link(name = "corvid_c_runtime", kind = "static")]
extern "C" {
    fn corvid_alloc_typed(payload_bytes: i64, typeinfo: *const CorvidTypeinfo) -> *mut u8;
    fn corvid_retain(payload: *mut u8);
    fn corvid_release(payload: *mut u8);
    fn corvid_gc_from_roots(roots: *mut *mut u8, n_roots: usize);
    fn corvid_gc();
    fn corvid_pool_cached_blocks_for_size(payload_bytes: i64) -> i64;

    static corvid_alloc_count: i64;
    static corvid_release_count: i64;
}

fn test_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .expect("test lock poisoned")
}

/// Trace fn for our synthetic 2-pointer "Cell" type: fields at
/// offsets 0 and 8. Called by the collector's mark + sweep-
/// decrement passes.
unsafe extern "C" fn cell_trace(
    payload: *mut u8,
    marker: Option<unsafe extern "C" fn(*mut u8, *mut u8)>,
    ctx: *mut u8,
) {
    let slots = payload as *mut *mut u8;
    for i in 0..2 {
        let ptr = *slots.add(i);
        if !ptr.is_null() {
            if let Some(m) = marker {
                m(ptr, ctx);
            }
        }
    }
}

static CELL_DESTROY_COUNT: AtomicUsize = AtomicUsize::new(0);

unsafe extern "C" fn cell_destroy(payload: *mut u8) {
    CELL_DESTROY_COUNT.fetch_add(1, Ordering::SeqCst);
    let slots = payload as *mut *mut u8;
    for i in 0..2 {
        let ptr = *slots.add(i);
        if !ptr.is_null() {
            corvid_release(ptr);
        }
    }
}

static CELL_TYPEINFO: CorvidTypeinfo = CorvidTypeinfo {
    size: 16,
    flags: 0x01, // CYCLIC_CAPABLE
    destroy_fn: Some(cell_destroy),
    trace_fn: Some(cell_trace),
    weak_fn: None,
    elem_typeinfo: std::ptr::null(),
    name: std::ptr::null(),
};

static CYCLE24_TYPEINFO: CorvidTypeinfo = CorvidTypeinfo {
    size: 24,
    flags: 0x01,
    destroy_fn: Some(cell_destroy),
    trace_fn: Some(cell_trace),
    weak_fn: None,
    elem_typeinfo: std::ptr::null(),
    name: c"Cycle24".as_ptr() as *const u8,
};

/// A size no other test allocates, so its pool bucket is this test's own.
static REUSED40_TYPEINFO: CorvidTypeinfo = CorvidTypeinfo {
    size: 40,
    flags: 0x01,
    destroy_fn: Some(cell_destroy),
    trace_fn: Some(cell_trace),
    weak_fn: None,
    elem_typeinfo: std::ptr::null(),
    name: c"Reused40".as_ptr() as *const u8,
};

/// Unreachable 2-block cycle with NO roots → collector frees both.
#[test]
fn cycle_with_no_roots_is_collected() {
    let _guard = test_lock();
    let start_allocs: i64;
    let start_releases: i64;
    unsafe {
        start_allocs = corvid_alloc_count;
        start_releases = corvid_release_count;
    }

    unsafe {
        let a = corvid_alloc_typed(16, &CELL_TYPEINFO);
        let b = corvid_alloc_typed(16, &CELL_TYPEINFO);
        // field0 points at the other block, field1 is NULL.
        (a as *mut *mut u8).write(b);
        (a as *mut *mut u8).add(1).write(std::ptr::null_mut());
        (b as *mut *mut u8).write(a);
        (b as *mut *mut u8).add(1).write(std::ptr::null_mut());
        // a and b go out of scope here — our refs drop, but each
        // is still held by the other's field. Refcount alone
        // can't reclaim this.
    }

    // Verify refcount alone didn't free the cycle.
    unsafe {
        assert_eq!(corvid_alloc_count - start_allocs, 2);
        assert_eq!(
            corvid_release_count - start_releases,
            0,
            "cycle leaks under refcount alone; that's why 17d exists"
        );
    }

    // Collect with NO roots — both blocks must be freed.
    unsafe {
        let empty: &mut [*mut u8] = &mut [];
        corvid_gc_from_roots(empty.as_mut_ptr(), 0);
        assert_eq!(
            corvid_release_count - start_releases,
            2,
            "2-block cycle reclaimed by sweep"
        );
    }
}

/// Cycle with ONE root held externally → mark phase traces from
/// the root through the cycle, sweep finds nothing unmarked, both
/// blocks survive.
#[test]
fn cycle_with_external_root_survives() {
    let _guard = test_lock();
    let start_allocs: i64;
    let start_releases: i64;
    unsafe {
        start_allocs = corvid_alloc_count;
        start_releases = corvid_release_count;
    }

    let root_a: *mut u8;
    unsafe {
        let a = corvid_alloc_typed(16, &CELL_TYPEINFO);
        let b = corvid_alloc_typed(16, &CELL_TYPEINFO);
        (a as *mut *mut u8).write(b);
        (a as *mut *mut u8).add(1).write(std::ptr::null_mut());
        (b as *mut *mut u8).write(a);
        (b as *mut *mut u8).add(1).write(std::ptr::null_mut());
        corvid_retain(a); // external ref; a's refcount = 2
        root_a = a;
    }

    // Collect with `a` as the root — both blocks must survive
    // (b is reachable via a->field0).
    unsafe {
        let mut roots: [*mut u8; 1] = [root_a];
        corvid_gc_from_roots(roots.as_mut_ptr(), 1);
        assert_eq!(
            corvid_release_count - start_releases,
            0,
            "cycle reachable from root — nothing freed"
        );
    }

    // Now drop our external ref. Refcount fast path fires
    // destroy_fn on `a`, which releases `b`, freeing it. Then `a`
    // is freed. Two deterministic frees via refcount alone — the
    // cycle's self-reference in `b` drops to 0 after a's destroy
    // calls release on b.
    //
    // Wait — not quite. After we release a, a's refcount drops
    // from 2 to 1 (b still holds a via b->field0). No free. The
    // cycle is now unreachable from any root but held alive by
    // itself. Another `corvid_gc_from_roots` call with empty
    // roots reclaims it.
    unsafe {
        corvid_release(root_a);
        let empty: &mut [*mut u8] = &mut [];
        corvid_gc_from_roots(empty.as_mut_ptr(), 0);
        assert_eq!(
            corvid_release_count - start_releases,
            2,
            "after releasing root + GC, cycle collected"
        );
    }

    let _ = start_allocs;
}

/// Non-cycle with destroy_fn — refcount fast path still works
/// alongside the collector. Sanity check that adding cycle-
/// collection infrastructure didn't regress the simple case.
#[test]
fn acyclic_refcount_path_still_works() {
    let _guard = test_lock();
    let start_releases: i64;
    unsafe {
        start_releases = corvid_release_count;
    }

    unsafe {
        let p = corvid_alloc_typed(16, &CELL_TYPEINFO);
        // `corvid_alloc_typed` returns the fields empty, so
        // `cell_destroy` finds nothing to release.
        corvid_release(p); // refcount 1 → 0, destroy_fn fires, free.
    }

    unsafe {
        assert_eq!(
            corvid_release_count - start_releases,
            1,
            "acyclic block freed immediately by refcount — no GC needed"
        );
    }
}

#[test]
fn gc_sweep_returns_cycle_blocks_to_pool() {
    let _guard = test_lock();
    unsafe {
        let before = corvid_pool_cached_blocks_for_size(24);

        let a = corvid_alloc_typed(24, &CYCLE24_TYPEINFO);
        let b = corvid_alloc_typed(24, &CYCLE24_TYPEINFO);
        (a as *mut *mut u8).write(b);
        (a as *mut *mut u8).add(1).write(std::ptr::null_mut());
        (b as *mut *mut u8).write(a);
        (b as *mut *mut u8).add(1).write(std::ptr::null_mut());

        let empty: &mut [*mut u8] = &mut [];
        corvid_gc_from_roots(empty.as_mut_ptr(), 0);

        let after = corvid_pool_cached_blocks_for_size(24);
        assert_eq!(
            after - before,
            2,
            "GC sweep should recycle freed fixed-size cycle blocks into the pool"
        );
    }
}

/// The collection the allocation threshold runs collects an unreachable
/// cycle, and leaves alone a block that only a local holds, which is how a
/// runtime function holds what it is building, and how an allocation holds
/// the block it is about to return.
#[test]
fn the_threshold_collection_frees_cycles_and_keeps_what_a_local_holds() {
    let _guard = test_lock();
    unsafe {
        let start_releases = corvid_release_count;
        let held = corvid_alloc_typed(16, &CELL_TYPEINFO);
        (held as *mut *mut u8).write(std::ptr::null_mut());
        (held as *mut *mut u8).add(1).write(std::ptr::null_mut());
        let a = corvid_alloc_typed(16, &CELL_TYPEINFO);
        let b = corvid_alloc_typed(16, &CELL_TYPEINFO);
        (a as *mut *mut u8).write(b);
        (a as *mut *mut u8).add(1).write(std::ptr::null_mut());
        (b as *mut *mut u8).write(a);
        (b as *mut *mut u8).add(1).write(std::ptr::null_mut());

        corvid_gc();
        assert_eq!(corvid_release_count - start_releases, 2, "the cycle is freed and the held block is not");

        // A cycle with a reference from outside the heap survives.
        let c = corvid_alloc_typed(16, &CELL_TYPEINFO);
        let d = corvid_alloc_typed(16, &CELL_TYPEINFO);
        (c as *mut *mut u8).write(d);
        (c as *mut *mut u8).add(1).write(std::ptr::null_mut());
        (d as *mut *mut u8).write(c);
        (d as *mut *mut u8).add(1).write(std::ptr::null_mut());
        corvid_retain(c);
        corvid_gc();
        assert_eq!(corvid_release_count - start_releases, 2, "an externally held cycle survives");

        corvid_release(held);
        corvid_release(c);
        corvid_gc();
        assert_eq!(corvid_release_count - start_releases, 5, "released, the held block and the cycle go");
    }
}

/// A block comes back from the allocator with its fields empty, even
/// when it is a pooled block that held references before. The collector
/// traces every live block, and a collection can run before the caller
/// fills the fields (inside the allocation, or while the program
/// allocates what goes in them), so a field not yet written must read
/// as no value. When it read the old contents instead, the collector
/// subtracted from whatever those stale pointers named, which changed
/// one byte of an unrelated string ("input" became "inout").
#[test]
fn a_reused_block_comes_back_with_its_fields_empty() {
    let _guard = test_lock();
    unsafe {
        let target = corvid_alloc_typed(40, &REUSED40_TYPEINFO);
        let first = corvid_alloc_typed(40, &REUSED40_TYPEINFO);
        corvid_retain(target);
        (first as *mut *mut u8).write(target);
        (first as *mut *mut u8).add(1).write(target);
        corvid_retain(target);
        corvid_release(first); // releases both references to `target`; `first` goes to the pool

        let second = corvid_alloc_typed(40, &REUSED40_TYPEINFO);
        assert_eq!(second, first, "the pool handed back the same block");
        let slots = second as *mut *mut u8;
        for i in 0..5 {
            assert!(slots.add(i).read().is_null(), "field {i} of a reused block is empty");
        }

        corvid_gc();
        assert!(!target.is_null());
        corvid_release(second);
        corvid_release(target);
    }
}

