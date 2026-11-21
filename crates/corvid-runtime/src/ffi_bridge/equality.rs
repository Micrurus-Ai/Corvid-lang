use corvid_abi::equality::EqualityLayout as Layout;
use std::collections::{HashMap, HashSet};

extern "C" {
    fn corvid_string_eq(a: u64, b: u64) -> i64;
    fn corvid_list_slots(payload: u64) -> *const u64;
}

// The compiler supplies a static layout matching both live values. Field
// slots are eight bytes apart, but a Bool field initializes only one byte.
unsafe fn read_slot(layout: *const u32, slot: *const u64) -> u64 {
    if unsafe { *layout } == Layout::Bool as u32 {
        unsafe { *slot.cast::<u8>() as u64 }
    } else {
        unsafe { *slot }
    }
}

/// # Safety
/// Both values must be live native values matching the compiler-emitted layout.
/// Every child offset in the layout must reference a valid comparison node.
#[no_mangle]
pub unsafe extern "C" fn corvid_structural_eq(layout: *const u32, left: u64, right: u64) -> u8 {
    let mut pending = vec![(layout, left, right)];
    let mut visited = HashSet::new();
    while let Some((node, a, b)) = pending.pop() {
        // SAFETY: all nodes originate in the compiler-emitted table. Payload
        // traversal uses only the selected variant and its declared fields.
        unsafe {
            let kind = *node;
            if kind == Layout::Float as u32 {
                if f64::from_bits(a) != f64::from_bits(b) {
                    return 0;
                }
                continue;
            }
            if kind == Layout::Int as u32
                || kind == Layout::Bool as u32
                || kind == Layout::Identity as u32
            {
                if a != b {
                    return 0;
                }
                continue;
            }
            if kind == Layout::String as u32 {
                if a != b && corvid_string_eq(a, b) == 0 {
                    return 0;
                }
                continue;
            }
            if !visited.insert((node, a, b)) {
                continue;
            }
            let child = |index: usize| layout.add(*node.add(index) as usize);
            let lhs = a as *const u64;
            let rhs = b as *const u64;
            let mut enqueue = |ty, x, y| pending.push((ty, read_slot(ty, x), read_slot(ty, y)));
            if kind == Layout::Option as u32 || kind == Layout::BoxedOption as u32 {
                if a == 0 || b == 0 {
                    if a != b {
                        return 0;
                    }
                } else if kind == Layout::BoxedOption as u32 {
                    enqueue(child(1), lhs, rhs);
                } else {
                    pending.push((child(1), a, b));
                }
            } else if kind == Layout::Result as u32 {
                if *lhs != *rhs {
                    return 0;
                }
                enqueue(child(1 + *lhs as usize), lhs.add(1), rhs.add(1));
            } else if kind == Layout::Record as u32 || kind == Layout::Sum as u32 {
                if a == b {
                    continue;
                }
                let (fields, start) = if kind == Layout::Sum as u32 {
                    if *lhs != *rhs {
                        return 0;
                    }
                    (child(2 + *lhs as usize), 1)
                } else {
                    (node, 0)
                };
                for i in 0..*fields.add(1) as usize {
                    let ty = layout.add(*fields.add(2 + i) as usize);
                    enqueue(ty, lhs.add(start + i), rhs.add(start + i));
                }
            } else {
                if *lhs != *rhs {
                    return 0;
                }
                let len = *lhs as usize;
                let xs = corvid_list_slots(a);
                let ys = corvid_list_slots(b);
                if kind == Layout::List as u32 {
                    for i in 0..len {
                        enqueue(child(1), xs.add(i), ys.add(i));
                    }
                } else if kind == Layout::StringMap as u32 {
                    if a == b {
                        continue;
                    }
                    let mut by_key = HashMap::with_capacity(len / 2);
                    for i in (0..len).step_by(2) {
                        let key = &*ys.add(i).cast::<crate::abi::CorvidString>();
                        by_key.insert(key.as_str(), ys.add(i + 1));
                    }
                    for i in (0..len).step_by(2) {
                        let key = &*xs.add(i).cast::<crate::abi::CorvidString>();
                        let Some(&value) = by_key.get(key.as_str()) else {
                            return 0;
                        };
                        enqueue(child(1), xs.add(i + 1), value);
                    }
                } else {
                    unreachable!("invalid comparison layout");
                }
            }
        }
    }
    1
}
