//! Grounded-value attestations exposed at the FFI boundary.
//!
//! The attestation store is the owner of grounded handles: it tracks
//! provenance chains, confidence, and leak accounting behind a single
//! process-global slotmap. The C-ABI wrappers only translate integer
//! handles to these operations; they do not own any lifetime logic.

use crate::attestation_store::AttestationStore;
use crate::provenance::ProvenanceChain;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

pub const NULL_GROUNDED_HANDLE: u64 = 0;

#[derive(Debug)]
pub struct GroundedAttestation {
    pub provenance: Arc<ProvenanceChain>,
    pub confidence: f64,
}

impl GroundedAttestation {
    pub fn source_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        for entry in &self.provenance.entries {
            if !matches!(entry.kind, crate::provenance::ProvenanceKind::Retrieval) {
                continue;
            }
            if !out.iter().any(|name| name == &entry.name) {
                out.push(entry.name.clone());
            }
        }
        out
    }
}

impl Default for GroundedHandleStore {
    fn default() -> Self {
        Self {
            handles: AttestationStore::new("grounded handle store"),
            pointer_attestations: HashMap::new(),
        }
    }
}

/// Attach a `GroundedAttestation` keyed by a heap-object pointer.
///
/// The map is heap-pointer-keyed and works for any refcounted heap
/// object the language can attest: `String` descriptors, `Struct`
/// allocations, future `List` allocations. The earlier name
/// (`attach_string_attestation`) reflected the first caller, not
/// the storage's actual semantics. The pointer is whatever stable
/// identifier the calling C-ABI bridge derives from the value
/// (`CorvidString::descriptor_key()` for strings, the raw heap
/// pointer for structs).
pub fn attach_pointer_attestation(ptr: usize, attestation: Arc<GroundedAttestation>) {
    if ptr == 0 {
        return;
    }
    let mut store = store().lock().unwrap();
    store.pointer_attestations.insert(ptr, attestation);
}

pub fn set_last_scalar_attestation(attestation: Arc<GroundedAttestation>) {
    LAST_SCALAR_ATTESTATION.with(|slot| {
        *slot.borrow_mut() = Some(attestation);
    });
}

/// Move a previously-attached pointer-keyed attestation into the
/// numeric-handle store and return its handle. Removes the entry
/// from the pointer-keyed map; subsequent lookups return the null
/// handle. Works for any heap-pointer key — `String` descriptors,
/// `Struct` allocations, and future heap shapes.
pub fn register_handle_for_pointer(ptr: usize) -> u64 {
    if ptr == 0 {
        return NULL_GROUNDED_HANDLE;
    }
    let mut store = store().lock().unwrap();
    let Some(attestation) = store.pointer_attestations.remove(&ptr) else {
        return NULL_GROUNDED_HANDLE;
    };
    store.handles.insert(attestation)
}

pub fn register_handle_for_last_scalar() -> u64 {
    LAST_SCALAR_ATTESTATION.with(|slot| {
        let Some(attestation) = slot.borrow_mut().take() else {
            return NULL_GROUNDED_HANDLE;
        };
        let mut store = store().lock().unwrap();
        store.handles.insert(attestation)
    })
}

pub fn sources_for_handle(handle: u64) -> Option<Vec<String>> {
    if handle == NULL_GROUNDED_HANDLE {
        return None;
    }
    let store = store().lock().unwrap();
    store
        .handles
        .get(handle)
        .map(|attestation| attestation.source_names())
}

pub fn confidence_for_handle(handle: u64) -> Option<f64> {
    if handle == NULL_GROUNDED_HANDLE {
        return None;
    }
    let store = store().lock().unwrap();
    store.handles.get(handle).map(|attestation| attestation.confidence)
}

pub fn release_handle(handle: u64) -> bool {
    if handle == NULL_GROUNDED_HANDLE {
        return true;
    }
    let mut store = store().lock().unwrap();
    store.handles.remove(handle)
}

pub fn emit_debug_leak_warning() {
    let store = store().lock().unwrap();
    store.handles.emit_debug_leak_warning();
}
struct GroundedHandleStore {
    handles: AttestationStore<GroundedAttestation>,
    /// Heap-pointer-keyed attestations awaiting promotion to a
    /// numeric handle via `register_handle_for_pointer`. Works for
    /// any refcounted heap object: `String` descriptors, `Struct`
    /// allocations, future heap shapes.
    pointer_attestations: HashMap<usize, Arc<GroundedAttestation>>,
}

static STORE: OnceLock<Mutex<GroundedHandleStore>> = OnceLock::new();

thread_local! {
    static LAST_SCALAR_ATTESTATION: RefCell<Option<Arc<GroundedAttestation>>> = const { RefCell::new(None) };
}

fn store() -> &'static Mutex<GroundedHandleStore> {
    STORE.get_or_init(|| Mutex::new(GroundedHandleStore::default()))
}

fn canonicalize_confidence(confidence: f64) -> f64 {
    if confidence.is_finite() {
        confidence.clamp(0.0, 1.0)
    } else {
        1.0
    }
}

pub fn make_attestation(provenance: ProvenanceChain, confidence: f64) -> Arc<GroundedAttestation> {
    Arc::new(GroundedAttestation {
        provenance: Arc::new(provenance),
        confidence: canonicalize_confidence(confidence),
    })
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::provenance::ProvenanceChain;

    #[test]
    fn release_zero_is_noop() {
        assert!(release_handle(NULL_GROUNDED_HANDLE));
    }

    #[test]
    fn pointer_attestation_roundtrips_through_handle_for_string_descriptor() {
        let attestation = make_attestation(ProvenanceChain::with_retrieval("lookup", 1), 0.75);
        attach_pointer_attestation(42, attestation);
        let handle = register_handle_for_pointer(42);
        assert_ne!(handle, NULL_GROUNDED_HANDLE);
        assert_eq!(sources_for_handle(handle).unwrap(), vec!["lookup".to_string()]);
        assert!((confidence_for_handle(handle).unwrap() - 0.75).abs() < 1e-9);
        assert!(release_handle(handle));
    }

    #[test]
    fn pointer_attestation_roundtrips_for_struct_pointer() {
        // A struct heap pointer is just a `usize` key like any other
        // heap allocation; the same map and the same pair of helpers
        // serve struct attestations without a parallel storage path.
        let attestation =
            make_attestation(ProvenanceChain::with_retrieval("classify", 5), 0.9);
        attach_pointer_attestation(0xdead_beef_usize, attestation);
        let handle = register_handle_for_pointer(0xdead_beef_usize);
        assert_ne!(handle, NULL_GROUNDED_HANDLE);
        assert_eq!(
            sources_for_handle(handle).unwrap(),
            vec!["classify".to_string()]
        );
        assert!(release_handle(handle));
    }

    #[test]
    fn stale_handle_fails_after_release() {
        let attestation = make_attestation(ProvenanceChain::with_retrieval("lookup", 1), 1.0);
        attach_pointer_attestation(7, attestation);
        let handle = register_handle_for_pointer(7);
        assert!(release_handle(handle));
        assert!(!release_handle(handle));
    }

    #[test]
    fn scalar_attestation_roundtrips() {
        let attestation = make_attestation(ProvenanceChain::with_retrieval("classify", 2), 0.5);
        set_last_scalar_attestation(attestation);
        let handle = register_handle_for_last_scalar();
        assert_ne!(handle, NULL_GROUNDED_HANDLE);
        assert_eq!(sources_for_handle(handle).unwrap(), vec!["classify".to_string()]);
        assert!((confidence_for_handle(handle).unwrap() - 0.5).abs() < 1e-9);
        assert!(release_handle(handle));
    }
}
