//! Wasm-clean core of the Corvid agent runtime.
//!
//! `corvid-runtime-core` owns the deterministic, IO-free runtime
//! state: agent / prompt / tool dispatch, effect-row composition,
//! approval-token state, grounded-provenance state, the replay
//! state machine, the trace-event schema, and the suspend/resume
//! `HostRequest` / `HostResponse` bridge.
//!
//! Native capabilities — tokio, real HTTP clients, DB drivers,
//! OTel SDK, OAuth flows, signing keys, filesystem-backed replay
//! sinks — live in the sibling crate `corvid-runtime-host`. The
//! browser playground (`corvid-browser`) consumes this crate
//! directly and resolves `HostRequest`s through `wasm-bindgen-
//! futures` instead.
//!
//! The split exists so that `run_agent(...)` works in the browser
//! without a fork: one core, two hosts. See
//! `docs/meta/runtime-split-design.md` for the boundary rationale
//! (decisions D1-D6) and `docs/meta/33J7b-fresh-session.md` for
//! the slice plan that built it.
//!
//! This crate compiles to `wasm32-unknown-unknown`. CI enforces
//! that property on every push; if a wasm-blocking dep creeps in,
//! the build refuses and the regression surfaces immediately.

#![deny(unsafe_code)]

pub mod host;
pub mod provenance;

pub use host::{
    HostBridge, HostErrorKind, HostRequest, HostRequestKind, HostResponse, HostResponseKind,
    LlmMessage, SchemaVersion, TokenUsage,
};
pub use provenance::{GroundedValue, ProvenanceChain, ProvenanceEntry, ProvenanceKind};
