//! Canonical guarantee data + lookup helpers — slice 35
//! / contract registry, decomposed in Phase 20j-A8.
//!
//! `GUARANTEE_REGISTRY` is the single source of truth for every
//! public Corvid guarantee. The lookup helpers here all walk the
//! registry slice; nothing else is allowed to query it directly,
//! so this file owns "what's in the registry and how to read it."
//!
//! Honesty rules over the registry data live in [`super::validate`]
//! and are enforced by [`super::validate::validate_slice`]. Doc
//! generation lives in [`super::render`]. The signed-cdylib claim
//! whitelist lives in [`super::signed_claim`].

use super::types::{Guarantee, GuaranteeClass, GuaranteeKind, Phase};

/// Canonical guarantee table.
///
/// Order matters only for stable doc generation — the generator
/// (Slice 35-D) emits rows in declaration order, so adding a new
/// guarantee at the bottom keeps the existing doc stable. Entries
/// that conceptually belong together are grouped by kind.
pub static GUARANTEE_REGISTRY: &[Guarantee] = &[
    // ----- Approval boundaries ------------------------------------
    Guarantee {
        id: "approval.dangerous_call_requires_token",
        kind: GuaranteeKind::Approval,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "A call site invoking a `@dangerous` tool must have an `approve` \
             token lexically in scope; otherwise the typechecker rejects \
             the program.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::dangerous_tool_with_matching_approve_is_ok",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::dangerous_tool_without_approve_is_compile_error",
            "crates/corvid-types/src/tests.rs::tagged_unapproved_dangerous_call_carries_approval_guarantee_id",
        ],
    },
    Guarantee {
        id: "approval.trust_tier_requires_token",
        kind: GuaranteeKind::Approval,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "A call site invoking a tool whose composed effect row carries `trust: supervisor_required` or `trust: human_required` must have an `approve` token lexically in scope, exactly like a `dangerous` tool — the requirement is DERIVED from the trust tier so a declaration that forgets the `dangerous` marker still gets compile-time protection. The diagnostic names the deriving effect and tier. `autonomous` and the confidence-gated `autonomous_if_confident(...)` (which typechecks as autonomous and escalates at runtime) derive nothing.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::high_trust_tool_with_matching_approve_is_ok",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::high_trust_tool_without_approve_is_compile_error",
            "crates/corvid-types/src/tests.rs::high_trust_error_names_deriving_effect_and_tier",
        ],
    },
    Guarantee {
        id: "approval.token_lexical_only",
        kind: GuaranteeKind::Approval,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "Approval tokens are lexically scoped — they cannot be returned, \
             stored in fields, or passed across opaque boundaries to \
             unlock a call site outside the original `approve` block.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::outer_approve_authorizes_inner_call",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::approve_does_not_leak_out_of_if_branch",
            "crates/corvid-types/src/tests.rs::mutation_nested_inner_approve_does_not_authorize_outer_call",
        ],
    },
    Guarantee {
        id: "approval.dangerous_marker_preserved",
        kind: GuaranteeKind::Approval,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::TypeCheck,
        description:
            "A `@dangerous` marker cannot be erased by re-exporting or \
             aliasing the tool through another module — every public \
             alias preserves the original danger annotation.",
        out_of_scope_reason:
            "Structural property of the language, not a separately-fired \
             diagnostic. Corvid's source syntax has no `import use` form \
             that can declare the alias's effect — aliases inherit their \
             source's `@dangerous` marker by construction. The property \
             is verified indirectly: when a dangerous imported tool is \
             aliased and then called without approve, the parent \
             diagnostic `approval.dangerous_call_requires_token` fires \
             correctly, which is only possible because the marker was \
             preserved through the alias. The cited test_refs assert \
             that parent-diagnostic firing through the alias path. \
             Phase 35V-T1-B (2026-05-08) downgraded this row from \
             `Static` to `OutOfScope` because no separate diagnostic \
             site exists to tag with this id; the property remains \
             documentary, the enforcement remains structural via the \
             parent diagnostic. A future syntax slice that introduces \
             an explicit alias-effect-override surface would promote \
             this row back to `Static` with a tagged diagnostic at the \
             override-rejection site.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "approval.reachable_entrypoints_require_contract",
        kind: GuaranteeKind::Approval,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "Externally reachable routes, schedules, and exported agents \
             are walked through their reachable agent calls; any reachable \
             `@dangerous` tool call must still have a matching lexical \
             approval contract.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::server_route_approve_authorizes_dangerous_tool",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::server_route_reachability_reports_helper_without_approval",
            "crates/corvid-types/src/tests.rs::schedule_reachability_reports_job_without_approval",
        ],
    },
    // ----- Effect rows --------------------------------------------
    Guarantee {
        id: "effect_row.body_completeness",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "A function's declared effect row must cover every effect \
             actually produced by its body (including effects of called \
             functions); under-reporting is a compile error.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::mutation_tool_uses_declared_effect_is_ok",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::mutation_baseline_trust_violation_exists",
            "crates/corvid-types/src/tests.rs::mutation_multiple_effects_on_one_tool_compose_cost_and_trust",
        ],
    },
    Guarantee {
        id: "effect_row.caller_propagation",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::TypeCheck,
        description:
            "Callers inherit the union of their callees' effects unless \
             they declare a wider row; callers cannot silently shrink the \
             effect surface.",
        out_of_scope_reason:
            "Subsumed by `effect_row.body_completeness` at the diagnostic \
             level. The shipped `analyze_effects` analysis composes \
             effects across calls (`collect_body_effects` walks the \
             body and unions every called tool/prompt/agent's effect \
             row into the composed profile) and fires a single \
             `EffectConstraintViolation` per dimension when the \
             declared row doesn't cover the composed result. The \
             violation message says \"dimension X: constraint requires \
             Y, but composed value is Z\" without distinguishing \
             whether the offending contribution came from a direct \
             body call or from a transitive callee — the unified \
             analysis treats them identically. The user's mitigation \
             is the same in both cases: widen the declared effect \
             row to cover the composed value. Phase 35V-T1-B \
             (2026-05-08) downgraded this row from `Static` to \
             `OutOfScope` because the analyzer's `ConstraintViolation` \
             struct does not carry a body-vs-callee source field, so \
             there is no discriminable diagnostic site to tag with \
             this id; the property is documentary, the enforcement \
             is via the parent's unified diagnostic. A future slice \
             that extends `ConstraintViolation` with a `source` field \
             plus per-violation discrimination at the firing site \
             would promote this row back to `Static`.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "effect_row.import_boundary",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::Static,
        phase: Phase::Resolve,
        description:
            "Cross-module imports preserve effect annotations exactly; \
             an importer cannot use a re-exported function with a \
             stripped or weakened effect row.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::python_import_with_unsafe_effect_warns",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::python_import_without_effects_is_rejected",
        ],
    },
    // ----- io_source dimension (Phase 33S1c) ----------------------
    //
    // The executing file-I/O surface (`io.read_text` / `io.write_text`
    // / `io.list_dir`, declared in `std/io.cor`) ships three runtime-
    // checked guarantees. They classify what the runtime enforces
    // about ANY call to these tools: path confinement against the
    // configured `[io] root`, write-quarantine on substitute-mode
    // replay, and read-passthrough on substitute-mode replay (which
    // is gated by the same trace contract that prevents bypass).
    //
    // The @deterministic-rejection property is NOT a separate
    // guarantee here — it's covered by the existing decl-replayability
    // rule (`crates/corvid-types/src/checker/decl_replayability.rs:184`)
    // that rejects all tool calls inside `@deterministic` bodies
    // regardless of effect. Slice 33S1b added pinning tests for the
    // io_read / io_write cases at
    // `crates/corvid-types/src/tests.rs::deterministic_agent_calling_io_*_tool_is_rejected`
    // — they're part of the broader replay.deterministic_pure_path
    // proof matrix, not a new io-specific guarantee.
    Guarantee {
        id: "secrets.trace_never_carries_value",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "The executing `secret_read` tool returns the real secret value to the program, but the recorded ToolResult trace event carries a redacted copy (`<redacted:XY>` marker, `value_redacted: true`) — traces never persist secret values. Substitute-mode replay RE-READS the live environment instead of substituting (there is nothing usable to substitute), so a changed environment diverges honestly. Residual channel stated as explicit non-scope: a secret the program forwards into another tool's arguments is recorded by that tool's own events; the structural taint fix (an opaque SecretHandle value) is the tracked post-v1.0 deepening.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs::secret_read_returns_real_value_to_the_program",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_rereads_secret_from_live_environment",
        ],
        adversarial_test_refs: &[
            "crates/corvid-driver/tests/executing_secrets_cache_through_driver.rs::secret_value_never_lands_in_the_trace",
        ],
    },
    Guarantee {
        id: "io_source.fs_path_confinement",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Every call to an executing file-I/O tool (io.read_text / \
             io.write_text / io.list_dir, from std/io.cor) resolves the \
             caller's path through the project's configured `[io] root` \
             before reaching the filesystem. Paths that traverse out of \
             the root (via `..` segments or absolute-prefix escapes) \
             are refused with a structured diagnostic naming the \
             offending path AND the configured root. When no `[io] root` \
             is configured, every call fails closed — the executing \
             file-I/O surface refuses to operate without an explicit \
             security boundary declared in corvid.toml. At the language \
             surface the refusal is an honest Err value (the io/db tools \
             return Result), so programs observe the diagnostic without \
             the boundary weakening.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/io.rs::io_tool_policy_relative_root_resolves_against_corvid_toml_dir",
            "crates/corvid-runtime/tests/executing_io_tools.rs::executing_io_tools_resolve_both_absolute_and_relative_roots",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/io.rs::io_tool_policy_rejects_parent_traversal_escape",
            "crates/corvid-runtime/src/io.rs::io_tool_policy_strips_leading_separator_to_confine_absolute_inputs",
            "crates/corvid-runtime/src/io.rs::io_tool_policy_unconfigured_fails_closed_on_resolve",
            "crates/corvid-runtime/tests/executing_io_tools.rs::executing_io_tools_reject_path_traversal_with_clear_diagnostic",
            "crates/corvid-runtime/tests/executing_io_tools.rs::executing_io_tools_fail_closed_without_io_root_configured",
        ],
    },
    Guarantee {
        id: "io_source.fs_write_quarantine_on_replay",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A Substitute-mode replay runtime refuses every executing \
             file-write call. Both the low-level `IoRuntime::write_text` \
             path AND the `Runtime::call_tool(\"io.write_text\", ...)` \
             dispatch path are covered: the low-level path returns \
             QuarantineViolation directly; the dispatch path goes \
             through the replay-substitution path first (so writes \
             either substitute from the recorded trace OR diverge — \
             they never reach the live filesystem). The filesystem \
             is provably untouched in both cases.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/tests/executing_io_tools.rs::executing_io_tools_round_trip_through_runtime_dispatch",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_quarantines_io_writes",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_io_write_tool_dispatch_from_escaping_to_filesystem",
        ],
    },
    Guarantee {
        id: "io_source.fs_read_quarantine_on_replay",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A Substitute-mode replay runtime gates every executing \
             file-read call (read_text / list_dir). The low-level \
             `IoRuntime::read_text` path passes through transparently \
             during replay (reads don't escape the process and the \
             quarantine flag is write-only). The `Runtime::call_tool` \
             dispatch path goes through replay-substitution first, so \
             dispatch-path reads either substitute from the recorded \
             trace OR diverge when no recorded event matches — they \
             never reach the live filesystem unless the trace \
             prescribed it.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_passes_through_io_reads",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_io_read_tool_dispatch_without_recorded_event",
        ],
    },
    // ----- io_source dimension (Phase 33S2c) ----------------------
    //
    // The executing HTTP-client surface (`http_get` / `http_post_json`,
    // declared in `std/http.cor`) ships three runtime-checked
    // guarantees. They classify what the runtime enforces about ANY
    // call to these tools: a structural SSRF block that refuses
    // private / loopback / link-local hosts regardless of
    // allowlist (load-bearing security floor), a required
    // `[http] allow` allowlist that fails closed when unconfigured,
    // and replay quarantine that refuses POST escapes and gates
    // GETs through the substitution path.
    //
    // The @deterministic-rejection property is NOT a separate
    // guarantee here — same rationale as the io.* surface in 33S1c.
    // It's covered by the existing decl-replayability rule
    // (`crates/corvid-types/src/checker/decl_replayability.rs`)
    // that rejects all tool calls inside `@deterministic` bodies
    // regardless of effect. Slice 33S2c adds pinning tests for the
    // http_get / http_post_json cases at
    // `crates/corvid-types/src/tests.rs::deterministic_agent_calling_http_*_tool_is_rejected`
    // — they're part of the broader replay.deterministic_pure_path
    // proof matrix, not a new http-specific guarantee.
    Guarantee {
        id: "io_source.http_ssrf_structural_block",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Every call to an executing HTTP tool (http_get / \
             http_post_json, from std/http.cor) parses the request \
             URL and refuses any host that lexically resolves to a \
             private RFC1918 range (10.0.0.0/8, 172.16.0.0/12, \
             192.168.0.0/16), loopback (127.0.0.0/8 + ::1), \
             link-local (169.254.0.0/16 + fe80::/10), unspecified \
             (0.0.0.0/8 + ::), ULA (fc00::/7), or the `localhost` \
             DNS alias. The block is a STRUCTURAL property of the \
             language: it runs regardless of `[http] allow` \
             contents — even a fully-misconfigured allowlist \
             containing `127.0.0.1` cannot bypass it. This is the \
             security floor underneath the configurable allowlist. \
             At the language surface the refusal is an honest Err \
             value (the http tools return Result), so programs \
             observe the diagnostic without the floor weakening.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/http.rs::http_egress_policy_allowlist_permits_matching_host",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/http.rs::http_egress_policy_ssrf_block_refuses_all_private_loopback_ipv4_ranges",
            "crates/corvid-runtime/src/http.rs::http_egress_policy_ssrf_block_refuses_ipv6_loopback_and_link_local",
            "crates/corvid-runtime/src/http.rs::http_egress_policy_ssrf_block_refuses_localhost_dns_alias",
            "crates/corvid-driver/tests/executing_http_through_driver.rs::ssrf_block_rejects_loopback_url_even_when_allowlist_contains_it",
        ],
    },
    Guarantee {
        id: "io_source.http_allowlist_enforcement",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Every call to an executing HTTP tool checks the \
             request URL's host against the project's configured \
             `[http] allow = [...]` list (or the \
             `CORVID_HTTP_ALLOW` env override). Case-insensitive \
             exact-host comparison. When no allowlist is \
             configured (missing `[http]` section, empty `allow`, \
             or unset env), every call fails closed with a \
             structured diagnostic naming the requested URL, the \
             missing config, and the env-override pathway. This \
             is the configurable layer on top of the always-on \
             SSRF block. At the language surface the refusal is an \
             honest Err value (the http tools return Result), so \
             programs observe the diagnostic without the gate \
             weakening.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/http.rs::http_egress_policy_allowlist_permits_matching_host",
            "crates/corvid-driver/src/run.rs::corvid_toml_with_http_allow_produces_configured_policy",
            "crates/corvid-driver/src/run.rs::env_var_overrides_corvid_toml_http_allow",
            "crates/corvid-driver/tests/executing_http_through_driver.rs::real_corvid_program_performs_get_through_executing_http_dispatch",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/http.rs::http_egress_policy_unconfigured_fails_closed_on_check",
            "crates/corvid-runtime/src/http.rs::http_egress_policy_empty_allow_list_is_unconfigured",
            "crates/corvid-runtime/src/http.rs::http_egress_policy_allowlist_refuses_unlisted_host_with_clear_diagnostic",
            "crates/corvid-driver/src/run.rs::corvid_toml_with_empty_http_allow_produces_unconfigured_policy",
            "crates/corvid-driver/src/run.rs::corvid_toml_without_http_section_produces_unconfigured_policy",
            "crates/corvid-driver/tests/executing_http_through_driver.rs::missing_http_allowlist_fails_closed_with_actionable_diagnostic",
        ],
    },
    // ----- io_source dimension (Phase 33S3d) ----------------------
    //
    // The executing SQLite surface (`db_open` / `db_query` /
    // `db_execute`, declared in `std/db.cor`) ships three
    // runtime-checked guarantees. They classify what the runtime
    // enforces about ANY call to these tools: structural
    // parameter-binding-only (no string interpolation path exists,
    // the typechecker's `List<DbParam>` signature forces every
    // value through typed constructors), write quarantine on
    // replay (db_execute refuses with `QuarantineViolation
    // { surface: "db", .. }` during Substitute-mode), and read
    // passthrough on replay (db_query is not blocked by the
    // write-quarantine; a future slice adds the trace-substitution
    // upper gate).
    //
    // Path confinement is NOT a separate sqlite guarantee row
    // because `db_open` reuses `IoToolPolicy::resolve` — the
    // existing `io_source.fs_path_confinement` guarantee carries
    // the property for both the io tools AND `db_open`. The
    // sqlite test refs are added to fs_path_confinement's
    // adversarial set above so the cross-reference sentinel
    // confirms the property holds across both surfaces.
    //
    // The @deterministic-rejection property is NOT a separate
    // guarantee here — same rationale as 33S1c (io.*) and 33S2c
    // (http.*): the existing decl-replayability rule covers all
    // tool calls regardless of effect. Slice 33S3d adds two
    // pinning tests at
    // `crates/corvid-types/src/tests.rs::deterministic_agent_calling_db_*_tool_is_rejected`
    // so the rule can't quietly relax for SQLite either.
    Guarantee {
        id: "io_source.sqlite_parameter_binding_only",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Every parameter passed to `db_query` / `db_execute` \
             (executing SQLite tools from std/db.cor) flows \
             through `rusqlite::params_from_iter` over the typed \
             `DbValue` enum — there is no string-interpolation \
             path on the dispatch. The typechecker's `List<DbParam>` \
             signature forces every value through the typed \
             constructors (`db_param_int`, `db_param_text`, \
             `db_param_float`, `db_param_bool`, `db_param_null`); \
             a literal `\"'; DROP TABLE users; --\"` placed in a \
             `db_param_text` value survives as TEXT data and \
             never reaches SQLite's parser. This is the load-\
             bearing structural property: SQL injection is \
             prevented by the language's type system + the \
             runtime's binding path, not by escaping or \
             sanitisation.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/db.rs::db_handle_registry_round_trip_against_memory_database",
            "crates/corvid-driver/tests/executing_sqlite_through_driver.rs::real_corvid_program_round_trips_data_through_executing_sqlite_dispatch",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/db.rs::db_param_text_with_sql_metacharacters_is_bound_as_data",
            "crates/corvid-driver/tests/executing_sqlite_through_driver.rs::db_param_text_with_sql_metacharacters_survives_round_trip_through_real_corvid_program",
        ],
    },
    Guarantee {
        id: "io_source.sqlite_write_quarantine_on_replay",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A Substitute-mode replay runtime refuses every \
             executing `db_execute` call. The dispatch path goes \
             through `Runtime::db_execute_tool` which delegates \
             to `DbHandleRegistry::execute`; the registry's \
             write-quarantine flag (flipped by \
             `RuntimeBuilder::build` during replay) short-\
             circuits with `QuarantineViolation { surface: \
             \"db\", .. }` regardless of SQL contents — \
             INSERTs, UPDATEs, DELETEs, and DDL all blocked. \
             The database is provably untouched during replay.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/db.rs::db_handle_registry_round_trip_against_memory_database",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/db.rs::db_handle_registry_quarantine_blocks_execute_with_db_surface_violation",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_db_execute_dispatch_from_escaping_to_database",
        ],
    },
    Guarantee {
        id: "io_source.sqlite_read_passthrough_on_replay",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A Substitute-mode replay runtime passes `db_query` \
             calls through to the live `DbHandleRegistry` — \
             SQLite reads don't escape the process so the \
             write-quarantine flag is the floor for mutations \
             only. A follow-up slice will add the trace-\
             substitution upper gate (replay db_query against a \
             recorded row event yields the recorded rows; a \
             missing event diverges); 33S3d pins the \
             dispatch-side read-passthrough property so a future \
             refactor can't silently flip the policy and start \
             blocking reads during replay.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/db.rs::db_handle_registry_quarantine_does_not_block_query",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_does_not_block_executing_db_query_dispatch_during_write_quarantine",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/db.rs::db_handle_registry_quarantine_does_not_block_query",
        ],
    },
    Guarantee {
        id: "io_source.http_quarantine_on_replay",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A Substitute-mode replay runtime refuses every \
             executing HTTP call. POST calls through the \
             `Runtime::call_tool(\"http_post_json\", ...)` \
             dispatch path go through the replay-substitution \
             path first (so they substitute from the recorded \
             trace OR diverge — they never reach the live \
             network); the `HttpClient::quarantine` flag is the \
             floor underneath that returns QuarantineViolation \
             from any direct `HttpClient::send` call. GET calls \
             through dispatch are also gated by the substitution \
             path (read-quarantine equivalent — they substitute \
             OR diverge, never reaching the live network).",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_quarantines_http_client_send",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_http_post_tool_dispatch_from_escaping_to_network",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_blocks_executing_http_get_tool_dispatch_without_recorded_event",
        ],
    },
    // ----- json (Phase 33R5b-c) -----------------------------------
    //
    // The executing JSON surface (`json_parse` / `json_get_*` /
    // `json_object_new` / `json_object_set_*` / `json_object_finish`
    // declared in `std/json.cor`) ships two RuntimeChecked
    // guarantees plus the typed-decoder convention (which uses
    // the same parse path so its safety properties are inherited).
    //
    // JSON has no security boundary beyond serde validation —
    // no I/O, no network, no filesystem, no SQLite. The
    // properties below are PURE SAFETY properties: malformed
    // input is recoverable, typed-accessor mismatches are
    // recoverable. Together they make "calling JSON tools is
    // structurally crash-free" a load-bearing language
    // property — a Corvid program can route JSON failures
    // through the standard Result<_, String> envelope without
    // ever risking a runtime panic or escape.
    //
    // The @deterministic-rejection property is NOT a separate
    // guarantee here — same rationale as 33S1c (io.*), 33S2c
    // (http.*), 33S3d (sqlite.*): the existing decl-replayability
    // rule covers all tool calls regardless of effect. 33R5b-c
    // adds two pinning tests at
    // `crates/corvid-types/src/tests.rs::deterministic_agent_calling_json_*_tool_is_rejected`
    // so a future relaxation of the rule would surface as
    // test breakage, not a silent regression.
    Guarantee {
        id: "json.parse_safety_no_panic",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Calling `json_parse(text)` against arbitrary bytes \
             returns `Result::Err(message)` rather than panicking. \
             The runtime's `crate::json::parse` routes through \
             `serde_json::from_str` whose parse failure surfaces \
             as a structured error description; the Result wraps \
             the error in the standard `Err(String)` envelope so \
             user code can pattern-match via `?` propagation and \
             route the diagnostic up to its caller. A Corvid \
             program calling `json_parse` on malformed text \
             cannot crash the runtime regardless of what bytes \
             are in the input. The typed-decoder convention \
             (`decode_<X>_from_json`) inherits this property — \
             malformed input through the decoder dispatch path \
             also returns `Result::Err`.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/json.rs::parse_round_trips_a_typical_object",
            "crates/corvid-driver/tests/executing_json_through_driver.rs::real_corvid_program_round_trips_data_through_opaque_json_dispatch",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/json.rs::malformed_json_returns_recoverable_error_never_panics",
            "crates/corvid-driver/tests/executing_json_through_driver.rs::malformed_json_returns_result_err_through_real_corvid_program",
        ],
    },
    Guarantee {
        id: "json.field_type_safety_at_access_boundary",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Each typed accessor on the executing JSON surface \
             (`json_get_int` / `json_get_float` / `json_get_string` \
             / `json_get_bool` / `json_get_object` / `json_get_array`) \
             returns `Result<T, String>` where the Err branch fires \
             on missing fields AND on type mismatches. \
             `json_get_int(value, field)` against a string-valued \
             field returns `Err(\"field 'x' is not an Int\")` rather \
             than coercing or panicking. The typed-decoder \
             convention inherits the same property — shape \
             mismatches (a JSON String where the user declared an \
             Int) flow through `json_to_value` against the target \
             struct and surface as `Result::Err` with a structured \
             diagnostic.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/json.rs::get_object_returns_subtree_for_further_typed_access",
            "crates/corvid-driver/tests/executing_json_through_driver.rs::real_corvid_program_decodes_typed_struct_via_decode_x_from_json_convention",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/json.rs::typed_accessor_mismatch_returns_recoverable_error",
            "crates/corvid-runtime/src/json.rs::missing_field_returns_recoverable_error_naming_the_field",
            "crates/corvid-driver/tests/executing_json_through_driver.rs::typed_decoder_shape_mismatch_returns_result_err_through_real_corvid_program",
        ],
    },
    // ----- Grounded<T> --------------------------------------------
    Guarantee {
        id: "taint.untrusted_cannot_reach_dangerous",
        kind: GuaranteeKind::EffectRow,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "Untrusted content — a value from a `data: untrusted` effect source (retrieved              documents, user messages, untrusted MCP output) or the output of a prompt that              consumed one — is typed `Tainted<T>` and cannot parameterize an              approval-requiring call (a `dangerous` tool, or one whose trust tier is              `supervisor_required`/`human_required`). Taint is never assignable to `T` and is              unwrapped only by the explicit, greppable `trusted(expr)` boundary. This makes              prompt injection (OWASP LLM #1) a compile error: attacker-influenced data cannot              reach a consequential action without a human-reviewed sanitization point.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::trusted_boundary_unwraps_taint_to_reach_dangerous",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::tainted_prompt_output_cannot_reach_dangerous_tool",
        ],
    },
    Guarantee {
        id: "grounded.provenance_required",
        kind: GuaranteeKind::Grounded,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "Constructing a `Grounded<T>` value requires citing a source; \
             unsourced `Grounded` construction is a compile error.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::mutation_direct_grounded_return_with_retrieval_chain_is_ok",
            "crates/corvid-types/src/tests.rs::grounded_connector_tool_return_is_wrapped_and_flows",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::mutation_grounded_return_without_retrieval_errors",
            "crates/corvid-types/src/tests.rs::grounded_connector_return_strip_is_the_tracked_legacy_coercion",
        ],
    },
    Guarantee {
        id: "grounded.propagation_across_calls",
        kind: GuaranteeKind::Grounded,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::TypeCheck,
        description:
            "Provenance is preserved across function boundaries — a \
             `Grounded<T>` returned from a callee retains its citation \
             chain into the caller without separate annotation.",
        out_of_scope_reason:
            "Subsumed by `grounded.provenance_required` at the diagnostic \
             level. The shipped grounded-return analysis fires a single \
             `UngroundedReturn` diagnostic when a function declares a \
             `Grounded<T>` return type but the returned expression's \
             provenance chain is empty. The check is unified: it does \
             not distinguish whether the missing provenance came from \
             a directly-constructed value (parent's framing: \
             provenance must be cited at construction) or from a \
             value flowed across a callee boundary (this row's \
             framing: provenance must be preserved across calls). \
             The user's mitigation is the same in both cases: ensure \
             the returned value carries a non-empty provenance chain. \
             Phase 35V-T1-B (2026-05-08) downgraded this row from \
             `Static` to `OutOfScope` because the analyzer fires one \
             diagnostic for both perspectives; there is no \
             discriminable site to tag separately. The property is \
             documentary; the enforcement is via the parent's unified \
             diagnostic. A future slice that splits the analyzer to \
             distinguish construction-site failures from \
             call-boundary propagation failures would promote this \
             row back to `Static`.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "grounded.no_laundering",
        kind: GuaranteeKind::Grounded,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "An agent annotated `@grounded_pure` fails compile if its body \
             launders a `Grounded<T>` value into a non-grounded slot — \
             either via the silent legacy coercion at a slot-check site \
             (return / parameter / field), an explicit \
             `.unwrap_discarding_sources()` call, or a transitive call \
             into another agent not itself marked `@grounded_pure`. The \
             moat composes through the call graph the same way \
             `@deterministic` does.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::grounded_pure_passes_when_body_preserves_grounded",
            "crates/corvid-types/src/tests.rs::grounded_pure_passes_when_calling_another_grounded_pure_agent",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::grounded_pure_rejects_implicit_coercion",
            "crates/corvid-types/src/tests.rs::grounded_pure_rejects_explicit_unwrap",
            "crates/corvid-types/src/tests.rs::grounded_pure_rejects_call_to_non_grounded_pure_agent",
        ],
    },
    // ----- Budgets ------------------------------------------------
    Guarantee {
        id: "budget.compile_time_ceiling",
        kind: GuaranteeKind::Budget,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "An agent annotated `@budget($X)` fails compile if the sum of \
             statically known per-call costs along any reachable path \
             exceeds `$X`.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::multi_dimensional_budget_within_bound_is_clean",
            "crates/corvid-types/src/tests.rs::mutation_budget_within_limit_is_ok",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::multi_dimensional_budget_violation_reports_path",
            "crates/corvid-types/src/tests.rs::mutation_budget_exceeded_is_effect_violation",
        ],
    },
    Guarantee {
        id: "budget.runtime_termination",
        kind: GuaranteeKind::Budget,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::Runtime,
        description:
            "Live runtime termination of an agent when actual runtime cost \
             crosses the `@budget($X)` threshold mid-execution.",
        out_of_scope_reason:
            "Today Corvid enforces budgets at compile time via \
             `budget.compile_time_ceiling`, and the runtime observes \
             per-call cost in trace events; live mid-execution \
             termination on threshold crossing is not yet implemented. \
             A future slice can promote this entry back to \
             `RuntimeChecked` once the enforcement ships. The compile-time \
             ceiling is the load-bearing guarantee for v1.0.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    // ----- Confidence ---------------------------------------------
    Guarantee {
        id: "confidence.min_threshold",
        kind: GuaranteeKind::Confidence,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "An agent annotated `@min_confidence(X)` requires every input \
             carrying a confidence tag to meet `X`; lower-confidence \
             inputs are rejected at the call site.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::min_confidence_passes_when_composed_confidence_meets_floor",
            "crates/corvid-types/src/tests.rs::tagged_invalid_confidence_carries_confidence_guarantee_id",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::min_confidence_fires_when_composed_confidence_below_floor",
            "crates/corvid-types/src/tests.rs::effect_confidence_out_of_range_is_rejected",
        ],
    },
    // ----- Trust --------------------------------------------------
    Guarantee {
        id: "trust.constraint_enforcement",
        kind: GuaranteeKind::Trust,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "An agent annotated `@trust(<level>)` (or \
             `@trust(autonomous_if_confident(threshold))`) fails compile \
             when the agent's body composes a trust dimension stricter \
             than the declared ceiling — e.g. an `@trust(autonomous)` \
             agent that reaches a `trust: human_required` tool without \
             an `approve` boundary is rejected. The lattice is \
             `autonomous < supervisor_required < human_required`; the \
             confidence-gated variant treats `autonomous_if_confident(t)` \
             as `autonomous` at typecheck and routes to `human_required` \
             at runtime when composed confidence < t. Added 2026-06-05 \
             under slice 33Q3 so `@trust` annotations participate in \
             `corvid build --sign`'s claim coverage and surface in \
             `claim --explain` output.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::mutation_budget_within_limit_is_ok",
            "crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_accepts_trust_constrained_agent",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::mutation_baseline_trust_violation_exists",
            "crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_rejects_trust_when_id_missing_from_descriptor",
        ],
    },
    // ----- Replay -------------------------------------------------
    Guarantee {
        id: "parallel.cancellation_reversibility",
        kind: GuaranteeKind::Replay,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A `parallel:` block fails fast, but a branch past a \
             NON-REVERSIBLE effect boundary is never cancelled (slice \
             52d): the moment an arm dispatches an irreversible tool \
             (composed `reversible: false`) it is shielded and runs to \
             completion even when a sibling fails; only arms that have \
             done nothing irreversible are cancelled, and they stop at a \
             tool-dispatch boundary BEFORE their next effect (cooperative, \
             so no irreversible action is ever left half-done). The live \
             cancellation is recorded per arm (`parallel.outcomes`: \
             outcome + `crossed_irreversible` + terminal dispatch count), \
             and Substitute-mode replay REPRODUCES it deterministically \
             — a cancelled arm replays to its recorded dispatch boundary \
             and stops, a shielded arm reaches its recorded terminal, and \
             non-cancelling blocks replay byte-identically. A trace \
             missing its outcomes record diverges honestly rather than \
             inventing a cancellation.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-vm/src/tests/parallel.rs::arm_past_irreversible_boundary_is_not_cancelled",
            "crates/corvid-vm/src/tests/parallel.rs::replay_reproduces_a_recorded_cancellation",
        ],
        adversarial_test_refs: &[
            "crates/corvid-vm/src/tests/parallel.rs::reversible_arm_is_cancelled_after_a_sibling_fails",
            "crates/corvid-vm/src/tests/parallel.rs::replay_reproduces_multiple_cancellations",
            "crates/corvid-vm/src/tests/parallel.rs::replay_reproduces_a_shielded_arm_reaching_its_terminal",
            "crates/corvid-vm/src/tests/parallel.rs::replay_with_missing_outcomes_record_diverges_honestly",
        ],
    },
    Guarantee {
        id: "replay.deterministic_pure_path",
        kind: GuaranteeKind::Replay,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A trace recorded from a `@replayable` agent reproduces \
             deterministically across `corvid replay` invocations on the \
             same compiled binary; non-deterministic divergence raises \
             the documented replay-divergence error.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::replayable_agent_with_pure_body_compiles_clean",
            "crates/corvid-types/src/tests.rs::deterministic_agent_with_pure_body_compiles_clean",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::deterministic_agent_calling_tool_is_rejected",
            "crates/corvid-types/src/tests.rs::deterministic_agent_calling_prompt_is_rejected",
        ],
    },
    Guarantee {
        id: "replay.trace_signature",
        kind: GuaranteeKind::Replay,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Trace receipts produced with `--sign` carry a DSSE envelope \
             whose signature `corvid receipt verify` checks against the \
             supplied verifying key.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/tests/receipt_signing.rs::sign_then_verify_roundtrips_end_to_end",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_envelope_signed_with_different_key",
            "crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_tampered_payload",
        ],
    },
    // ----- Provenance / receipts ----------------------------------
    Guarantee {
        id: "provenance_trace.receipt_signature",
        kind: GuaranteeKind::ProvenanceTrace,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid receipt verify` rejects any DSSE-wrapped receipt \
             whose signature does not validate against the supplied \
             verifying key, with a non-zero exit and the documented \
             `verification failed` message.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/tests/receipt_signing.rs::sign_then_verify_roundtrips_end_to_end",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_envelope_signed_with_different_key",
            "crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_tampered_payload",
        ],
    },
    // ----- ABI descriptor -----------------------------------------
    Guarantee {
        id: "abi_descriptor.cdylib_emission",
        kind: GuaranteeKind::AbiDescriptor,
        class: GuaranteeClass::Static,
        phase: Phase::Codegen,
        description:
            "Every `corvid build --target=cdylib` output exports a \
             `CORVID_ABI_DESCRIPTOR` symbol whose payload is the canonical \
             effect/approval/provenance surface for the compiled program.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-codegen-cl/tests/cdylib_emission.rs::cdylib_target_produces_shared_library_file",
            "crates/corvid-codegen-cl/tests/cdylib_emission.rs::cdylib_symbol_is_resolvable_via_dlopen",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/tests/build_cdylib.rs::cli_build_cdylib_fails_cleanly_on_non_scalar_signature",
        ],
    },
    Guarantee {
        id: "abi_descriptor.byte_determinism",
        kind: GuaranteeKind::AbiDescriptor,
        class: GuaranteeClass::Static,
        phase: Phase::Codegen,
        description:
            "Two byte-identical Corvid sources compiled with the same \
             toolchain version produce byte-identical descriptor JSON; \
             the descriptor is canonical, not pretty-printed.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-abi/tests/determinism.rs::identical_source_produces_byte_identical_descriptor_modulo_generated_at",
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::descriptor_bytes_are_byte_identical_across_two_emissions_of_same_source",
        ],
        adversarial_test_refs: &[
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::descriptor_section_rejects_random_byte_flips",
        ],
    },
    Guarantee {
        id: "abi_descriptor.bilateral_source_match",
        kind: GuaranteeKind::AbiDescriptor,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::AbiVerify,
        description:
            "`corvid-abi-verify --source <file> <cdylib>` independently \
             rebuilds the ABI descriptor from source and byte-compares it \
             against the embedded `CORVID_ABI_DESCRIPTOR` symbol; mismatch \
             is rejected before host acceptance.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-abi-verify/src/lib.rs::verifier_accepts_matching_cdylib_descriptor",
            "crates/corvid-abi-verify/src/lib.rs::verifier_accepts_matching_cdylib_with_imported_agent",
        ],
        adversarial_test_refs: &[
            "crates/corvid-abi-verify/src/lib.rs::verifier_rejects_source_descriptor_mismatch",
        ],
    },
    // ----- ABI attestation ----------------------------------------
    Guarantee {
        id: "abi_attestation.envelope_signature",
        kind: GuaranteeKind::AbiAttestation,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::AbiVerify,
        description:
            "`corvid receipt verify-abi` rejects a signed cdylib whose \
             attestation envelope does not validate against the supplied \
             verifying key, exiting 1 with `attestation verification \
             failed`.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_verifies_against_matching_key",
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::signing_key_round_trip_baseline",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_rejects_wrong_key",
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::dsse_envelope_signature_tampering_is_rejected",
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::dsse_envelope_payload_tampering_is_rejected",
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::dsse_envelope_payload_type_swap_is_rejected",
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::attestation_section_rejects_every_magic_or_version_byte_flip",
            "crates/corvid-abi/tests/byte_fuzz_corpus.rs::attestation_section_body_mutations_break_signature_verification",
        ],
    },
    Guarantee {
        id: "abi_attestation.descriptor_match",
        kind: GuaranteeKind::AbiAttestation,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::AbiVerify,
        description:
            "After signature validation, the recovered attestation \
             payload must bit-match the embedded \
             `CORVID_ABI_DESCRIPTOR`; mismatch is rejected even when \
             the signature is valid.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_verifies_against_matching_key",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_rejects_wrong_key",
        ],
    },
    Guarantee {
        id: "abi_attestation.absent_reports_unsigned",
        kind: GuaranteeKind::AbiAttestation,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::AbiVerify,
        description:
            "`corvid receipt verify-abi` on a cdylib lacking the \
             `CORVID_ABI_ATTESTATION` symbol exits 2 with the documented \
             `unsigned` message, leaving the host policy to decide \
             whether to accept it.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_verifies_against_matching_key",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/tests/abi_attestation.rs::unsigned_cdylib_reports_absent_attestation",
        ],
    },
    Guarantee {
        id: "abi_attestation.sign_requires_claim_coverage",
        kind: GuaranteeKind::AbiAttestation,
        class: GuaranteeClass::Static,
        phase: Phase::Codegen,
        description:
            "`corvid build --target=cdylib --sign` refuses to sign when \
             any contract declared by the source lacks a non-out-of-scope \
             guarantee id in the descriptor's signed claim set.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_accepts_registered_contracts",
        ],
        adversarial_test_refs: &[
            "crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_rejects_missing_declared_contract_id",
            "crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_rejects_out_of_scope_contract_id",
        ],
    },
    // ----- Jobs (Phase 38) ---------------------------------------
    // These rows are placeholders so `validate_signed_claim_coverage`
    // can recognise the contract surfaces named by the developer-flow
    // doc when their parser-level keywords land. Each row gets
    // promoted to `Static` or `RuntimeChecked` by the audit-correction
    // slice that wires the surface end-to-end (38K/38L/38M).
    Guarantee {
        id: "jobs.cron_schedule_durable",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A `schedule \"cron\" zone \"…\" -> job(args)` declaration \
             persists to the durable queue store and survives process \
             restart. Slice 35-N walks `Decl::Schedule` so a signed \
             cdylib that declares a cron schedule cannot ship without \
             this guarantee in its descriptor.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_walks_schedule_decl",
        ],
        adversarial_test_refs: &[
            "crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_rejects_schedule_without_jobs_coverage",
        ],
    },
    Guarantee {
        id: "jobs.retry_budget_bound",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::Runtime,
        description:
            "`@retry(max_attempts: N, backoff: ...)` bounds the runtime \
             retry envelope of a job so a transient failure cannot \
             escalate into unbounded re-spend.",
        out_of_scope_reason:
            "The runtime queue and lease envelopes are shipped and the \
             retry policy is configurable at enqueue time via the host \
             API + `corvid jobs limit`. `@retry` as a Corvid source-level \
             attribute is filed as a post-v1.0 ergonomic improvement \
             (35V2-P38-H), not a launch-blocker — the runtime behaviour \
             the attribute would surface is already shipped. Slice 38K \
             promoted the runtime; the syntactic promotion of this row \
             tracks with the post-v1.0 syntax slice.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "jobs.idempotency_key_uniqueness",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Across N concurrent workers, exactly one durable queue \
             row exists for a given non-null idempotency key. \
             Enforced by a partial UNIQUE INDEX on \
             `queue_jobs(idempotency_key) WHERE idempotency_key IS \
             NOT NULL` in the SQLite schema, plus the existing \
             `enqueue_typed_idempotent` collision-fallback path \
             that returns the surviving row when the insert hits \
             the UNIQUE constraint.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/queue/tests/durable_basics.rs::durable_queue_idempotency_key_collapses_duplicate_jobs",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/tests/durability_corpus.rs::t38l_d1_four_workers_collapse_to_one_row",
        ],
    },
    Guarantee {
        id: "jobs.lease_exclusivity",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A job lease prevents two workers from running the same \
             job concurrently. Slice 38K's `WorkerPool` over \
             `DurableQueueRuntime` runs N tokio tasks each \
             contesting `lease_next_at`; the SQLite UPDATE that \
             flips `pending` → `leased` is atomic, so exactly one \
             worker wins each contention round. Lease expiry plus \
             a fresh worker re-leasing is shipped (slice 38L's D3 \
             test); heartbeat extension for long-running steps \
             remains a follow-up.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/worker_pool.rs::t38k_pool_runs_each_job_exactly_once",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/worker_pool.rs::t38k_two_workers_cannot_both_lease_same_job",
            "crates/corvid-runtime/src/worker_pool.rs::t38k_pool_drains_gracefully_without_claiming_new_work",
        ],
    },
    Guarantee {
        id: "jobs.durable_resume",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A worker that drops uncleanly mid-step (the SIGKILL \
             surrogate the queue runtime is responsible for) leaves \
             behind durable checkpoint rows; a fresh worker that \
             opens the same SQLite file after the lease TTL elapses \
             can re-lease the job and resume from those checkpoints. \
             SQLite WAL fsync makes this property structural. The \
             count-bounded `no double LLM call` extension joins the \
             Phase 21 Replay corpus when step-skip semantics land at \
             the VM layer.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/queue/tests/checkpoints.rs::durable_queue_records_ordered_agent_checkpoints",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/tests/durability_corpus.rs::t38l_d3_checkpoints_survive_unclean_shutdown",
        ],
    },
    Guarantee {
        id: "jobs.cron_dst_correct",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Cron schedules expressed in `America/New_York` (and \
             other DST-observing timezones) produce monotonic UTC \
             fire times across the spring-forward and fall-back \
             transitions, with no duplicates and no fire at the \
             non-existent local instant. `chrono-tz` is wired into \
             the queue runtime; the cron-crate's `Schedule::after` \
             iterator is timezone-aware.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/tests/durability_corpus.rs::t38l_d2_dst_spring_forward_is_deterministic",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/tests/durability_corpus.rs::t38l_d2_dst_fall_back_is_monotonic",
        ],
    },
    Guarantee {
        id: "jobs.approval_wait_resume",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::Runtime,
        description:
            "An approval boundary inside a job pauses the run until \
             an approval token arrives, expires, or is denied; the \
             resume path writes the audit transition.",
        out_of_scope_reason:
            "Runtime approval-wait state ships and is reachable via \
             `corvid jobs wait-approval` + `corvid jobs approval \
             approve/deny` (the shipped surface). `await_approval` \
             as a Corvid source-level keyword is filed as a post-v1.0 \
             ergonomic improvement (35V2-P38-H), not a launch-blocker \
             — the runtime behaviour already ships, the syntax just \
             surfaces it more compactly.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "jobs.loop_bounds_enforced",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Agent loops driven by jobs honor max-steps, max-wall-time, \
             max-spend, and max-tool-calls; exceeding any bound moves \
             the job to `loop_budget_exceeded` and writes a \
             `loop_bound_exceeded` audit event listing the violated \
             bounds. Post-termination `record_loop_usage` calls are \
             refused so a stale worker cannot silently keep charging \
             spend / steps against a terminal job.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/queue/tests/loops.rs::durable_queue_enforces_loop_budget_limits_with_audit",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/queue/tests/loops.rs::durable_queue_refuses_loop_usage_after_budget_exceeded_termination",
        ],
    },
    Guarantee {
        id: "jobs.explain_sources_grounded",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid jobs explain <job_id>` renders a typed \
             operational summary whose `sources` array names every \
             audit-event id the explanation consulted — the \
             Grounded<T> shape at the JSON layer. Every transition \
             surfaced in the explanation has a back-reference in \
             `sources`, so an operator can audit-trail every claim \
             back to a queue row. A missing job id is refused with \
             an explicit diagnostic rather than served as an empty \
             report.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/jobs_explain_cmd.rs::jobs_explain_denied_approval_carries_grounded_sources",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/jobs_explain_cmd.rs::jobs_explain_unknown_job_refuses",
        ],
    },
    Guarantee {
        id: "jobs.replayable_side_effects",
        kind: GuaranteeKind::Jobs,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A job marked `@replayable` records a per-job JSONL trace \
             (slice C-2) at `<trace_dir>/<job_id>.jsonl`, and a later \
             `corvid jobs replay --source <path>.cor --job <job_id>` \
             (slice C-3) drives the same agent body against a runtime \
             in `RuntimeMode::Replay(source)` with every side-effect \
             surface quarantined — LLM adapters wrap into \
             `QuarantinedLlmAdapter` (slice C-4); HTTP / store writes / \
             file writes refuse with typed `QuarantineViolation` (slice \
             C-5). The recorded substitution path consumes recorded \
             `LlmCall` / `LlmResult` events for matched dispatch; any \
             call that bypasses substitution and reaches the manager \
             layer fails closed. The durable queue uses raw rusqlite \
             and trace emission uses JsonlTraceWriter, so neither \
             routes through the quarantined manager types.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_passes_through_store_reads",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_passes_through_io_reads",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::live_mode_does_not_quarantine_any_surface",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::differential_replay_does_not_quarantine_llm_registry",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_quarantines_llm_registry_direct_calls",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_quarantines_http_client_send",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_quarantines_store_writes",
            "crates/corvid-runtime/tests/replay_quarantine_corpus.rs::replay_quarantines_io_writes",
        ],
    },
    // ----- Auth (Phase 39) ---------------------------------------
    Guarantee {
        id: "auth.session_rotation_on_privilege_change",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A session id rotates on a named privilege-change \
             event (role upgrade, password change, MFA enrolment, \
             admin elevation) so a stolen pre-escalation cookie \
             cannot exercise the post-escalation privilege. The \
             rotation is recorded in the auth-audit trail with \
             the typed `PrivilegeChangeReason` as evidence; the \
             pre-elevation cookie is rejected from that point on. \
             Catches the `session-fixation` adversarial-corpus \
             threat.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/auth/sessions.rs::session_rotation_on_privilege_change_rejects_pre_elevation_session_fixation_attempt",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/auth/sessions.rs::session_rotation_on_privilege_change_refuses_empty_trace_id",
        ],
    },
    Guarantee {
        id: "auth.api_key_at_rest_hashed",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "API keys are stored only as Argon2id hashes; the \
             plaintext leaves Corvid memory exactly once at issuance \
             and is never logged. Verified by the existing \
             `hash_api_key_secret`/`verify_api_key_secret` path in \
             `corvid-runtime/src/auth.rs`.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/auth/api_keys.rs::api_key_runtime_resolves_service_actor_with_argon2_hash_and_redacted_audit",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/auth/api_keys.rs::api_key_runtime_rejects_wrong_tenant_revoked_expired_and_user_actors",
        ],
    },
    Guarantee {
        id: "auth.api_key_scope_subset_check",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "An API key's granted scope is a structured set of \
             `<resource>.<action>` permissions, not an opaque \
             hash. `enforce_scope_grant(granted, required)` \
             refuses the call when the required set is not a \
             subset of the granted set, and the typed error \
             names every missing permission so the audit trail \
             records exactly which scope was attempted. Catches \
             the `scope-escalation` adversarial-corpus threat: a \
             key issued with `{orders.read}` cannot satisfy a \
             required `{refunds.write}` action. Canonical \
             fingerprint over the sorted set is stable across \
             permission-insertion order so the value can be \
             persisted alongside `ApiKeyRecord::scope_fingerprint` \
             without re-computing the source set. Wiring the \
             enforcement into every route is downstream work; \
             this row commits the typed model + the predicate.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/auth/scope.rs::scope_with_subset_satisfies_required_grant",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/auth/scope.rs::scope_escalation_attempt_refused_with_specific_missing_permission",
            "crates/corvid-runtime/src/auth/scope.rs::scope_escalation_lists_every_missing_permission_not_just_the_first",
            "crates/corvid-runtime/src/auth/scope.rs::empty_granted_scope_refuses_any_non_empty_required",
        ],
    },
    Guarantee {
        id: "auth.jwt_kid_rotation",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "JWT verification fetches the JWKS, picks the key by \
             `kid`, verifies the signature with `jsonwebtoken`, and \
             rejects tokens whose `kid` is missing from the current \
             JWKS, whose alg does not match the contract, whose \
             signature fails to verify, whose exp/iss/aud do not \
             align with the contract, or whose required \
             subject/tenant claim is missing. Out-of-scope at \
             Phase 39 base; promoted to `RuntimeChecked` by slice \
             39K when `corvid-runtime/src/jwt_verify/` shipped.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::parse_alg_accepts_supported_and_refuses_others",
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::decoding_key_for_rsa_jwk_constructs",
            "crates/corvid-runtime/src/jwt_verify/mod.rs::error_slugs_are_stable_for_audit_log",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::kid_downgrade_returns_kid_not_found",
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::header_alg_must_match_contract_alg",
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::alg_none_in_header_is_refused",
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::malformed_token_is_refused_before_fetch",
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::jwks_fetch_failure_is_surfaced",
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::decoding_key_for_rejects_rsa_without_n",
            "crates/corvid-runtime/src/jwt_verify/verifier.rs::decoding_key_for_rejects_unknown_kty",
        ],
    },
    Guarantee {
        id: "contract.matches_compiled_surface",
        kind: GuaranteeKind::AbiDescriptor,
        class: GuaranteeClass::Static,
        phase: Phase::AbiEmit,
        description:
            "The emitted Application Contract (slice 51) describes \
             exactly the surface the compiler checked: only PUBLIC \
             agents/prompts/types appear, each callable's declared \
             inputs + return type + AI-native capabilities (streaming, \
             grounding, approvals, confidence, cost, latency, \
             pagination) come from its checked signature and composed \
             effect row, field refinements and `@ui`/`@status`/`@upload` \
             attributes carry through from the checked AST, and a \
             private declaration is never exposed. The OpenAPI 3.1 + \
             `corvid-ai.json` projections and every generated SDK read \
             this same contract, so no consumer can observe a surface \
             the compiler did not verify.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-abi/src/app_contract.rs::public_agent_capabilities_reflect_return_type_and_effects",
            "crates/corvid-abi/src/app_contract.rs::route_requires_policy_surfaces_and_binds_typed_actor",
        ],
        adversarial_test_refs: &[
            "crates/corvid-abi/src/app_contract.rs::private_agents_are_not_in_the_contract",
        ],
    },
    Guarantee {
        id: "contract.runtime_closure",
        kind: GuaranteeKind::Server,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Before `corvid serve` / `corvid dev` bind a listener, they \
             walk the public HTTP surface the Application Contract \
             advertises (slice 52b) and assert a runtime execution path \
             exists for every route. A route the contract describes but \
             the interpreter tier cannot yet execute is a startup error \
             (`E5204 Contract not executable`) that names the offending \
             element and the capability it needs — never a silent \
             runtime `501`: the developer's source is the forcing \
             function. The closure surface is driven by a \
             `RuntimeCapabilities` snapshot that each Phase 52 slice \
             flips as it lands the capability, so the running backend \
             can never advertise more than it delivers. As of slice \
             52f the interpreter tier is complete — it serves route \
             execution, `Stream<T>` responses (Server-Sent Events), \
             `Upload<Format>` bodies (multipart), `Page<Item>` responses \
             (cursor envelope), and authorization enforcement (a \
             `requires authenticated|role|permission` route resolves the \
             caller's session to a verified `actor` and enforces it \
             before the handler runs). A direct `Upload<Format>` route \
             must declare `@upload(max_bytes: N)` or \
             `@upload(max_mb: N)`; the compiler lowers that policy into \
             the Application Contract and OpenAPI, and the server streams \
             the file while enforcing that exact limit and accepted MIME \
             set. There is no runtime-only upload-size default. The \
             refuse-to-start mechanism \
             still guards any future capability and native tiers that \
             lack a capability refuse the routes that need it.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-driver/src/contract_closure.rs::reference_shape_has_no_closure_gaps",
            "crates/corvid-driver/src/contract_closure.rs::capability_present_closes_the_gap",
        ],
        adversarial_test_refs: &[
            "crates/corvid-driver/src/contract_closure.rs::stream_response_route_is_a_gap_when_streaming_is_off",
            "crates/corvid-driver/src/contract_closure.rs::upload_body_route_is_a_closure_gap",
            "crates/corvid-driver/src/contract_closure.rs::page_response_route_is_a_closure_gap",
            "crates/corvid-driver/src/contract_closure.rs::policy_route_without_auth_enforcement_is_a_closure_gap",
        ],
    },
    Guarantee {
        id: "auth.jwt_tamper_and_fuzz_resistant",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "The local mock identity provider \
             (`corvid-runtime/src/jwt_verify/mock_idp.rs`) mints \
             Ed25519-signed ID tokens the real `JwtVerifier` accepts, \
             and every source-bypass MUTATION it can produce is \
             refused: dropping the signature (`alg=none`), tampering \
             the signature bytes, forging the `kid`, swapping the \
             issuer or audience, and backdating `exp`. A deterministic \
             byte-fuzz (2000 malformed inputs) proves the JWT parser \
             never panics and never forges a valid result on \
             adversarial bytes — the safe-defaults cannot be bypassed \
             and the verifier degrades gracefully rather than \
             crashing.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/jwt_verify/mock_idp.rs::mock_idp_token_verifies_end_to_end",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/jwt_verify/mock_idp.rs::every_mutated_token_is_refused",
            "crates/corvid-runtime/src/jwt_verify/mock_idp.rs::byte_fuzz_never_panics_and_never_forges",
        ],
    },
    Guarantee {
        id: "auth.oauth_pkce_required",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "OAuth callback state requires PKCE for public clients; \
             the state record carries the code-verifier hash and is \
             single-use, tenant-scoped, and expiry-bound.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/auth/oauth.rs::oauth_callback_state_is_hashed_single_use_and_restart_safe",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/auth/oauth.rs::oauth_callback_rejects_expired_and_cross_tenant_state",
        ],
    },
    Guarantee {
        id: "auth.csrf_double_submit",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "CSRF protection on cookie-bearing state-changing \
             requests (POST / PUT / PATCH / DELETE) uses a \
             double-submit token of shape \
             `<binding>.<hex_hmac>` where `hex_hmac` is \
             HMAC-SHA256(server_secret, \"corvid-csrf-v1:\" || \
             binding). The verifier enforces three independent \
             checks: header and cookie both present, equal under \
             constant-time comparison (the double-submit \
             invariant — a cross-site request cannot read the \
             cookie), and the HMAC component verifies against \
             the server secret (so a forged token without the \
             secret is rejected). Safe methods (GET / HEAD / \
             OPTIONS) skip the check; unknown methods fail \
             closed. An empty server secret also fails closed \
             on state-changing requests. The rendered axum \
             server wires the verifier into its middleware when \
             `CORVID_CSRF_SECRET` is set; the canonical \
             implementation lives in \
             `corvid-runtime::auth::csrf` with 8 exhaustive \
             unit tests, and the rendered-server end-to-end \
             test asserts the wire behaviour matches.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/auth/csrf.rs::mint_and_verify_round_trip_on_each_state_changing_method",
            "crates/corvid-cli/tests/build_server.rs::rendered_server_csrf_middleware_refuses_state_change_without_double_submit_token",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/auth/csrf.rs::csrf_bypass_attempt_without_header_refused_on_put_patch_delete",
            "crates/corvid-runtime/src/auth/csrf.rs::csrf_token_forged_without_server_secret_refused_on_hmac",
            "crates/corvid-runtime/src/auth/csrf.rs::csrf_header_and_cookie_must_match_constant_time",
            "crates/corvid-runtime/src/auth/csrf.rs::csrf_empty_server_secret_fails_closed_on_state_changing_methods",
        ],
    },
    Guarantee {
        id: "tenant.cross_tenant_compile_error",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::TypeCheck,
        description:
            "A function whose actor came from tenant A may not pass \
             a record owned by tenant B to a tool that writes back \
             into A — the typechecker rejects the cross-tenant \
             reference.",
        out_of_scope_reason:
            "Tenant tagging exists in runtime envelopes + the CLI \
             (`corvid approvals` honours tenant scoping; the \
             approval_bypass_rejects_tenant_crossing_actor test \
             pins the runtime half). The parser-level `tenant Org \
             { ... }` block + the typechecker reachability that \
             would refuse a cross-tenant value at compile time \
             does not exist yet. Filed as post-v1.0 \
             `35V2-P39-I-post-v1.0-auth-syntax-sugar` — the \
             runtime behaviour ships today, the syntactic \
             promotion of this row tracks with the post-v1.0 \
             parser surface slice.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "approval.policy_clause_static_check",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::Static,
        phase: Phase::TypeCheck,
        description:
            "Every served route that can reach an `approve` boundary \
             declares a complete `@approval(...)` policy: reviewer \
             role, risk, data class, expiry, cost ceiling, and \
             reversibility. The reviewer role must exist and the \
             identity must grant `approvals.decide`; dead policies \
             are rejected.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-types/src/tests.rs::authenticated_approval_route_typechecks",
            "crates/corvid-cli/tests/serve_smoke.rs::approval_decisions_reject_every_unauthorized_path",
        ],
        adversarial_test_refs: &[
            "crates/corvid-types/src/tests.rs::approval_route_rejects_an_undeclared_reviewer_role",
            "crates/corvid-types/src/tests.rs::approval_route_requires_a_declared_decision_permission",
            "crates/corvid-types/src/tests.rs::ordinary_route_rejects_a_dead_approval_policy",
        ],
    },
    Guarantee {
        id: "approval.batch_equivalence_typed",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::TypeCheck,
        description:
            "An `approval ... batch_with: same_tool, same_data_class, \
             same_role` clause groups equivalent approvals so a \
             reviewer can approve one record and have N \
             equivalent-by-typed-shape records auto-resolve.",
        out_of_scope_reason:
            "The runtime half of the batch-equivalence guarantee \
             ships today as `approval.batch_refuses_cross_data_class_drift` \
             (RuntimeChecked): `corvid approvals batch` refuses to \
             span >1 data class unless `--require-data-class` pins \
             the batch. The typecheck-time `batch_with: same_tool, \
             same_data_class, same_role` source-level clause is \
             post-v1.0 ergonomic surface — filed as \
             `35V2-P39-I-post-v1.0-auth-syntax-sugar`. The runtime \
             check prevents the threat today; the source-level \
             sugar lets contracts declare the batch shape directly.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "approval.batch_refuses_cross_data_class_drift",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid approvals batch` refuses outright when the \
             supplied ids span >1 `data_class` unless the operator \
             pins the batch with `--require-data-class <CLASS>`. \
             Catches the `batch-approval-drift-across-data-classes` \
             threat where `financial` and `pii` records would \
             otherwise resolve in the same invocation under a \
             single reviewer's role check.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/approvals_cmd/interaction.rs::approvals_batch_require_data_class_pins_to_one_class",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/approvals_cmd/interaction.rs::approvals_batch_refuses_cross_data_class_drift_without_pin",
        ],
    },
    Guarantee {
        id: "approval.explain_sources_grounded",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid approvals explain <id>` renders a typed \
             reviewer summary whose `sources` array names every \
             audit-event id the explanation consulted — the \
             Grounded<T> shape at the JSON layer. Every transition \
             surfaced in the explanation has a back-reference in \
             `sources`, so a reviewer can audit-trail every claim \
             back to a queue row. Cross-tenant requests are \
             refused with an explicit diagnostic rather than \
             silently leaking state.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/approvals_cmd/explain.rs::approvals_explain_pending_carries_grounded_sources",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/approvals_cmd/explain.rs::approvals_explain_cross_tenant_refused",
        ],
    },
    Guarantee {
        id: "approval.confused_deputy_typecheck",
        kind: GuaranteeKind::Auth,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::TypeCheck,
        description:
            "A reachable path from any route or job to a `@dangerous` \
             tool must have an `approve` token whose `required_role` \
             covers every reachable caller — otherwise typecheck \
             rejects.",
        out_of_scope_reason:
            "Lexical-scope approve-presence check ships \
             (`approval.dangerous_call_requires_token` + \
             `approval.token_lexical_only`). The role-coverage \
             extension needs a typechecker pass that walks the \
             call graph from every route / job entry point AND \
             a source-level role-declaration syntax for the pass \
             to reason over. Recon under slice \
             `35V2-P39-J-LR-role-coverage-reachability` found the \
             role syntax does not exist in the AST today \
             (`AgentAttribute` variants today are `Replayable`, \
             `Deterministic`, `Wrapping`, `GroundedPure` — no \
             `@requires(role)` or `@approval(role)` variant). \
             The required source-level surface is filed at \
             post-v1.0 `35V2-P39-I-post-v1.0-auth-syntax-sugar` \
             (the `auth` / `tenant` / `role` / `permission` / \
             `approval Name:` / `@requires` / `@approval` \
             keyword set). The role-coverage typechecker pass \
             therefore moves to post-v1.0 too, blocked on the \
             same syntax dependency. The runtime half of the \
             confused-deputy threat — approve-presence check + \
             role-fingerprint match at request time — already \
             ships (test: \
             `approval_bypass_rejects_confused_deputy_self_approval` \
             in `crates/corvid-runtime/src/approval_queue.rs`).",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    // ----- Connector (Phase 41) ----------------------------------
    Guarantee {
        id: "connector.scope_minimum_enforced",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A connector cannot use a scope its manifest does not \
             declare and an actor cannot use a scope its auth state \
             does not authorise. The runtime fires \
             `ConnectorAuthError::MissingScope` (or `UnknownScope`) \
             before any HTTP layer touches the network, so a leaked \
             low-scope token cannot escalate to a higher-scope \
             operation by guessing the scope id.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-connector-runtime/src/runtime.rs::mock_mode_checks_auth_rate_limit_and_emits_trace",
        ],
        adversarial_test_refs: &[
            "crates/corvid-connector-runtime/tests/threat_corpus.rs::t1_github_rejects_unauthorised_scope",
            "crates/corvid-connector-runtime/tests/threat_corpus.rs::t1_gmail_rejects_unauthorised_scope",
            "crates/corvid-connector-runtime/tests/threat_corpus.rs::t1_slack_rejects_unauthorised_scope",
        ],
    },
    Guarantee {
        id: "connector.per_user_token_separate_from_session",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A connector call is authorized only by a `ConnectorAccess` \
             credential; a `LoginSession` identity token (from the \
             `identity` block) is refused at the connector boundary \
             with `ConnectorAuthError::NotAConnectorCredential`. The \
             login session and the connector workspace/per-user access \
             token are distinct credentials that never interchange, so \
             a stolen or replayed login cookie cannot act as a \
             connector token. A `per_user` connector additionally \
             requires the end-user actor to authorize \
             (`PerUserRequiresEndUser`), keeping each user's connector \
             grant scoped to that user.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-connector-runtime/src/auth.rs::per_user_connector_requires_an_end_user_actor",
        ],
        adversarial_test_refs: &[
            "crates/corvid-connector-runtime/src/auth.rs::login_session_credential_cannot_authorize_a_connector",
        ],
    },
    Guarantee {
        id: "connector.write_requires_approval",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::TypeCheck,
        description:
            "A connector method whose effect set names a write \
             (`gmail.send`, `slack.post`, `github.create_issue`) \
             reaches typecheck only when its caller has a matching \
             `approve` boundary in lexical scope.",
        out_of_scope_reason:
            "Manifest declares write effects (`*.write`, `send_*`) \
             in `shipped_manifests` and the runtime refuses unsafe \
             effects via `ConnectorRuntimeError::ReplayWriteQuarantined` \
             when not authorized. The source-level `connector ... \
             uses dangerous` declaration that would let typecheck \
             refuse a call without a lexical-scope `approve` does \
             not exist yet — connectors are configured by Rust data, \
             not Corvid source. Filed as post-v1.0 \
             `35V2-P41-I-post-v1.0-connector-syntax-sugar` — the \
             runtime behaviour (manifest enforcement at write time) \
             ships today; the typecheck-time form is the syntax \
             sugar that promotes this row to Static.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "connector.rate_limit_respects_provider",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A connector honors the provider's rate-limit advice \
             (`Retry-After`, 429, 5xx). The shared `ReqwestRealClient` \
             parses RFC 7231 `Retry-After` integer-seconds into \
             milliseconds via `parse_retry_after_header` and surfaces \
             it as `ConnectorRuntimeError::RateLimited { retry_after_ms }`, \
             which the runtime forwards verbatim to the caller \
             instead of retrying behind their back.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-connector-runtime/src/real_client.rs::parse_retry_after_seconds_form",
        ],
        adversarial_test_refs: &[
            "crates/corvid-connector-runtime/src/real_client.rs::parse_retry_after_returns_none_for_malformed",
            "crates/corvid-connector-runtime/src/runtime.rs::real_mode_propagates_rate_limited_from_bound_client",
            "crates/corvid-connector-runtime/tests/threat_corpus.rs::t5_rate_limited_propagates_retry_after_ms",
        ],
    },
    Guarantee {
        id: "connector.contract_drift_detected",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid connectors check --baseline <file> --observed \
             <file>` runs a schema-agnostic structural drift \
             detector over two JSON payloads and exits non-zero \
             when any field is added, removed, or type-changed \
             between the baseline and the observed response. The \
             detector reports each drift site as a sorted JSON \
             path so the output is deterministic and \
             diff-friendly in CI. The canonical detector ships \
             in `corvid_connector_runtime::contract_drift` \
             (9 unit tests, schema-agnostic so adopting it does \
             not require a manifest schema change). The CLI \
             wires it via the file-input flow — capture the \
             provider response separately in CI and pipe it \
             through the command. The live-HTTP fetch path that \
             would compute `observed` from a real provider call \
             stays operational scope at \
             `35V2-P41-E-LR-live-provider-ci-matrix` (provider \
             credentials live in CI secrets, not local config).",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-connector-runtime/src/contract_drift.rs::identical_shapes_produce_empty_drift_report",
            "crates/corvid-cli/src/connectors_cmd/check.rs::contract_drift_identical_files_report_no_drift",
        ],
        adversarial_test_refs: &[
            "crates/corvid-connector-runtime/src/contract_drift.rs::provider_removed_field_appears_in_removed_paths_central_threat",
            "crates/corvid-connector-runtime/src/contract_drift.rs::provider_type_change_appears_in_type_changed_paths",
            "crates/corvid-cli/src/connectors_cmd/check.rs::contract_drift_removed_field_surfaces_with_non_empty_report",
            "crates/corvid-cli/src/connectors_cmd/check.rs::contract_drift_malformed_baseline_file_surfaces_typed_error",
        ],
    },
    Guarantee {
        id: "connector.drift_narration_grounded",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid connectors check --baseline <file> \
             --observed <file> --narrate` pairs every site in \
             the structural drift report with a typed \
             `DriftNarration` carrying a one-line consequence \
             (e.g. \"connector code that consumed this field is \
             now broken at deserialization\"), a typed severity \
             (`breaking` for removed/type-changed sites, \
             `compatible` for added sites), and a Grounded<T> \
             `sources` array that back-references the detector \
             bucket + path the narration summarised. The order \
             is breaking-first so an operator triaging CI \
             output reads the most consequential items first. \
             Deterministic + LLM-free; the slice's \
             \"RAG-grounded\" framing refers to the \
             evidence-citation property, not to a live LLM \
             round-trip. The first sub-slice of \
             `35V2-P41-H-LR-connectors-ai-helpers` to ship; \
             `mock-fixture-gen` (generative) and `fail-sim` \
             (adversarial) need LLM work and remain filed under \
             the umbrella.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-connector-runtime/src/contract_drift.rs::every_drift_narration_carries_grounded_sources",
            "crates/corvid-cli/src/connectors_cmd/check.rs::contract_drift_narration_flow_pairs_every_site_with_grounded_sources",
        ],
        adversarial_test_refs: &[
            "crates/corvid-connector-runtime/src/contract_drift.rs::drift_narration_classifies_breaking_versus_compatible",
            "crates/corvid-connector-runtime/src/contract_drift.rs::removed_field_narration_names_deserialization_consequence",
            "crates/corvid-connector-runtime/src/contract_drift.rs::drift_narration_orders_breaking_before_compatible",
        ],
    },
    Guarantee {
        id: "connector.webhook_signature_verified",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Inbound webhook payloads from Slack, GitHub, and Linear \
             are HMAC-SHA256 verified against the manifest's shared \
             secret before any handler runs. Per-provider schemes are \
             honored: GitHub uses `X-Hub-Signature-256: sha256=<hex>`, \
             Slack uses `v0:<ts>:<body>` with a 5-minute replay \
             window, and Linear uses a bare hex digest. Comparison is \
             constant-time; a malformed header, mismatched digest, or \
             stale Slack timestamp returns a categorical \
             `WebhookVerificationOutcome` that the dispatcher must \
             reject before any side effect.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-connector-runtime/src/webhook_verify.rs::github_verifies_correct_signature",
            "crates/corvid-connector-runtime/src/webhook_verify.rs::slack_verifies_correct_signature_inside_window",
            "crates/corvid-connector-runtime/src/webhook_verify.rs::linear_verifies_correct_signature",
        ],
        adversarial_test_refs: &[
            "crates/corvid-connector-runtime/tests/threat_corpus.rs::t7_github_webhook_forgery_rejected",
            "crates/corvid-connector-runtime/tests/threat_corpus.rs::t7_slack_webhook_replay_outside_window_rejected",
            "crates/corvid-connector-runtime/tests/threat_corpus.rs::t7_linear_webhook_wrong_secret_rejected",
        ],
    },
    Guarantee {
        id: "connector.replay_quarantine",
        kind: GuaranteeKind::Connector,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "A connector running in replay mode must not perform \
             provider writes. The runtime returns \
             `ConnectorRuntimeError::ReplayWriteQuarantined` for any \
             scope whose effects include a `*.write` or `send_*` \
             effect when the active mode is `Replay`, regardless of \
             whether a real client is bound. Read-shaped operations \
             still complete from the recorded cassette so deterministic \
             replay continues to work.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-connector-runtime/src/test_kit.rs::fixture_runs_mock_and_replay_read_paths",
        ],
        adversarial_test_refs: &[
            "crates/corvid-connector-runtime/src/runtime.rs::replay_mode_quarantines_writes",
            "crates/corvid-connector-runtime/src/test_kit.rs::fixture_proves_replay_write_quarantine",
            "crates/corvid-connector-runtime/src/calendar.rs::calendar_replay_quarantines_writes",
            "crates/corvid-connector-runtime/src/slack.rs::slack_replay_quarantines_writes",
        ],
    },
    // ----- Observability (Phase 40) ------------------------------
    Guarantee {
        id: "observability.otel_conformance",
        kind: GuaranteeKind::Observability,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Lineage events flow through the standard \
             `opentelemetry` + `opentelemetry-otlp` SDK and emit \
             OTLP/HTTP spans whose attributes carry \
             `corvid.guarantee_id`, `corvid.cost_usd`, \
             `corvid.approval_id`, `corvid.replay_key`. The \
             attribute set is constructed by \
             `corvid_runtime::otel_sdk_export::corvid_span_attributes` \
             and the live wire path is exercised by the \
             docker-compose Jaeger harness in \
             `docs/operations/observability-conformance.md`.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/otel_sdk_export.rs::span_attributes_include_corvid_named_keys",
            "crates/corvid-runtime/src/otel_sdk_export.rs::span_name_uses_corvid_prefix_with_kind",
            "crates/corvid-runtime/src/otel_sdk_export.rs::span_kind_maps_lineage_to_otel",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/otel_sdk_export.rs::span_attributes_omit_missing_optional_keys",
            "crates/corvid-runtime/src/otel_sdk_export.rs::sdk_exporter_reaches_in_process_otlp_receiver",
        ],
    },
    Guarantee {
        id: "observability.lineage_completeness",
        kind: GuaranteeKind::Observability,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Every lineage event carries a (trace_id, span_id) \
             pair plus parent linkage when a parent exists, so a \
             SQL JOIN against the local trace store reconstructs \
             the route → job → agent → prompt → tool → approval \
             → DB tree. Validated on every event via \
             `corvid_runtime::lineage::validate_lineage`.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/lineage.rs::lineage_ids_are_stable_and_parented_across_backend_kinds",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/lineage.rs::lineage_validation_fails_closed_for_missing_parent_or_duplicate_root",
        ],
    },
    Guarantee {
        id: "observability.redaction_determinism",
        kind: GuaranteeKind::Observability,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "Redacting the same lineage event twice with the same \
             `LineageRedactionPolicy` yields byte-identical \
             output; trace topology (trace_id, span_id, parent \
             linkage) is preserved across redaction so observe / \
             eval / OTel keep correlating after sensitive values \
             are removed.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/lineage_redact.rs::redaction_preserves_topology_and_redacts_identifiers_deterministically",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/lineage_redact.rs::redaction_removes_obvious_secrets_from_serialized_lineage",
        ],
    },
    Guarantee {
        id: "observability.contract_aware_grouping",
        kind: GuaranteeKind::Observability,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid observe show` groups incidents by \
             guarantee_id, effect, budget, provenance, and \
             approval rule rather than by service.name — so an \
             analyst's first pivot lands on the contract that \
             broke. Implemented by \
             `lineage_incidents::group_lineage_incidents`.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/lineage_incidents.rs::incidents_group_by_guarantee_effect_budget_provenance_and_approval",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/lineage_incidents.rs::non_incident_ok_events_are_not_grouped",
        ],
    },
    Guarantee {
        id: "eval.drift_attribution",
        kind: GuaranteeKind::Observability,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid eval-drift --explain` decomposes the drift \
             between two trace runs into the four named \
             dimensions (model_id, prompt_hash, \
             retrieval_index_hash, input_fingerprint) plus a \
             residual percentage for unattributable changes. The \
             output's `sources` array carries the trace_id + \
             span_id of every event the analysis consulted.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/observe_helpers_cmd/eval_drift.rs::drift_explain_attributes_model_swap",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/observe_helpers_cmd/eval_drift.rs::drift_explain_surfaces_residual_when_status_flips_alone",
        ],
    },
    Guarantee {
        id: "eval.promotion_signed_lineage",
        kind: GuaranteeKind::Observability,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid eval-from-feedback` synthesises a typed \
             eval fixture from a 'wrong answer' feedback record, \
             redacting the matching lineage trace via the \
             production redaction policy before writing the \
             fixture. The fixture's `sources` field lists every \
             redacted event so downstream consumers can \
             reconstruct evidence without seeing raw identifiers.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/observe_helpers_cmd/eval_from_feedback.rs::eval_generate_from_feedback_writes_redacted_fixture",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/observe_helpers_cmd/eval_from_feedback.rs::eval_generate_from_feedback_missing_trace_id_refused",
        ],
    },
    Guarantee {
        id: "review_queue.cost_of_being_wrong_ranking",
        kind: GuaranteeKind::Observability,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid review-queue list --rank=cost-of-being-wrong` \
             surfaces low-confidence + high-risk outputs ranked \
             by the `cost_of_being_wrong` policy.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/review_queue_cmd.rs::rank_cost_of_being_wrong_sorts_highest_first",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/review_queue_cmd.rs::rank_unknown_policy_refused",
        ],
    },
    // ----- Deploy / Release / Upgrade / Ops / Claim (Phase 43) ----
    Guarantee {
        id: "deploy.reproducible_build",
        kind: GuaranteeKind::Deploy,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "Building the same `corvid deploy package` input twice on \
             the same host with two different `CARGO_TARGET_DIR` values \
             produces bit-identical signed artifacts (binary + SBOM + \
             DSSE attestation envelope). A second build that differs is \
             a build-environment leak — embedded timestamps, hostnames, \
             paths baked at compile time — and the verification CI \
             rejects it. The original determinism gap that kept this \
             row OutOfScope until 2026-05-30 was a `cargo:rustc-env=\
             CORVID_STATICLIB_DIR=<absolute target dir>` emission from \
             `crates/corvid-codegen-cl/build.rs` that `link.rs` and \
             `cdylib.rs` read through `env!()`, baking the host's \
             `target-build-1` / `target-build-2` path into the corvid \
             binary's read-only data section. Closed by routing the \
             staticlib lookup through \
             `crate::staticlib_discovery::discover_staticlib` at \
             runtime (`current_exe()`-relative resolution with an \
             explicit `CORVID_STATICLIB_DIR` override env var); the \
             build script no longer emits any host-dependent strings. \
             The production-grade oracle remains \
             `.github/workflows/reproducible-build.yml` running on \
             every push to `main`; structural test refs below lock \
             the regression in place locally.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-codegen-cl/tests/reproducibility.rs::build_script_emits_no_corvid_staticlib_dir_env_var",
            "crates/corvid-codegen-cl/tests/reproducibility.rs::link_and_cdylib_do_not_read_corvid_staticlib_dir_via_env_macro",
            "crates/corvid-codegen-cl/tests/reproducibility.rs::staticlib_discovery_module_is_wired_into_consumers",
        ],
        adversarial_test_refs: &[
            "crates/corvid-codegen-cl/tests/reproducibility.rs::reproducible_build_workflow_file_exists_and_diffs_two_target_dirs",
            "crates/corvid-codegen-cl/src/staticlib_discovery.rs::override_env_var_pointing_at_missing_file_falls_through",
            "crates/corvid-codegen-cl/src/staticlib_discovery.rs::resolution_strategy_descriptions_are_stable",
        ],
    },
    Guarantee {
        id: "deploy.attestation_chain",
        kind: GuaranteeKind::Deploy,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "`corvid deploy package --cdylib <path>` binds the deploy \
             attestation to the SHA-256 of the cdylib's bytes; the \
             cdylib itself carries its `corvid claim --explain` \
             embedded attestation, so the chain `claim --explain → \
             cdylib bytes → deploy attestation` cannot drift without \
             changing one of the digests. The attestation payload \
             carries `chain_status: \"complete\"` + `cdylib_sha256: \
             <hex>` when `--cdylib` is provided; `chain_status: \
             \"incomplete\"` + `cdylib_sha256: null` when omitted so \
             downstream verification can refuse an unchained \
             deploy.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/deploy_cmd.rs::deploy_attestation_binds_to_cdylib_digest_when_provided",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/deploy_cmd.rs::deploy_attestation_marks_chain_incomplete_without_cdylib",
        ],
    },
    Guarantee {
        id: "deploy.sbom_completeness",
        kind: GuaranteeKind::Deploy,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "`corvid deploy package` emits an SPDX 2.3 JSON SBOM \
             (`sbom.spdx.json`) naming the app's Corvid source (by \
             SHA-256) and the Corvid runtime the image links \
             against, with the relationship between them declared. \
             A future slice expands this to enumerate every \
             transitively-linked Rust dependency via `cargo metadata` \
             — the full-dep-enumeration completeness check tracks \
             separately at the dep-enumeration registry row that \
             lands when 43V wires `cargo metadata` into the SBOM.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/deploy_cmd.rs::deploy_sbom_is_structurally_valid_spdx_2_3",
            "crates/corvid-cli/src/deploy_cmd.rs::deploy_sbom_names_app_source_and_corvid_runtime",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/deploy_cmd.rs::deploy_sbom_names_app_source_and_corvid_runtime",
        ],
    },
    Guarantee {
        id: "release.signed_artifact",
        kind: GuaranteeKind::Release,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "Every artifact emitted by `corvid release nightly/beta/\
             stable` is signed with the release key + paired with a \
             `SHA256SUMS.txt` whose contents the user can verify \
             with `sha256sum -c`. The signed manifest is a DSSE \
             envelope over the release contents, with payload type \
             `application/vnd.corvid.release.manifest.v1+json`. The \
             channel + version pair must satisfy the channel's \
             naming convention (`-nightly.` / `-beta.` / plain \
             MAJOR.MINOR.PATCH) — a stable-shaped version cannot \
             be published to the nightly channel and vice versa.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/release_cmd.rs::release_validate_version_accepts_each_channel_shape",
            "crates/corvid-cli/src/release_cmd.rs::sign_release_manifest_emits_v1_payload_type",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/release_cmd.rs::release_validate_version_refuses_channel_version_mismatch",
        ],
    },
    Guarantee {
        id: "release.notes_grounded",
        kind: GuaranteeKind::Release,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "`corvid release notes <from> <to>` walks \
             `git log <from>..<to>` over the current repository, \
             categorises each non-merge commit by conventional-\
             commit prefix (feat / fix / perf / refactor / docs / \
             test / chore / build / ci / style), and emits \
             markdown grouped by category. Every rendered line \
             ends with the short SHA so every claim in the notes \
             back-references commit history — the Grounded<T> \
             property at the release-notes layer. Empty ranges \
             produce a typed \"No changes between X and Y\" stub \
             rather than a partially-rendered section header \
             with no entries. The slice's \"RAG-grounded\" \
             framing in the 43T umbrella refers to this \
             commit-history citation property, not to a live LLM \
             round-trip: the renderer is deterministic, the \
             generative half of the audit's helper set stays \
             filed under `35V2-P43-T-LR-phase-43-ai-helpers` for \
             the other four sub-helpers that need LLM \
             prompt-grounding.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/release_cmd.rs::release_notes_categorise_commits_routes_each_prefix",
            "crates/corvid-cli/src/release_cmd.rs::release_notes_markdown_renders_sections_with_grounded_shas",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/release_cmd.rs::release_notes_unrecognised_prefix_falls_through_to_other",
            "crates/corvid-cli/src/release_cmd.rs::release_notes_empty_range_renders_no_changes_stub",
            "crates/corvid-cli/src/release_cmd.rs::release_notes_ref_validation_refuses_empty_or_flag_shapes",
            "crates/corvid-cli/src/release_cmd.rs::release_notes_parse_git_log_output_drops_malformed_lines",
        ],
    },
    Guarantee {
        id: "upgrade.claim_regression_check",
        kind: GuaranteeKind::Upgrade,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "`corvid upgrade check --claims-current <path> \
             --claims-target <path>` compares two claim manifests \
             and refuses (exit 1) if the upgrade target removes any \
             registered guarantee id OR downgrades any class \
             (Static → RuntimeChecked / OutOfScope, RuntimeChecked → \
             OutOfScope). Upgrades (OutOfScope → RuntimeChecked, \
             etc.) are NOT regressions. The two manifests are JSON \
             arrays of `{id, class}` rows the operator produces via \
             `corvid claim --explain --json <cdylib>` against the \
             current and target binaries. The `--json` mode of \
             `claim --explain` itself lands as a sibling launch- \
             readiness slice — the comparison + rejection is what \
             this row promises.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/upgrade_cmd.rs::claim_regression_check_passes_when_manifests_match",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/upgrade_cmd.rs::claim_regression_check_flags_removed_guarantee",
            "crates/corvid-cli/src/upgrade_cmd.rs::claim_regression_check_flags_class_downgrades_only",
            "crates/corvid-cli/src/upgrade_cmd.rs::upgrade_check_refuses_unpaired_claim_manifest_flag",
        ],
    },
    Guarantee {
        id: "ops.live_introspection_signed",
        kind: GuaranteeKind::Ops,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "The Phase 36-generated axum server exposes a \
             `/__ops` endpoint returning a signed DSSE envelope \
             over a typed `OpsShowSnapshot` (build_id, \
             started_unix_ms, generated_unix_ms, request_count, \
             claim_manifest_ids). The envelope is signed with \
             the ed25519 key supplied via `CORVID_OPS_SIGNING_KEY` \
             — empty/unset key returns 503 (fail-closed; an \
             unsigned snapshot is what a MITM would produce). \
             The `corvid ops show --envelope-file <path> --pubkey \
             <path>` CLI verifies the envelope against an \
             operator-supplied public key: a signature mismatch \
             (wrong key, MITM), payload tampering, or wrong \
             payload-type (`corvid.ops.show.v1` is pinned so a \
             signature valid over an ABI attestation cannot be \
             replayed against the ops surface) all fail closed \
             with typed errors. The canonical implementation \
             lives in `corvid_runtime::ops_show` with 5 unit \
             tests; the rendered server inlines the producer \
             side (matching DSSE PAE byte-for-byte) and the CLI \
             reads + verifies via the runtime helper.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-runtime/src/ops_show.rs::ops_snapshot_round_trips_through_sign_then_verify",
            "crates/corvid-cli/src/ops_cmd.rs::ops_show_verifies_envelope_signed_with_matching_key",
            "crates/corvid-cli/tests/build_server.rs::rendered_server_ops_show_signs_snapshot_and_cli_verifies_it",
        ],
        adversarial_test_refs: &[
            "crates/corvid-runtime/src/ops_show.rs::ops_snapshot_signed_with_wrong_key_fails_verification",
            "crates/corvid-runtime/src/ops_show.rs::ops_snapshot_tampered_payload_fails_verification",
            "crates/corvid-runtime/src/ops_show.rs::ops_snapshot_refuses_envelope_with_wrong_payload_type",
            "crates/corvid-cli/src/ops_cmd.rs::ops_show_refuses_envelope_signed_with_wrong_key",
            "crates/corvid-cli/src/ops_cmd.rs::ops_show_refuses_malformed_envelope_file",
        ],
    },
    Guarantee {
        id: "claim.audit_runnable_artifacts",
        kind: GuaranteeKind::Claim,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "Every claim listed in `docs/meta/launch-claim-audit.md` \
             points at either a runnable command (backticked code), \
             a linked artifact (`[link]`-style markdown), or an \
             explicit `blocked` / `non-scope` status. `corvid claim \
             audit` exits 0 only when every claim has evidence; \
             aspirational wording flagged at audit time fails the \
             check unless the row carries an explicit \
             blocked/non-scope status.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/claim_cmd.rs::audit_passes_when_every_claim_resolves",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/claim_cmd.rs::audit_fails_when_a_claim_lacks_evidence",
        ],
    },
    Guarantee {
        id: "claim.audit_explain_failures_grounded",
        kind: GuaranteeKind::Claim,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Platform,
        description:
            "`corvid claim audit --explain-failures` pairs every \
             finding with a typed `ClaimFindingKind` \
             (`missing_evidence` or `aspirational_wording`) + a \
             `suggested_fix` string that back-references the \
             inventory line (Grounded<T> shape: every \
             remediation cites the row it addresses). Without \
             the flag, the `kind` + `suggested_fix` fields are \
             absent from the JSON so the pre-existing \
             `{line, claim, reason}` shape stays \
             backward-compatible for CI scripts that read the \
             legacy output. The narration layer never \
             synthesises explanations for rows that aren't \
             flagged. Same deterministic-narrator pattern as \
             `corvid connectors check --narrate` and \
             `corvid release notes`; the audit's \
             \"adversarial — narrates each failed claim with \
             the specific evidence path + suggested fix\" \
             framing in the 43T umbrella refers to this typed-\
             remediation property, not to a live LLM round-trip.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-cli/src/claim_cmd.rs::explain_failures_classifies_missing_evidence_with_line_grounded_fix",
            "crates/corvid-cli/src/claim_cmd.rs::explain_failures_classifies_aspirational_wording_with_typed_remediation",
        ],
        adversarial_test_refs: &[
            "crates/corvid-cli/src/claim_cmd.rs::explain_failures_off_preserves_legacy_finding_shape",
            "crates/corvid-cli/src/claim_cmd.rs::explain_failures_on_clean_inventory_yields_zero_findings",
        ],
    },
    // ----- Platform: explicit non-defenses ------------------------
    Guarantee {
        id: "platform.host_kernel_compromise",
        kind: GuaranteeKind::Platform,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::Platform,
        description:
            "Defending against a compromised host kernel or \
             privileged-process tampering with the running Corvid \
             binary's memory.",
        out_of_scope_reason:
            "Outside Corvid's trust boundary — a kernel that can rewrite \
             user-space memory can defeat any user-space invariant. The \
             security model assumes a non-malicious kernel; otherwise \
             the host is responsible.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "platform.signing_key_compromise",
        kind: GuaranteeKind::Platform,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::Platform,
        description:
            "Defending against compromise of the ed25519 signing key used \
             to attest a cdylib or sign a receipt.",
        out_of_scope_reason:
            "Key management is a host responsibility. Corvid signs and \
             verifies; rotating, revoking, and protecting keys is \
             outside the language's scope and explicitly delegated to \
             the host's key-management practice.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "platform.toolchain_compromise",
        kind: GuaranteeKind::Platform,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::Platform,
        description:
            "Defending against a compromised Rust toolchain, Cranelift \
             release, or system linker producing a Corvid binary that \
             does not match its source.",
        out_of_scope_reason:
            "Reproducible builds across heterogeneous toolchains are a \
             post-v1.0 hardening goal. Today Corvid trusts the rustc and \
             Cranelift releases the user installs; the bilateral verifier \
             (Slice 35-H) is the closest approximation of \
             toolchain-independence available pre-v1.0.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
    Guarantee {
        id: "app.pr_describe_grounded",
        kind: GuaranteeKind::App,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid app pr-describe --base <base.cor> \
             --head <head.cor>` lowers both sources to ABI \
             descriptors in-process and renders a typed \
             `PrDescription` summarising what the change set \
             means for the app's claim surface. Emits typed \
             sections (`Breaking`, `Additive`, `Informational`) \
             over agents, tools, approval gates, types, stores, \
             claim guarantees, and ABI / compiler versions. \
             Every bullet carries a non-empty `sources` array \
             back-referencing the descriptor field that \
             diverged. Sections are sorted by severity \
             (Breaking → Additive → Informational) then \
             alphabetically by heading so the reviewer reads \
             the most consequential changes first. The walker \
             catches the subtle cases the helper exists to \
             surface: removed agents/tools/approvals are \
             flagged Breaking; `pub extern \"c\"` revoked or \
             approval-tier weakened (operator → autonomous, \
             human_required → anything else) is Breaking; field \
             count drops on a same-name type is Breaking. \
             Replay-stable: two invocations on the same \
             `(base, head)` pair produce byte-identical \
             output. Deterministic + LLM-free; the helper's \
             \"generative\" framing describes its purpose \
             (generating PR-description text) not an LLM \
             round-trip. The third sub-slice of \
             `35V2-P42-H-LR-per-app-ai-helpers` to ship; the \
             umbrella closes with it.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-abi/src/pr_describe.rs::pr_describe_emits_bullets_grounded_to_descriptor_fields",
            "crates/corvid-cli/src/app_cmd.rs::pr_describe_renders_added_agent_in_additive_section_with_grounded_sources",
        ],
        adversarial_test_refs: &[
            "crates/corvid-abi/src/pr_describe.rs::no_change_case_produces_typed_grounded_description",
            "crates/corvid-abi/src/pr_describe.rs::breaking_section_precedes_additive_in_rendered_output",
            "crates/corvid-abi/src/pr_describe.rs::approval_tier_weakening_is_flagged_breaking",
            "crates/corvid-abi/src/pr_describe.rs::render_pr_description_is_byte_identical_across_two_invocations",
            "crates/corvid-cli/src/app_cmd.rs::pr_describe_with_unparseable_base_returns_typed_error_not_panic",
        ],
    },
    Guarantee {
        id: "app.adversarial_refresh_grounded",
        kind: GuaranteeKind::App,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid app adversarial-refresh <source.cor>` \
             walks every surface element in the app's ABI \
             descriptor and emits one typed \
             `AdversarialSuggestion` per (surface_element, \
             threat_category) pair. Threat categories include \
             `CrossTenant`, `MissingBudget`, `ApprovalBypass`, \
             `UnauthorisedCaller`, `ReplayWithoutToken`, \
             `WriteWithoutApproval`, `RoleBypass`, \
             `ExpiredApprovalReuse`, `DataClassDrift`, and \
             `MalformedPayload`. Per-surface coverage: every \
             approval site gets cross-tenant + role-bypass + \
             expired-approval-reuse suggestions, plus \
             data-class-drift when dangerous_targets is \
             non-empty; every `dangerous: true` tool gets \
             cross-tenant + approval-bypass + missing-budget; \
             every `pub extern \"c\"` agent gets \
             malformed-payload + unauthorised-caller, plus \
             replay-without-token when `@replayable`; every \
             writeable store gets cross-tenant-write + \
             write-without-approval. Each suggestion carries a \
             non-empty `sources` array back-referencing the \
             descriptor field it was derived from. Suggestions \
             are sorted deterministically (kind → name → \
             threat) so two runs on the same descriptor produce \
             byte-identical reports. Deterministic + LLM-free; \
             the helper's purpose is to make every surface \
             element's adversarial coverage requirements visible \
             so no surface ships without its named adversarial \
             counterpart. The second sub-slice of \
             `35V2-P42-H-LR-per-app-ai-helpers` to ship.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-abi/src/adversarial_refresh.rs::every_suggestion_carries_non_empty_sources",
            "crates/corvid-cli/src/app_cmd.rs::adversarial_refresh_for_extern_agent_renders_grounded_suggestions",
        ],
        adversarial_test_refs: &[
            "crates/corvid-abi/src/adversarial_refresh.rs::empty_surface_descriptor_produces_empty_report_not_sourceless",
            "crates/corvid-abi/src/adversarial_refresh.rs::render_adversarial_refresh_is_byte_identical_across_two_invocations",
            "crates/corvid-abi/src/adversarial_refresh.rs::non_dangerous_tools_get_no_suggestions",
            "crates/corvid-abi/src/adversarial_refresh.rs::read_only_stores_get_no_write_suggestions",
            "crates/corvid-abi/src/adversarial_refresh.rs::replayable_agents_get_replay_without_token_suggestion_non_replayable_do_not",
            "crates/corvid-cli/src/app_cmd.rs::adversarial_refresh_for_unparseable_source_returns_typed_error_not_panic",
        ],
    },
    Guarantee {
        id: "app.boot_summary_grounded",
        kind: GuaranteeKind::App,
        class: GuaranteeClass::RuntimeChecked,
        phase: Phase::Runtime,
        description:
            "`corvid app boot-summary <source.cor>` lowers the \
             supplied Corvid source through the standard frontend \
             pipeline, builds the ABI descriptor in-process, and \
             renders a typed `BootSummary` (surface counts, \
             flagship `pub extern \"c\"` entrypoints, approval \
             gates, enforced guarantees, dangerous-surface counts, \
             stores-writeable flag, descriptor sha256). Every \
             derived field is paired with a `BootSource` entry \
             naming the descriptor field that supplied the value, \
             mirroring the Grounded<T> sources posture that \
             `connector.drift_narration_grounded` uses for the \
             drift narrator. Replay-stable: two invocations on \
             the same source produce byte-identical output. The \
             first sub-slice of `35V2-P42-H-LR-per-app-ai-helpers` \
             to ship; `adversarial-refresh` and `pr-describe` \
             remain filed under the umbrella as the next slices.",
        out_of_scope_reason: "",
        positive_test_refs: &[
            "crates/corvid-abi/src/boot_summary.rs::boot_summary_grounds_every_derived_field_to_a_descriptor_source",
            "crates/corvid-cli/src/app_cmd.rs::boot_summary_for_minimal_app_renders_grounded_block",
        ],
        adversarial_test_refs: &[
            "crates/corvid-abi/src/boot_summary.rs::boot_summary_empty_surface_descriptor_returns_grounded_summary_not_sourceless",
            "crates/corvid-abi/src/boot_summary.rs::render_boot_summary_is_byte_identical_across_two_invocations",
            "crates/corvid-cli/src/app_cmd.rs::boot_summary_for_unparseable_source_returns_typed_error_not_panic",
        ],
    },
    Guarantee {
        id: "package.hosted_registry_available",
        kind: GuaranteeKind::Platform,
        class: GuaranteeClass::OutOfScope,
        phase: Phase::Platform,
        description:
            "A Corvid-operated public package registry service that \
             serves the published index format and source artifacts.",
        out_of_scope_reason:
            "No hosted Corvid package registry service runs yet; \
             The CLI ships the published index format + signed-publish \
             tooling (`corvid package publish`, `verify-registry`, \
             `verify-lock`) and `--url-base` accepts file:// and any \
             http endpoint a user runs themselves. A hosted public \
             registry is post-v1.0 work; see `docs/internals/package-manager-scope.md` \
             for the full boundary.",
        positive_test_refs: &[],
        adversarial_test_refs: &[],
    },
];

pub fn lookup(id: &str) -> Option<&'static Guarantee> {
    GUARANTEE_REGISTRY.iter().find(|g| g.id == id)
}

/// Iterate every guarantee in declaration order.
pub fn iter() -> impl Iterator<Item = &'static Guarantee> {
    GUARANTEE_REGISTRY.iter()
}

/// Iterate guarantees of a given class in declaration order.
pub fn by_class(class: GuaranteeClass) -> impl Iterator<Item = &'static Guarantee> {
    GUARANTEE_REGISTRY.iter().filter(move |g| g.class == class)
}

/// Iterate guarantees of a given kind in declaration order.
pub fn by_kind(kind: GuaranteeKind) -> impl Iterator<Item = &'static Guarantee> {
    GUARANTEE_REGISTRY.iter().filter(move |g| g.kind == kind)
}
