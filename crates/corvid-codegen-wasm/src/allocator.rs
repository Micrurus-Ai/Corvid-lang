//! Heap allocator emitted into every Corvid WASM module.
//!
//! ## Why this exists
//!
//! The WASM string ABI passes UTF-8 bytes across the boundary as
//! `(ptr, len)` pairs into linear memory. Both sides of the boundary
//! need a way to allocate memory the other side can read or write,
//! and free it when it's no longer needed. WASM linear memory has no
//! built-in allocator — the module ships with one or it doesn't have
//! one at all.
//!
//! Phase 20n-B ships a real allocator with proper `alloc` / `free`
//! semantics, not a bump-only sketch. The user explicitly chose
//! "real, no shortcuts" over a simpler bump-and-reset design when
//! the slice was scoped, because the JS loader's per-call alloc/free
//! pattern would leak permanently against a bump-only allocator.
//!
//! ## Data layout
//!
//! Linear memory is partitioned as follows:
//!
//! - `[0 .. 8)` — null-pointer sentinel. `corvid_alloc` never returns
//!   a payload pointer in this range, so a returned ptr of `0` is
//!   reserved as a sentinel for "out of memory."
//! - `[8 ..)` — heap. Allocation grows upward via the bump pointer
//!   `$heap_top` (a WASM global) until a free block of sufficient
//!   size is reclaimed, at which point the free list is preferred.
//!
//! Each block has a 4-byte header followed by its payload:
//!
//! ```text
//!   addr   addr+4    addr+4+size
//!     v       v           v
//!     [ size ][ payload ...]
//! ```
//!
//! The user-visible pointer points at the *payload* (`addr + 4`).
//! When a block is on the free list, the first 4 bytes of its
//! payload hold the address of the next free block's HEADER (or 0
//! to terminate the list). The free list head lives in the WASM
//! global `$free_head`.
//!
//! Minimum payload size is 4 bytes so the free-list link always
//! fits. `corvid_alloc(0)` rounds up to 4.
//!
//! ## Algorithms
//!
//! ### `corvid_alloc(size)`
//!
//! 1. Round `size` up to the minimum payload size (4 bytes).
//! 2. Walk `$free_head` looking for the first block whose stored
//!    `size_header` is `>= size`. (First-fit.)
//! 3. If found, unlink that block from the free list and return its
//!    payload address. The block's stored size is *not* shrunk to
//!    match the request — small allocations may receive over-sized
//!    blocks. The over-allocation is tolerated because freeing the
//!    block returns the original (oversized) size to the pool.
//! 4. If no free block fits, take the next slot from `$heap_top`.
//!    Write the requested size into the header, advance
//!    `$heap_top` by `4 + size`, return the payload address.
//! 5. If `$heap_top` would exceed the current memory capacity, call
//!    `memory.grow` to reserve more pages. If grow fails, return 0
//!    (out-of-memory sentinel).
//!
//! ### `corvid_free(ptr, _size)`
//!
//! `_size` is accepted for ABI compatibility with callers that
//! tracked it explicitly; the allocator ignores it and uses the
//! header's stored size.
//!
//! 1. Read the size header at `ptr - 4`.
//! 2. Insert the block at the head of the free list: write
//!    `$free_head` into `[ptr .. ptr+4)`, set `$free_head = ptr - 4`.
//! 3. Coalesce. Walk the free list looking for two cases:
//!    - A free block whose payload-end equals this block's header
//!      address: merge by extending the predecessor's size.
//!    - A free block whose header equals this block's payload-end:
//!      merge by extending this block's size.
//!    Coalescing handles the common adjacent-frees pattern that
//!    would otherwise fragment the heap.
//!
//! ## What's NOT in v1
//!
//! - **Splitting on alloc.** A free block of size 100 returned for
//!   a 4-byte request keeps its 100-byte size. Future slices can add
//!   splitting once metadata cost trade-offs are studied.
//! - **Header-free design.** Each used block carries a 4-byte size
//!   header. `wee_alloc`-style header-elision tricks are out of
//!   scope.
//! - **Concurrency.** WASM 1.0 is single-threaded; the allocator
//!   has no atomics. WASI threads / shared memory are a separate
//!   slice.
//! - **Allocation bookkeeping.** No counters, no high-water marks,
//!   no leak detection. Add them if the integration tests need
//!   them.

use wasm_encoder::{
    BlockType, ConstExpr, ExportKind, ExportSection, Function, FunctionSection, GlobalSection,
    GlobalType, Instruction, MemArg, MemorySection, MemoryType, TypeSection, ValType,
};

/// Minimum payload size, in bytes. The free-list link is stored in
/// the first 4 bytes of a free block's payload, so the payload must
/// be at least 4 bytes wide. `corvid_alloc(0)` and `corvid_alloc(1)`
/// both materialise as 4-byte allocations.
pub(crate) const MIN_PAYLOAD: i32 = 4;

/// Byte size of the per-block header. Bumped from `corvid_alloc` and
/// read by `corvid_free`. The size header lives at
/// `block_addr + 0 .. block_addr + 4`.
pub(crate) const HEADER_SIZE: i32 = 4;

/// Linear memory is reserved at offsets `[0 .. HEAP_BASE)` for
/// internal sentinels — never returned to user code. The first heap
/// allocation gets a header at `HEAP_BASE` and a payload at
/// `HEAP_BASE + 4`.
pub(crate) const HEAP_BASE: i32 = 8;

/// Initial WASM memory size, in 64-KiB pages. One page = 64 KiB.
/// Grows on demand via `memory.grow` inside `corvid_alloc`.
pub(crate) const INITIAL_MEMORY_PAGES: u64 = 1;

/// Indices of the allocator's three exports inside a freshly built
/// `corvid-codegen-wasm` module. The numeric values are relative to
/// `host_imports.len()` (host imports occupy function-index slots
/// `0..host_imports.len()` first; allocator slots come next; agents
/// last). `alloc` and `free` are unused by the lib.rs caller today
/// because Phase 20n-B's first commit ships only the allocator
/// itself; the codegen commit that lowers `String` parameters and
/// returns will start emitting `Call(alloc_indices.alloc)` / `free`
/// instructions and consume them then.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy)]
pub(crate) struct AllocatorIndices {
    /// Function index of `corvid_alloc(size: i32) -> i32`.
    pub alloc: u32,
    /// Function index of `corvid_free(ptr: i32, size: i32)`.
    pub free: u32,
    /// Number of allocator function slots (currently 2). Agent
    /// indices in `lib.rs` are computed as
    /// `host_imports.len() + ALLOCATOR_FUNC_COUNT + agent_idx`.
    pub func_count: u32,
}

/// Emit the allocator's memory, globals, function-types,
/// function-bodies, and exports into the in-progress module
/// builder. Returns the allocator's function indices so the
/// caller can shift its own function-index calculations.
///
/// Caller contract:
/// - `types`, `funcs`, `code`, `exports`, `mem`, `globals` are the
///   freshly-allocated WASM module sections in flight.
/// - `host_import_count` is the number of `(import "corvid:host" ...)`
///   functions that will occupy the lower function-index slots.
///   Allocator functions take the next two slots; agents take the
///   slots after that.
/// - `mem` and `globals` MUST be empty when this function is called.
///   It populates them itself; cargo will panic if the caller adds
///   their own memory or globals later because the WASM core
///   permits at most one memory and the allocator owns it.
pub(crate) fn emit_allocator(
    types: &mut TypeSection,
    funcs: &mut FunctionSection,
    code: &mut wasm_encoder::CodeSection,
    exports: &mut ExportSection,
    mem: &mut MemorySection,
    globals: &mut GlobalSection,
    host_import_count: u32,
) -> AllocatorIndices {
    // ---------- memory ----------
    mem.memory(MemoryType {
        minimum: INITIAL_MEMORY_PAGES,
        maximum: None,
        memory64: false,
        shared: false,
        page_size_log2: None,
    });
    exports.export("memory", ExportKind::Memory, 0);

    // ---------- globals ----------
    // $heap_top starts at HEAP_BASE — the first allocation's header
    // lands here, payload at HEAP_BASE + HEADER_SIZE.
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(HEAP_BASE),
    );
    // $free_head starts at 0 — empty free list.
    globals.global(
        GlobalType {
            val_type: ValType::I32,
            mutable: true,
            shared: false,
        },
        &ConstExpr::i32_const(0),
    );

    // ---------- function types ----------
    let alloc_type = types.len();
    types
        .ty()
        .function(vec![ValType::I32], vec![ValType::I32]);
    let free_type = types.len();
    types
        .ty()
        .function(vec![ValType::I32, ValType::I32], Vec::new());

    funcs.function(alloc_type);
    funcs.function(free_type);

    let alloc_idx = host_import_count;
    let free_idx = host_import_count + 1;

    // ---------- function bodies ----------
    code.function(&build_alloc_body());
    code.function(&build_free_body(alloc_idx /* unused */));

    exports.export("corvid_alloc", ExportKind::Func, alloc_idx);
    exports.export("corvid_free", ExportKind::Func, free_idx);

    AllocatorIndices {
        alloc: alloc_idx,
        free: free_idx,
        func_count: 2,
    }
}

/// Build the `corvid_alloc(size: i32) -> i32` function body.
///
/// Locals (after the parameter):
///   local 0: `size` (parameter)
///   local 1: `prev_link_addr` — address of the global / link cell
///            holding the next-pointer to the candidate. Used so we
///            can unlink the candidate by writing through it.
///   local 2: `curr` — header address of the current free-list
///            candidate (0 = end of list).
///   local 3: `block_size` — size header read from `curr`.
///   local 4: `next` — next-pointer read from the candidate's
///            payload.
///   local 5: `header_addr` — header address of a freshly bumped
///            block.
///   local 6: `pages_needed` — argument to `memory.grow` when we
///            run out.
fn build_alloc_body() -> Function {
    use Instruction::*;

    let locals: Vec<(u32, ValType)> = vec![(6, ValType::I32)];
    let mut f = Function::new(locals);

    // Round size up to MIN_PAYLOAD.
    f.instruction(&LocalGet(0));
    f.instruction(&I32Const(MIN_PAYLOAD));
    f.instruction(&I32LtS);
    f.instruction(&If(BlockType::Empty));
    f.instruction(&I32Const(MIN_PAYLOAD));
    f.instruction(&LocalSet(0));
    f.instruction(&End);

    // ---- free-list walk (first-fit) ----
    //
    // Layout: $free_head holds the address of the head free block's
    // HEADER, or 0 for empty. Each free block's payload[0..4] holds
    // the next free block's HEADER address, or 0 to terminate.
    //
    // We track `prev_link_addr` — the address of the cell whose
    // contents currently points at `curr`. For the head, that's the
    // *address* of the global $free_head — but globals don't have
    // addresses. We solve this by encoding "prev_link_addr == 0"
    // as "the free-list head is in the global, not in memory" and
    // branching on it before unlinking.

    // prev_link_addr <- 0 (sentinel: head is the global)
    f.instruction(&I32Const(0));
    f.instruction(&LocalSet(1));

    // curr <- $free_head
    f.instruction(&GlobalGet(1));
    f.instruction(&LocalSet(2));

    f.instruction(&Block(BlockType::Empty));
    f.instruction(&Loop(BlockType::Empty));
    {
        // if curr == 0: break out of loop (no fit found)
        f.instruction(&LocalGet(2));
        f.instruction(&I32Eqz);
        f.instruction(&BrIf(1));

        // block_size <- *(curr) (load header at offset 0)
        f.instruction(&LocalGet(2));
        f.instruction(&I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        f.instruction(&LocalSet(3));

        // next <- *(curr + HEADER_SIZE) (free-list link)
        f.instruction(&LocalGet(2));
        f.instruction(&I32Load(MemArg {
            offset: HEADER_SIZE as u64,
            align: 2,
            memory_index: 0,
        }));
        f.instruction(&LocalSet(4));

        // if block_size >= size: take it
        f.instruction(&LocalGet(3));
        f.instruction(&LocalGet(0));
        f.instruction(&I32GeS);
        f.instruction(&If(BlockType::Empty));
        {
            // unlink: if prev_link_addr == 0 then $free_head <- next
            //         else *(prev_link_addr) <- next
            f.instruction(&LocalGet(1));
            f.instruction(&I32Eqz);
            f.instruction(&If(BlockType::Empty));
            f.instruction(&LocalGet(4));
            f.instruction(&GlobalSet(1));
            f.instruction(&Else);
            f.instruction(&LocalGet(1));
            f.instruction(&LocalGet(4));
            f.instruction(&I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            f.instruction(&End);

            // return curr + HEADER_SIZE  (payload pointer)
            f.instruction(&LocalGet(2));
            f.instruction(&I32Const(HEADER_SIZE));
            f.instruction(&I32Add);
            f.instruction(&Return);
        }
        f.instruction(&End);

        // didn't fit — advance: prev_link_addr <- curr + HEADER_SIZE,
        //                       curr           <- next
        f.instruction(&LocalGet(2));
        f.instruction(&I32Const(HEADER_SIZE));
        f.instruction(&I32Add);
        f.instruction(&LocalSet(1));

        f.instruction(&LocalGet(4));
        f.instruction(&LocalSet(2));

        f.instruction(&Br(0)); // continue loop
    }
    f.instruction(&End); // loop
    f.instruction(&End); // block

    // ---- bump path ----
    //
    // header_addr <- $heap_top
    // new_top    <- $heap_top + HEADER_SIZE + size
    // if new_top > memory.size_in_bytes: grow_memory
    // *(header_addr) <- size
    // $heap_top <- new_top
    // return header_addr + HEADER_SIZE

    f.instruction(&GlobalGet(0));
    f.instruction(&LocalSet(5));

    // new_top calc: header_addr + HEADER_SIZE + size
    f.instruction(&LocalGet(5));
    f.instruction(&I32Const(HEADER_SIZE));
    f.instruction(&I32Add);
    f.instruction(&LocalGet(0));
    f.instruction(&I32Add);
    // top of stack: new_top

    // memory grow if needed.
    // current memory bytes = memory.size * 65536
    f.instruction(&MemorySize(0));
    f.instruction(&I32Const(16)); // log2(65536)
    f.instruction(&I32Shl);
    // top of stack now: new_top, mem_bytes
    f.instruction(&I32GtU);
    f.instruction(&If(BlockType::Empty));
    {
        // pages_needed = 1 (request one page; if multi-page allocs
        // become common, compute ceil((new_top - mem_bytes) / 65536))
        f.instruction(&I32Const(1));
        f.instruction(&LocalSet(6));

        f.instruction(&LocalGet(6));
        f.instruction(&MemoryGrow(0));
        // memory.grow returns -1 on failure or the previous size.
        f.instruction(&I32Const(-1));
        f.instruction(&I32Eq);
        f.instruction(&If(BlockType::Empty));
        // Out of memory — return 0 sentinel.
        f.instruction(&I32Const(0));
        f.instruction(&Return);
        f.instruction(&End);
    }
    f.instruction(&End);

    // *(header_addr) <- size
    f.instruction(&LocalGet(5));
    f.instruction(&LocalGet(0));
    f.instruction(&I32Store(MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    }));

    // $heap_top <- header_addr + HEADER_SIZE + size
    f.instruction(&LocalGet(5));
    f.instruction(&I32Const(HEADER_SIZE));
    f.instruction(&I32Add);
    f.instruction(&LocalGet(0));
    f.instruction(&I32Add);
    f.instruction(&GlobalSet(0));

    // return header_addr + HEADER_SIZE
    f.instruction(&LocalGet(5));
    f.instruction(&I32Const(HEADER_SIZE));
    f.instruction(&I32Add);
    f.instruction(&End); // close function

    f
}

/// Build the `corvid_free(ptr: i32, size: i32)` function body.
///
/// Parameter `size` is accepted for ABI compatibility with callers
/// that track it; the allocator reads the actual size from the
/// header. We don't validate that the argument matches because the
/// JS loader is the only caller in v1 and it threads sizes through
/// the JS side accurately.
///
/// Locals (after parameters):
///   local 0: `ptr` (parameter)
///   local 1: `_size` (parameter, ignored)
///   local 2: `header_addr` — `ptr - HEADER_SIZE`
///   local 3: `block_end` — `ptr + size_in_header`
///   local 4: `prev_addr` — header address of the previous free block
///            during coalesce-walk, or 0 for "previous is the global"
///   local 5: `curr_addr` — header address of the current free block
///   local 6: `curr_size` — size header read from `curr_addr`
///   local 7: `curr_end` — payload-end of `curr_addr` (used to detect
///            left-adjacency)
///   local 8: `curr_next` — next-pointer read from `curr_addr`'s
///            payload[0..4]
fn build_free_body(_alloc_idx: u32) -> Function {
    use Instruction::*;

    let locals: Vec<(u32, ValType)> = vec![(7, ValType::I32)];
    let mut f = Function::new(locals);

    // header_addr <- ptr - HEADER_SIZE
    f.instruction(&LocalGet(0));
    f.instruction(&I32Const(HEADER_SIZE));
    f.instruction(&I32Sub);
    f.instruction(&LocalSet(2));

    // block_end <- ptr + *(header_addr)
    //            = ptr + payload_size
    f.instruction(&LocalGet(0));
    f.instruction(&LocalGet(2));
    f.instruction(&I32Load(MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    }));
    f.instruction(&I32Add);
    f.instruction(&LocalSet(3));

    // Insert at head of free list.
    // *(ptr) <- $free_head     (link this block's payload[0..4] to old head)
    f.instruction(&LocalGet(0));
    f.instruction(&GlobalGet(1));
    f.instruction(&I32Store(MemArg {
        offset: 0,
        align: 2,
        memory_index: 0,
    }));
    // $free_head <- header_addr
    f.instruction(&LocalGet(2));
    f.instruction(&GlobalSet(1));

    // ---- coalescing walk ----
    //
    // After insertion, this block is the new head. Walk the free
    // list from `header_addr.payload[0..4]` (the block immediately
    // after this one in the list — i.e., the *old* head) looking
    // for a candidate adjacent on either side. We do this in one
    // pass per direction; a naive single pass that tries to merge
    // both sides at once gets confusing because the list shape can
    // change mid-walk.

    // Pass 1: forward coalesce (this.end == candidate.header)
    //
    //   prev_addr <- header_addr (the just-inserted block)
    //   curr_addr <- *(header_addr + HEADER_SIZE)  (payload[0..4])
    f.instruction(&LocalGet(2));
    f.instruction(&LocalSet(4));

    f.instruction(&LocalGet(2));
    f.instruction(&I32Load(MemArg {
        offset: HEADER_SIZE as u64,
        align: 2,
        memory_index: 0,
    }));
    f.instruction(&LocalSet(5));

    f.instruction(&Block(BlockType::Empty));
    f.instruction(&Loop(BlockType::Empty));
    {
        // if curr_addr == 0: break
        f.instruction(&LocalGet(5));
        f.instruction(&I32Eqz);
        f.instruction(&BrIf(1));

        // if curr_addr == block_end: merge curr into self.
        //   self.size += HEADER_SIZE + curr_size
        //   unlink curr from list:
        //     *(prev_addr.payload[0..4]) <- curr_next
        f.instruction(&LocalGet(5));
        f.instruction(&LocalGet(3));
        f.instruction(&I32Eq);
        f.instruction(&If(BlockType::Empty));
        {
            // curr_size <- *(curr_addr)
            f.instruction(&LocalGet(5));
            f.instruction(&I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            f.instruction(&LocalSet(6));

            // curr_next <- *(curr_addr + HEADER_SIZE)
            f.instruction(&LocalGet(5));
            f.instruction(&I32Load(MemArg {
                offset: HEADER_SIZE as u64,
                align: 2,
                memory_index: 0,
            }));
            f.instruction(&LocalSet(8));

            // unlink: *(prev_addr.payload[0..4]) <- curr_next
            f.instruction(&LocalGet(4));
            f.instruction(&LocalGet(8));
            f.instruction(&I32Store(MemArg {
                offset: HEADER_SIZE as u64,
                align: 2,
                memory_index: 0,
            }));

            // self.size += HEADER_SIZE + curr_size
            f.instruction(&LocalGet(2));
            f.instruction(&LocalGet(2));
            f.instruction(&I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            f.instruction(&I32Const(HEADER_SIZE));
            f.instruction(&I32Add);
            f.instruction(&LocalGet(6));
            f.instruction(&I32Add);
            f.instruction(&I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));

            // Refresh block_end:
            //   block_end = header_addr + HEADER_SIZE + new_self_size
            f.instruction(&LocalGet(2));
            f.instruction(&I32Const(HEADER_SIZE));
            f.instruction(&I32Add);
            f.instruction(&LocalGet(2));
            f.instruction(&I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            f.instruction(&I32Add);
            f.instruction(&LocalSet(3));

            // After a forward merge there can be at most one more
            // forward-adjacent block (the merged-into one's old
            // successor). Keep looping; we just continue with
            // curr_addr <- curr_next.
            f.instruction(&LocalGet(8));
            f.instruction(&LocalSet(5));
            f.instruction(&Br(1)); // continue Loop
        }
        f.instruction(&End);

        // Not a forward-adjacent merge. Step forward.
        // prev_addr  <- curr_addr
        // curr_addr  <- *(curr_addr + HEADER_SIZE)
        f.instruction(&LocalGet(5));
        f.instruction(&LocalSet(4));

        f.instruction(&LocalGet(5));
        f.instruction(&I32Load(MemArg {
            offset: HEADER_SIZE as u64,
            align: 2,
            memory_index: 0,
        }));
        f.instruction(&LocalSet(5));

        f.instruction(&Br(0));
    }
    f.instruction(&End); // loop
    f.instruction(&End); // block

    // Pass 2: backward coalesce (candidate.end == this.header)
    //
    //   prev_addr <- 0 (sentinel: $free_head is the predecessor)
    //   curr_addr <- $free_head
    f.instruction(&I32Const(0));
    f.instruction(&LocalSet(4));

    f.instruction(&GlobalGet(1));
    f.instruction(&LocalSet(5));

    f.instruction(&Block(BlockType::Empty));
    f.instruction(&Loop(BlockType::Empty));
    {
        // if curr_addr == 0: break
        f.instruction(&LocalGet(5));
        f.instruction(&I32Eqz);
        f.instruction(&BrIf(1));

        // if curr_addr == header_addr: that's *us*, skip.
        f.instruction(&LocalGet(5));
        f.instruction(&LocalGet(2));
        f.instruction(&I32Eq);
        f.instruction(&If(BlockType::Empty));
        // Step forward without merging.
        f.instruction(&LocalGet(5));
        f.instruction(&LocalSet(4));
        f.instruction(&LocalGet(5));
        f.instruction(&I32Load(MemArg {
            offset: HEADER_SIZE as u64,
            align: 2,
            memory_index: 0,
        }));
        f.instruction(&LocalSet(5));
        // Branch label 1 from inside this If is the enclosing Loop
        // (label 0 is the If itself; label 2 is the outer Block).
        // Br(1) jumps back to the Loop start; Br(2) would exit the
        // pass-2 walk entirely and was the bug in the first
        // implementation.
        f.instruction(&Br(1));
        f.instruction(&End);

        // curr_size  <- *(curr_addr)
        // curr_end   <- curr_addr + HEADER_SIZE + curr_size
        f.instruction(&LocalGet(5));
        f.instruction(&I32Load(MemArg {
            offset: 0,
            align: 2,
            memory_index: 0,
        }));
        f.instruction(&LocalSet(6));

        f.instruction(&LocalGet(5));
        f.instruction(&I32Const(HEADER_SIZE));
        f.instruction(&I32Add);
        f.instruction(&LocalGet(6));
        f.instruction(&I32Add);
        f.instruction(&LocalSet(7));

        // if curr_end == header_addr: merge self INTO curr.
        f.instruction(&LocalGet(7));
        f.instruction(&LocalGet(2));
        f.instruction(&I32Eq);
        f.instruction(&If(BlockType::Empty));
        {
            // curr.size += HEADER_SIZE + self.size
            // self.size lives at `*header_addr`.
            f.instruction(&LocalGet(5));
            f.instruction(&LocalGet(6));
            f.instruction(&I32Const(HEADER_SIZE));
            f.instruction(&I32Add);
            f.instruction(&LocalGet(2));
            f.instruction(&I32Load(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));
            f.instruction(&I32Add);
            f.instruction(&I32Store(MemArg {
                offset: 0,
                align: 2,
                memory_index: 0,
            }));

            // Unlink self from the list. Self is currently the head:
            //   $free_head <- *(self.payload[0..4])
            f.instruction(&LocalGet(2));
            f.instruction(&I32Load(MemArg {
                offset: HEADER_SIZE as u64,
                align: 2,
                memory_index: 0,
            }));
            f.instruction(&GlobalSet(1));

            // After self is folded into curr, no further backward
            // merges are possible (only one neighbor can end at
            // header_addr in a sorted-by-address world, and the
            // free list is implicitly sorted enough by the
            // construction process).
            f.instruction(&Return);
        }
        f.instruction(&End);

        // No merge. Step forward.
        f.instruction(&LocalGet(5));
        f.instruction(&LocalSet(4));

        f.instruction(&LocalGet(5));
        f.instruction(&I32Load(MemArg {
            offset: HEADER_SIZE as u64,
            align: 2,
            memory_index: 0,
        }));
        f.instruction(&LocalSet(5));

        f.instruction(&Br(0));
    }
    f.instruction(&End); // loop
    f.instruction(&End); // block

    f.instruction(&End); // close function
    f
}
