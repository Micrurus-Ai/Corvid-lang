//! Bacon-Rajan trial deletion for the interpreter-owned heap graph.
//!
//! This collector is VM-only. The native tier uses the C runtime's
//! mark-sweep collector over its own heap. Parity is behavioural, not
//! implementation-sharing.

use crate::value::{Color, ObjectRef, WeakObjectRef};
use parking_lot::{Mutex, RwLock, RwLockReadGuard};
use std::cell::Cell;
use std::collections::HashSet;

// Acquire before touching a cell payload or its ownership count. Multiple
// application threads can use the heap together. Trial deletion requires a
// stable graph and defers if any application thread is using it.
static HEAP: RwLock<()> = RwLock::new(());
static ROOTS: Mutex<Vec<WeakObjectRef>> = Mutex::new(Vec::new());
thread_local! {
    static IN_COLLECTION: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn heap_access() -> Option<RwLockReadGuard<'static, ()>> {
    (!IN_COLLECTION.get()).then(|| HEAP.read_recursive())
}

struct CollectionScope;

impl Drop for CollectionScope {
    fn drop(&mut self) {
        IN_COLLECTION.set(false);
    }
}

fn auto_trigger() -> usize {
    std::env::var("CORVID_VM_GC_TRIGGER")
        .ok()
        .and_then(|raw| raw.parse::<usize>().ok())
        .unwrap_or(10_000)
}

pub(crate) fn release_object(object: ObjectRef) {
    let access = heap_access();
    // All condemned cells were retired before their edges were removed.
    // Edges from a condemned cell to a live cell still need a normal release.
    if IN_COLLECTION.get() && object.strong_count() == 0 {
        return;
    }

    let old = object.release_strong();
    if old == 1 {
        object.free_zero_path();
        return;
    }

    let should_collect = {
        let mut roots = ROOTS.lock();
        if !object.buffered() {
            object.set_buffered(true);
            object.set_color(Color::Purple);
            roots.push(object.downgrade());
        }
        let trigger = auto_trigger();
        trigger > 0 && roots.len() >= trigger
    };

    drop(access);
    if should_collect {
        let _ = collect_cycles();
    }
}

/// Reclaim unreachable cycles while the heap is idle. If another heap
/// operation is active, leave the candidate roots queued for a later pass.
pub fn collect_cycles() -> usize {
    let Some(_exclusive) = HEAP.try_write() else {
        return 0;
    };
    IN_COLLECTION.set(true);
    let _scope = CollectionScope;
    let roots = std::mem::take(&mut *ROOTS.lock());

    for root in &roots {
        if let Some(object) = root.upgrade() {
            if object.buffered() && object.strong_count() > 0 {
                mark_gray(&object);
            }
        }
    }

    for root in &roots {
        if let Some(object) = root.upgrade() {
            scan(&object);
        }
    }

    let mut condemned = Vec::new();
    let mut seen = HashSet::new();
    for root in &roots {
        if let Some(object) = root.upgrade() {
            collect_white(&object, &mut seen, &mut condemned);
        }
    }
    let collected = condemned.len();

    if !condemned.is_empty() {
        for object in &condemned {
            object.prepare_collect();
        }
        for object in &condemned {
            object.clear_payload();
        }
    }

    collected
}

enum MarkTask {
    Visit(ObjectRef),
    DecShadow(ObjectRef),
}

fn mark_gray(object: &ObjectRef) {
    let mut stack = vec![MarkTask::Visit(object.clone())];
    while let Some(task) = stack.pop() {
        match task {
            MarkTask::Visit(object) => {
                if object.color() == Color::Gray {
                    continue;
                }
                object.set_color(Color::Gray);
                object.set_shadow(object.strong_count());
                let children = object.children();
                for child in children.into_iter().rev() {
                    stack.push(MarkTask::DecShadow(child.clone()));
                    stack.push(MarkTask::Visit(child));
                }
            }
            MarkTask::DecShadow(child) => child.dec_shadow(),
        }
    }
}

fn scan(object: &ObjectRef) {
    let mut stack = vec![ScanTask::Scan(object.clone())];
    while let Some(task) = stack.pop() {
        match task {
            ScanTask::Scan(object) => {
                if object.color() != Color::Gray {
                    continue;
                }

                if object.shadow_count() > 0 {
                    stack.push(ScanTask::ScanBlack(object));
                } else {
                    object.set_color(Color::White);
                    for child in object.children().into_iter().rev() {
                        stack.push(ScanTask::Scan(child));
                    }
                }
            }
            ScanTask::ScanBlack(object) => {
                if object.color() == Color::Black {
                    object.set_buffered(false);
                    continue;
                }

                object.set_color(Color::Black);
                object.set_buffered(false);
                let children = object.children();
                for child in children.into_iter().rev() {
                    child.inc_shadow();
                    stack.push(ScanTask::ScanBlack(child));
                }
            }
        }
    }
}

enum ScanTask {
    Scan(ObjectRef),
    ScanBlack(ObjectRef),
}

fn collect_white(
    object: &ObjectRef,
    seen: &mut HashSet<usize>,
    out: &mut Vec<ObjectRef>,
) {
    let mut stack = vec![object.clone()];
    while let Some(object) = stack.pop() {
        if !seen.insert(object.ptr_key()) {
            continue;
        }

        if object.color() == Color::White {
            out.push(object.clone());
            for child in object.children().into_iter().rev() {
                stack.push(child);
            }
        } else {
            object.set_color(Color::Black);
            object.set_buffered(false);
        }
    }
}
