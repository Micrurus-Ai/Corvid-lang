# Corvid core semantics

> Auto-generated from `corvid_guarantees::GUARANTEE_REGISTRY`. **Do not hand-edit.** Update by running
> `cargo run -q -p corvid-cli -- contract regen-doc docs/reference/core-semantics.md` and committing the result.

Every public Corvid promise about effects, approvals, grounding, budgets, confidence, replay, provenance, and the ABI surface is enumerated below. Each row carries:

- a stable **id** referenced by diagnostics, tests, the bilateral verifier, and `corvid claim --explain`,
- a **kind** (which moat dimension it belongs to),
- a **class** — `static` (compiler refuses to produce a binary on violation), `runtime_checked` (runtime detects and surfaces), or `out_of_scope` (a documented promise that does NOT have a check today; reason recorded inline below),
- the pipeline **phase** that owns the enforcement.

Per the no-shortcuts rule, every `out_of_scope` entry carries an explicit reason. Anything Corvid does not defend appears below in plain language; we do not rely on omission.

## Summary

| id | kind | class | phase |
|----|------|-------|-------|
| `approval.dangerous_call_requires_token` | approval | static | typecheck |
| `approval.token_lexical_only` | approval | static | typecheck |
| `approval.dangerous_marker_preserved` | approval | out_of_scope | typecheck |
| `approval.reachable_entrypoints_require_contract` | approval | static | typecheck |
| `effect_row.body_completeness` | effect_row | static | typecheck |
| `effect_row.caller_propagation` | effect_row | out_of_scope | typecheck |
| `effect_row.import_boundary` | effect_row | static | resolve |
| `grounded.provenance_required` | grounded | static | typecheck |
| `grounded.propagation_across_calls` | grounded | out_of_scope | typecheck |
| `grounded.no_laundering` | grounded | static | typecheck |
| `budget.compile_time_ceiling` | budget | static | typecheck |
| `budget.runtime_termination` | budget | out_of_scope | runtime |
| `confidence.min_threshold` | confidence | static | typecheck |
| `replay.deterministic_pure_path` | replay | runtime_checked | runtime |
| `replay.trace_signature` | replay | runtime_checked | runtime |
| `provenance_trace.receipt_signature` | provenance_trace | runtime_checked | runtime |
| `abi_descriptor.cdylib_emission` | abi_descriptor | static | codegen |
| `abi_descriptor.byte_determinism` | abi_descriptor | static | codegen |
| `abi_descriptor.bilateral_source_match` | abi_descriptor | runtime_checked | abi_verify |
| `abi_attestation.envelope_signature` | abi_attestation | runtime_checked | abi_verify |
| `abi_attestation.descriptor_match` | abi_attestation | runtime_checked | abi_verify |
| `abi_attestation.absent_reports_unsigned` | abi_attestation | runtime_checked | abi_verify |
| `abi_attestation.sign_requires_claim_coverage` | abi_attestation | static | codegen |
| `jobs.cron_schedule_durable` | jobs | runtime_checked | runtime |
| `jobs.retry_budget_bound` | jobs | out_of_scope | runtime |
| `jobs.idempotency_key_uniqueness` | jobs | runtime_checked | runtime |
| `jobs.lease_exclusivity` | jobs | runtime_checked | runtime |
| `jobs.durable_resume` | jobs | runtime_checked | runtime |
| `jobs.cron_dst_correct` | jobs | runtime_checked | runtime |
| `jobs.approval_wait_resume` | jobs | out_of_scope | runtime |
| `jobs.loop_bounds_enforced` | jobs | out_of_scope | runtime |
| `jobs.replayable_side_effects` | jobs | out_of_scope | runtime |
| `auth.session_rotation_on_privilege_change` | auth | out_of_scope | runtime |
| `auth.api_key_at_rest_hashed` | auth | runtime_checked | runtime |
| `auth.jwt_kid_rotation` | auth | runtime_checked | runtime |
| `auth.oauth_pkce_required` | auth | runtime_checked | runtime |
| `auth.csrf_double_submit` | auth | out_of_scope | runtime |
| `tenant.cross_tenant_compile_error` | auth | out_of_scope | typecheck |
| `approval.policy_clause_static_check` | auth | out_of_scope | typecheck |
| `approval.batch_equivalence_typed` | auth | out_of_scope | typecheck |
| `approval.batch_refuses_cross_data_class_drift` | auth | runtime_checked | runtime |
| `approval.explain_sources_grounded` | auth | runtime_checked | runtime |
| `approval.confused_deputy_typecheck` | auth | out_of_scope | typecheck |
| `connector.scope_minimum_enforced` | connector | runtime_checked | runtime |
| `connector.write_requires_approval` | connector | out_of_scope | typecheck |
| `connector.rate_limit_respects_provider` | connector | runtime_checked | runtime |
| `connector.contract_drift_detected` | connector | out_of_scope | runtime |
| `connector.webhook_signature_verified` | connector | runtime_checked | runtime |
| `connector.replay_quarantine` | connector | runtime_checked | runtime |
| `observability.otel_conformance` | observability | runtime_checked | runtime |
| `observability.lineage_completeness` | observability | runtime_checked | runtime |
| `observability.redaction_determinism` | observability | runtime_checked | runtime |
| `observability.contract_aware_grouping` | observability | runtime_checked | runtime |
| `eval.drift_attribution` | observability | runtime_checked | runtime |
| `eval.promotion_signed_lineage` | observability | runtime_checked | runtime |
| `review_queue.cost_of_being_wrong_ranking` | observability | runtime_checked | runtime |
| `deploy.reproducible_build` | deploy | out_of_scope | platform |
| `deploy.attestation_chain` | deploy | runtime_checked | platform |
| `deploy.sbom_completeness` | deploy | runtime_checked | platform |
| `release.signed_artifact` | release | runtime_checked | platform |
| `upgrade.claim_regression_check` | upgrade | runtime_checked | platform |
| `ops.live_introspection_signed` | ops | out_of_scope | runtime |
| `claim.audit_runnable_artifacts` | claim | runtime_checked | platform |
| `platform.host_kernel_compromise` | platform | out_of_scope | platform |
| `platform.signing_key_compromise` | platform | out_of_scope | platform |
| `platform.toolchain_compromise` | platform | out_of_scope | platform |
| `package.hosted_registry_available` | platform | out_of_scope | platform |

## Detail

### Approval boundaries

#### `approval.dangerous_call_requires_token`
- **class**: static
- **phase**: typecheck

A call site invoking a `@dangerous` tool must have an `approve` token lexically in scope; otherwise the typechecker rejects the program.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::dangerous_tool_with_matching_approve_is_ok`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::dangerous_tool_without_approve_is_compile_error`
- `crates/corvid-types/src/tests.rs::tagged_unapproved_dangerous_call_carries_approval_guarantee_id`

#### `approval.token_lexical_only`
- **class**: static
- **phase**: typecheck

Approval tokens are lexically scoped — they cannot be returned, stored in fields, or passed across opaque boundaries to unlock a call site outside the original `approve` block.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::outer_approve_authorizes_inner_call`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::approve_does_not_leak_out_of_if_branch`
- `crates/corvid-types/src/tests.rs::mutation_nested_inner_approve_does_not_authorize_outer_call`

#### `approval.dangerous_marker_preserved`
- **class**: out_of_scope
- **phase**: typecheck

A `@dangerous` marker cannot be erased by re-exporting or aliasing the tool through another module — every public alias preserves the original danger annotation.

> **Why out of scope:** Structural property of the language, not a separately-fired diagnostic. Corvid's source syntax has no `import use` form that can declare the alias's effect — aliases inherit their source's `@dangerous` marker by construction. The property is verified indirectly: when a dangerous imported tool is aliased and then called without approve, the parent diagnostic `approval.dangerous_call_requires_token` fires correctly, which is only possible because the marker was preserved through the alias. The cited test_refs assert that parent-diagnostic firing through the alias path. Phase 35V-T1-B (2026-05-08) downgraded this row from `Static` to `OutOfScope` because no separate diagnostic site exists to tag with this id; the property remains documentary, the enforcement remains structural via the parent diagnostic. A future syntax slice that introduces an explicit alias-effect-override surface would promote this row back to `Static` with a tagged diagnostic at the override-rejection site.

#### `approval.reachable_entrypoints_require_contract`
- **class**: static
- **phase**: typecheck

Externally reachable routes, schedules, and exported agents are walked through their reachable agent calls; any reachable `@dangerous` tool call must still have a matching lexical approval contract.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::server_route_approve_authorizes_dangerous_tool`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::server_route_reachability_reports_helper_without_approval`
- `crates/corvid-types/src/tests.rs::schedule_reachability_reports_job_without_approval`

### Effect rows

#### `effect_row.body_completeness`
- **class**: static
- **phase**: typecheck

A function's declared effect row must cover every effect actually produced by its body (including effects of called functions); under-reporting is a compile error.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::mutation_tool_uses_declared_effect_is_ok`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::mutation_baseline_trust_violation_exists`
- `crates/corvid-types/src/tests.rs::mutation_multiple_effects_on_one_tool_compose_cost_and_trust`

#### `effect_row.caller_propagation`
- **class**: out_of_scope
- **phase**: typecheck

Callers inherit the union of their callees' effects unless they declare a wider row; callers cannot silently shrink the effect surface.

> **Why out of scope:** Subsumed by `effect_row.body_completeness` at the diagnostic level. The shipped `analyze_effects` analysis composes effects across calls (`collect_body_effects` walks the body and unions every called tool/prompt/agent's effect row into the composed profile) and fires a single `EffectConstraintViolation` per dimension when the declared row doesn't cover the composed result. The violation message says "dimension X: constraint requires Y, but composed value is Z" without distinguishing whether the offending contribution came from a direct body call or from a transitive callee — the unified analysis treats them identically. The user's mitigation is the same in both cases: widen the declared effect row to cover the composed value. Phase 35V-T1-B (2026-05-08) downgraded this row from `Static` to `OutOfScope` because the analyzer's `ConstraintViolation` struct does not carry a body-vs-callee source field, so there is no discriminable diagnostic site to tag with this id; the property is documentary, the enforcement is via the parent's unified diagnostic. A future slice that extends `ConstraintViolation` with a `source` field plus per-violation discrimination at the firing site would promote this row back to `Static`.

#### `effect_row.import_boundary`
- **class**: static
- **phase**: resolve

Cross-module imports preserve effect annotations exactly; an importer cannot use a re-exported function with a stripped or weakened effect row.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::python_import_with_unsafe_effect_warns`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::python_import_without_effects_is_rejected`

### Grounded provenance

#### `grounded.provenance_required`
- **class**: static
- **phase**: typecheck

Constructing a `Grounded<T>` value requires citing a source; unsourced `Grounded` construction is a compile error.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::mutation_direct_grounded_return_with_retrieval_chain_is_ok`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::mutation_grounded_return_without_retrieval_errors`

#### `grounded.propagation_across_calls`
- **class**: out_of_scope
- **phase**: typecheck

Provenance is preserved across function boundaries — a `Grounded<T>` returned from a callee retains its citation chain into the caller without separate annotation.

> **Why out of scope:** Subsumed by `grounded.provenance_required` at the diagnostic level. The shipped grounded-return analysis fires a single `UngroundedReturn` diagnostic when a function declares a `Grounded<T>` return type but the returned expression's provenance chain is empty. The check is unified: it does not distinguish whether the missing provenance came from a directly-constructed value (parent's framing: provenance must be cited at construction) or from a value flowed across a callee boundary (this row's framing: provenance must be preserved across calls). The user's mitigation is the same in both cases: ensure the returned value carries a non-empty provenance chain. Phase 35V-T1-B (2026-05-08) downgraded this row from `Static` to `OutOfScope` because the analyzer fires one diagnostic for both perspectives; there is no discriminable site to tag separately. The property is documentary; the enforcement is via the parent's unified diagnostic. A future slice that splits the analyzer to distinguish construction-site failures from call-boundary propagation failures would promote this row back to `Static`.

#### `grounded.no_laundering`
- **class**: static
- **phase**: typecheck

An agent annotated `@grounded_pure` fails compile if its body launders a `Grounded<T>` value into a non-grounded slot — either via the silent legacy coercion at a slot-check site (return / parameter / field), an explicit `.unwrap_discarding_sources()` call, or a transitive call into another agent not itself marked `@grounded_pure`. The moat composes through the call graph the same way `@deterministic` does.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::grounded_pure_passes_when_body_preserves_grounded`
- `crates/corvid-types/src/tests.rs::grounded_pure_passes_when_calling_another_grounded_pure_agent`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::grounded_pure_rejects_implicit_coercion`
- `crates/corvid-types/src/tests.rs::grounded_pure_rejects_explicit_unwrap`
- `crates/corvid-types/src/tests.rs::grounded_pure_rejects_call_to_non_grounded_pure_agent`

### Budgets

#### `budget.compile_time_ceiling`
- **class**: static
- **phase**: typecheck

An agent annotated `@budget($X)` fails compile if the sum of statically known per-call costs along any reachable path exceeds `$X`.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::multi_dimensional_budget_within_bound_is_clean`
- `crates/corvid-types/src/tests.rs::mutation_budget_within_limit_is_ok`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::multi_dimensional_budget_violation_reports_path`
- `crates/corvid-types/src/tests.rs::mutation_budget_exceeded_is_effect_violation`

#### `budget.runtime_termination`
- **class**: out_of_scope
- **phase**: runtime

Live runtime termination of an agent when actual runtime cost crosses the `@budget($X)` threshold mid-execution.

> **Why out of scope:** Today Corvid enforces budgets at compile time via `budget.compile_time_ceiling`, and the runtime observes per-call cost in trace events; live mid-execution termination on threshold crossing is not yet implemented. A future slice can promote this entry back to `RuntimeChecked` once the enforcement ships. The compile-time ceiling is the load-bearing guarantee for v1.0.

### Confidence thresholds

#### `confidence.min_threshold`
- **class**: static
- **phase**: typecheck

An agent annotated `@min_confidence(X)` requires every input carrying a confidence tag to meet `X`; lower-confidence inputs are rejected at the call site.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::min_confidence_passes_when_composed_confidence_meets_floor`
- `crates/corvid-types/src/tests.rs::tagged_invalid_confidence_carries_confidence_guarantee_id`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::min_confidence_fires_when_composed_confidence_below_floor`
- `crates/corvid-types/src/tests.rs::effect_confidence_out_of_range_is_rejected`

### Replay determinism

#### `replay.deterministic_pure_path`
- **class**: runtime_checked
- **phase**: runtime

A trace recorded from a `@replayable` agent reproduces deterministically across `corvid replay` invocations on the same compiled binary; non-deterministic divergence raises the documented replay-divergence error.

**Positive tests:**

- `crates/corvid-types/src/tests.rs::replayable_agent_with_pure_body_compiles_clean`
- `crates/corvid-types/src/tests.rs::deterministic_agent_with_pure_body_compiles_clean`

**Adversarial tests:**

- `crates/corvid-types/src/tests.rs::deterministic_agent_calling_tool_is_rejected`
- `crates/corvid-types/src/tests.rs::deterministic_agent_calling_prompt_is_rejected`

#### `replay.trace_signature`
- **class**: runtime_checked
- **phase**: runtime

Trace receipts produced with `--sign` carry a DSSE envelope whose signature `corvid receipt verify` checks against the supplied verifying key.

**Positive tests:**

- `crates/corvid-cli/tests/receipt_signing.rs::sign_then_verify_roundtrips_end_to_end`

**Adversarial tests:**

- `crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_envelope_signed_with_different_key`
- `crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_tampered_payload`

### Provenance traces

#### `provenance_trace.receipt_signature`
- **class**: runtime_checked
- **phase**: runtime

`corvid receipt verify` rejects any DSSE-wrapped receipt whose signature does not validate against the supplied verifying key, with a non-zero exit and the documented `verification failed` message.

**Positive tests:**

- `crates/corvid-cli/tests/receipt_signing.rs::sign_then_verify_roundtrips_end_to_end`

**Adversarial tests:**

- `crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_envelope_signed_with_different_key`
- `crates/corvid-cli/tests/receipt_signing.rs::verify_rejects_tampered_payload`

### ABI descriptor

#### `abi_descriptor.cdylib_emission`
- **class**: static
- **phase**: codegen

Every `corvid build --target=cdylib` output exports a `CORVID_ABI_DESCRIPTOR` symbol whose payload is the canonical effect/approval/provenance surface for the compiled program.

**Positive tests:**

- `crates/corvid-codegen-cl/tests/cdylib_emission.rs::cdylib_target_produces_shared_library_file`
- `crates/corvid-codegen-cl/tests/cdylib_emission.rs::cdylib_symbol_is_resolvable_via_dlopen`

**Adversarial tests:**

- `crates/corvid-cli/tests/build_cdylib.rs::cli_build_cdylib_fails_cleanly_on_non_scalar_signature`

#### `abi_descriptor.byte_determinism`
- **class**: static
- **phase**: codegen

Two byte-identical Corvid sources compiled with the same toolchain version produce byte-identical descriptor JSON; the descriptor is canonical, not pretty-printed.

**Positive tests:**

- `crates/corvid-abi/tests/determinism.rs::identical_source_produces_byte_identical_descriptor_modulo_generated_at`
- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::descriptor_bytes_are_byte_identical_across_two_emissions_of_same_source`

**Adversarial tests:**

- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::descriptor_section_rejects_random_byte_flips`

#### `abi_descriptor.bilateral_source_match`
- **class**: runtime_checked
- **phase**: abi_verify

`corvid-abi-verify --source <file> <cdylib>` independently rebuilds the ABI descriptor from source and byte-compares it against the embedded `CORVID_ABI_DESCRIPTOR` symbol; mismatch is rejected before host acceptance.

**Positive tests:**

- `crates/corvid-abi-verify/src/lib.rs::verifier_accepts_matching_cdylib_descriptor`
- `crates/corvid-abi-verify/src/lib.rs::verifier_accepts_matching_cdylib_with_imported_agent`

**Adversarial tests:**

- `crates/corvid-abi-verify/src/lib.rs::verifier_rejects_source_descriptor_mismatch`

### ABI attestation

#### `abi_attestation.envelope_signature`
- **class**: runtime_checked
- **phase**: abi_verify

`corvid receipt verify-abi` rejects a signed cdylib whose attestation envelope does not validate against the supplied verifying key, exiting 1 with `attestation verification failed`.

**Positive tests:**

- `crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_verifies_against_matching_key`
- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::signing_key_round_trip_baseline`

**Adversarial tests:**

- `crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_rejects_wrong_key`
- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::dsse_envelope_signature_tampering_is_rejected`
- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::dsse_envelope_payload_tampering_is_rejected`
- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::dsse_envelope_payload_type_swap_is_rejected`
- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::attestation_section_rejects_every_magic_or_version_byte_flip`
- `crates/corvid-abi/tests/byte_fuzz_corpus.rs::attestation_section_body_mutations_break_signature_verification`

#### `abi_attestation.descriptor_match`
- **class**: runtime_checked
- **phase**: abi_verify

After signature validation, the recovered attestation payload must bit-match the embedded `CORVID_ABI_DESCRIPTOR`; mismatch is rejected even when the signature is valid.

**Positive tests:**

- `crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_verifies_against_matching_key`

**Adversarial tests:**

- `crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_rejects_wrong_key`

#### `abi_attestation.absent_reports_unsigned`
- **class**: runtime_checked
- **phase**: abi_verify

`corvid receipt verify-abi` on a cdylib lacking the `CORVID_ABI_ATTESTATION` symbol exits 2 with the documented `unsigned` message, leaving the host policy to decide whether to accept it.

**Positive tests:**

- `crates/corvid-cli/tests/abi_attestation.rs::signed_cdylib_verifies_against_matching_key`

**Adversarial tests:**

- `crates/corvid-cli/tests/abi_attestation.rs::unsigned_cdylib_reports_absent_attestation`

#### `abi_attestation.sign_requires_claim_coverage`
- **class**: static
- **phase**: codegen

`corvid build --target=cdylib --sign` refuses to sign when any contract declared by the source lacks a non-out-of-scope guarantee id in the descriptor's signed claim set.

**Positive tests:**

- `crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_accepts_registered_contracts`

**Adversarial tests:**

- `crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_rejects_missing_declared_contract_id`
- `crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_rejects_out_of_scope_contract_id`

### Durable jobs

#### `jobs.cron_schedule_durable`
- **class**: runtime_checked
- **phase**: runtime

A `schedule "cron" zone "…" -> job(args)` declaration persists to the durable queue store and survives process restart. Slice 35-N walks `Decl::Schedule` so a signed cdylib that declares a cron schedule cannot ship without this guarantee in its descriptor.

**Positive tests:**

- `crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_walks_schedule_decl`

**Adversarial tests:**

- `crates/corvid-driver/src/build/tests.rs::signed_claim_coverage_rejects_schedule_without_jobs_coverage`

#### `jobs.retry_budget_bound`
- **class**: out_of_scope
- **phase**: runtime

`@retry(max_attempts: N, backoff: ...)` bounds the runtime retry envelope of a job so a transient failure cannot escalate into unbounded re-spend.

> **Why out of scope:** The runtime queue and lease envelopes are shipped and the retry policy is configurable at enqueue time via the host API + `corvid jobs limit`. `@retry` as a Corvid source-level attribute is filed as a post-v1.0 ergonomic improvement (35V2-P38-H), not a launch-blocker — the runtime behaviour the attribute would surface is already shipped. Slice 38K promoted the runtime; the syntactic promotion of this row tracks with the post-v1.0 syntax slice.

#### `jobs.idempotency_key_uniqueness`
- **class**: runtime_checked
- **phase**: runtime

Across N concurrent workers, exactly one durable queue row exists for a given non-null idempotency key. Enforced by a partial UNIQUE INDEX on `queue_jobs(idempotency_key) WHERE idempotency_key IS NOT NULL` in the SQLite schema, plus the existing `enqueue_typed_idempotent` collision-fallback path that returns the surviving row when the insert hits the UNIQUE constraint.

**Positive tests:**

- `crates/corvid-runtime/src/queue/tests/durable_basics.rs::durable_queue_idempotency_key_collapses_duplicate_jobs`

**Adversarial tests:**

- `crates/corvid-runtime/tests/durability_corpus.rs::t38l_d1_four_workers_collapse_to_one_row`

#### `jobs.lease_exclusivity`
- **class**: runtime_checked
- **phase**: runtime

A job lease prevents two workers from running the same job concurrently. Slice 38K's `WorkerPool` over `DurableQueueRuntime` runs N tokio tasks each contesting `lease_next_at`; the SQLite UPDATE that flips `pending` → `leased` is atomic, so exactly one worker wins each contention round. Lease expiry plus a fresh worker re-leasing is shipped (slice 38L's D3 test); heartbeat extension for long-running steps remains a follow-up.

**Positive tests:**

- `crates/corvid-runtime/src/worker_pool.rs::t38k_pool_runs_each_job_exactly_once`

**Adversarial tests:**

- `crates/corvid-runtime/src/worker_pool.rs::t38k_two_workers_cannot_both_lease_same_job`
- `crates/corvid-runtime/src/worker_pool.rs::t38k_pool_drains_gracefully_without_claiming_new_work`

#### `jobs.durable_resume`
- **class**: runtime_checked
- **phase**: runtime

A worker that drops uncleanly mid-step (the SIGKILL surrogate the queue runtime is responsible for) leaves behind durable checkpoint rows; a fresh worker that opens the same SQLite file after the lease TTL elapses can re-lease the job and resume from those checkpoints. SQLite WAL fsync makes this property structural. The count-bounded `no double LLM call` extension joins the Phase 21 Replay corpus when step-skip semantics land at the VM layer.

**Positive tests:**

- `crates/corvid-runtime/src/queue/tests/checkpoints.rs::durable_queue_records_ordered_agent_checkpoints`

**Adversarial tests:**

- `crates/corvid-runtime/tests/durability_corpus.rs::t38l_d3_checkpoints_survive_unclean_shutdown`

#### `jobs.cron_dst_correct`
- **class**: runtime_checked
- **phase**: runtime

Cron schedules expressed in `America/New_York` (and other DST-observing timezones) produce monotonic UTC fire times across the spring-forward and fall-back transitions, with no duplicates and no fire at the non-existent local instant. `chrono-tz` is wired into the queue runtime; the cron-crate's `Schedule::after` iterator is timezone-aware.

**Positive tests:**

- `crates/corvid-runtime/tests/durability_corpus.rs::t38l_d2_dst_spring_forward_is_deterministic`

**Adversarial tests:**

- `crates/corvid-runtime/tests/durability_corpus.rs::t38l_d2_dst_fall_back_is_monotonic`

#### `jobs.approval_wait_resume`
- **class**: out_of_scope
- **phase**: runtime

An approval boundary inside a job pauses the run until an approval token arrives, expires, or is denied; the resume path writes the audit transition.

> **Why out of scope:** Runtime approval-wait state ships and is reachable via `corvid jobs wait-approval` + `corvid jobs approval approve/deny` (the shipped surface). `await_approval` as a Corvid source-level keyword is filed as a post-v1.0 ergonomic improvement (35V2-P38-H), not a launch-blocker — the runtime behaviour already ships, the syntax just surfaces it more compactly.

#### `jobs.loop_bounds_enforced`
- **class**: out_of_scope
- **phase**: runtime

Agent loops driven by jobs honor max-steps, max-wall-time, max-spend, and max-tool-calls; exceeding any bound escalates or terminates with trace evidence.

> **Why out of scope:** Loop-bound storage + tracking ship at `corvid-runtime::queue::loops` (set_loop_limits, loop_limits, loop_usage, record_loop_usage_report) and the multi-worker runner from 38K is in place. What's missing is the runner's enforcement hook — exceeding a bound should terminate or escalate the job with trace evidence; today the bounds are recorded but not enforced at the runner. Filed as a v1.0 launch-readiness slice (35V2-P38-D-LR-loop-bounds-enforcement-hook) — promotes this row to RuntimeChecked when the enforcement hook ships.

#### `jobs.replayable_side_effects`
- **class**: out_of_scope
- **phase**: runtime

A job marked `@replayable` records its tool / prompt / approval / DB side-effects into the trace so a later `corvid replay <job-trace>` reproduces the run without re-issuing real side-effect calls.

> **Why out of scope:** The Phase 21 replay infrastructure ships and the queue runtime persists step checkpoints, but the integration wiring that would let a recorded job trace drive a replay-mode job runner (with the LlmRegistry quarantined so a real provider call cannot leave the process) does not exist. The 35V2-P38-A audit assumed the wiring was present and only the test was missing; recon under 35V2-P38-C found the wiring is the work, not the test. Filed as a v1.0 launch-readiness slice (35V2-P38-C-deferred) — promotes this row to RuntimeChecked when the wiring ships and the cross-layer assertion test joins the durability corpus.

### Auth and approvals

#### `auth.session_rotation_on_privilege_change`
- **class**: out_of_scope
- **phase**: runtime

A session id rotates on privilege escalation (role upgrade, password change) so a stolen pre-escalation cookie cannot exercise the post-escalation privilege.

> **Why out of scope:** Session storage + the rotation primitive exist in `corvid-runtime/src/auth/sessions.rs` (verified by the session_rotation_invalidates_old_token_* test). The gap is the runtime hook that fires the rotation on a privilege-change event (role upgrade, password change) inside the generated axum server. Filed as launch- readiness slice `35V2-P39-D-LR-session-rotation-hook` — promotes this row to RuntimeChecked when the hook ships + the named-threat test for session-fixation lands alongside it.

#### `auth.api_key_at_rest_hashed`
- **class**: runtime_checked
- **phase**: runtime

API keys are stored only as Argon2id hashes; the plaintext leaves Corvid memory exactly once at issuance and is never logged. Verified by the existing `hash_api_key_secret`/`verify_api_key_secret` path in `corvid-runtime/src/auth.rs`.

**Positive tests:**

- `crates/corvid-runtime/src/auth/api_keys.rs::api_key_runtime_resolves_service_actor_with_argon2_hash_and_redacted_audit`

**Adversarial tests:**

- `crates/corvid-runtime/src/auth/api_keys.rs::api_key_runtime_rejects_wrong_tenant_revoked_expired_and_user_actors`

#### `auth.jwt_kid_rotation`
- **class**: runtime_checked
- **phase**: runtime

JWT verification fetches the JWKS, picks the key by `kid`, verifies the signature with `jsonwebtoken`, and rejects tokens whose `kid` is missing from the current JWKS, whose alg does not match the contract, whose signature fails to verify, whose exp/iss/aud do not align with the contract, or whose required subject/tenant claim is missing. Out-of-scope at Phase 39 base; promoted to `RuntimeChecked` by slice 39K when `corvid-runtime/src/jwt_verify/` shipped.

**Positive tests:**

- `crates/corvid-runtime/src/jwt_verify/verifier.rs::parse_alg_accepts_supported_and_refuses_others`
- `crates/corvid-runtime/src/jwt_verify/verifier.rs::decoding_key_for_rsa_jwk_constructs`
- `crates/corvid-runtime/src/jwt_verify/mod.rs::error_slugs_are_stable_for_audit_log`

**Adversarial tests:**

- `crates/corvid-runtime/src/jwt_verify/verifier.rs::kid_downgrade_returns_kid_not_found`
- `crates/corvid-runtime/src/jwt_verify/verifier.rs::header_alg_must_match_contract_alg`
- `crates/corvid-runtime/src/jwt_verify/verifier.rs::alg_none_in_header_is_refused`
- `crates/corvid-runtime/src/jwt_verify/verifier.rs::malformed_token_is_refused_before_fetch`
- `crates/corvid-runtime/src/jwt_verify/verifier.rs::jwks_fetch_failure_is_surfaced`
- `crates/corvid-runtime/src/jwt_verify/verifier.rs::decoding_key_for_rejects_rsa_without_n`
- `crates/corvid-runtime/src/jwt_verify/verifier.rs::decoding_key_for_rejects_unknown_kty`

#### `auth.oauth_pkce_required`
- **class**: runtime_checked
- **phase**: runtime

OAuth callback state requires PKCE for public clients; the state record carries the code-verifier hash and is single-use, tenant-scoped, and expiry-bound.

**Positive tests:**

- `crates/corvid-runtime/src/auth/oauth.rs::oauth_callback_state_is_hashed_single_use_and_restart_safe`

**Adversarial tests:**

- `crates/corvid-runtime/src/auth/oauth.rs::oauth_callback_rejects_expired_and_cross_tenant_state`

#### `auth.csrf_double_submit`
- **class**: out_of_scope
- **phase**: runtime

CSRF protection on cookie-bearing requests uses a double-submit token verified by HMAC-SHA256.

> **Why out of scope:** Token shape is documented in the design brief; the middleware path that enforces it on every cookie-bearing POST / PUT / PATCH / DELETE is not yet wired into the generated axum server. Filed as launch-readiness slice `35V2-P39-C-LR-csrf-middleware` — promotes this row to RuntimeChecked when the middleware ships + the named-threat test for `CSRF-bypass-on-PUT/PATCH/DELETE` lands alongside it.

#### `tenant.cross_tenant_compile_error`
- **class**: out_of_scope
- **phase**: typecheck

A function whose actor came from tenant A may not pass a record owned by tenant B to a tool that writes back into A — the typechecker rejects the cross-tenant reference.

> **Why out of scope:** Tenant tagging exists in runtime envelopes + the CLI (`corvid approvals` honours tenant scoping; the approval_bypass_rejects_tenant_crossing_actor test pins the runtime half). The parser-level `tenant Org { ... }` block + the typechecker reachability that would refuse a cross-tenant value at compile time does not exist yet. Filed as post-v1.0 `35V2-P39-I-post-v1.0-auth-syntax-sugar` — the runtime behaviour ships today, the syntactic promotion of this row tracks with the post-v1.0 parser surface slice.

#### `approval.policy_clause_static_check`
- **class**: out_of_scope
- **phase**: typecheck

An `approval Name:` block's `policy { ... }` clause type-checks at compile time so a malformed predicate (wrong field name, wrong type, undeclared role) cannot ship.

> **Why out of scope:** Approval store + queue API ship and are reachable via `corvid approvals queue/inspect/approve/deny`. The `approval Name:` parser-level block is post-v1.0 ergonomic surface — filed as `35V2-P39-I-post-v1.0-auth-syntax-sugar`. The runtime behaviour (typed approval contracts with required fields validated at issue time) ships today via the host API; the source-level `policy { ... }` clause is the sugar.

#### `approval.batch_equivalence_typed`
- **class**: out_of_scope
- **phase**: typecheck

An `approval ... batch_with: same_tool, same_data_class, same_role` clause groups equivalent approvals so a reviewer can approve one record and have N equivalent-by-typed-shape records auto-resolve.

> **Why out of scope:** The runtime half of the batch-equivalence guarantee ships today as `approval.batch_refuses_cross_data_class_drift` (RuntimeChecked): `corvid approvals batch` refuses to span >1 data class unless `--require-data-class` pins the batch. The typecheck-time `batch_with: same_tool, same_data_class, same_role` source-level clause is post-v1.0 ergonomic surface — filed as `35V2-P39-I-post-v1.0-auth-syntax-sugar`. The runtime check prevents the threat today; the source-level sugar lets contracts declare the batch shape directly.

#### `approval.batch_refuses_cross_data_class_drift`
- **class**: runtime_checked
- **phase**: runtime

`corvid approvals batch` refuses outright when the supplied ids span >1 `data_class` unless the operator pins the batch with `--require-data-class <CLASS>`. Catches the `batch-approval-drift-across-data-classes` threat where `financial` and `pii` records would otherwise resolve in the same invocation under a single reviewer's role check.

**Positive tests:**

- `crates/corvid-cli/src/approvals_cmd/interaction.rs::approvals_batch_require_data_class_pins_to_one_class`

**Adversarial tests:**

- `crates/corvid-cli/src/approvals_cmd/interaction.rs::approvals_batch_refuses_cross_data_class_drift_without_pin`

#### `approval.explain_sources_grounded`
- **class**: runtime_checked
- **phase**: runtime

`corvid approvals explain <id>` renders a typed reviewer summary whose `sources` array names every audit-event id the explanation consulted — the Grounded<T> shape at the JSON layer. Every transition surfaced in the explanation has a back-reference in `sources`, so a reviewer can audit-trail every claim back to a queue row. Cross-tenant requests are refused with an explicit diagnostic rather than silently leaking state.

**Positive tests:**

- `crates/corvid-cli/src/approvals_cmd/explain.rs::approvals_explain_pending_carries_grounded_sources`

**Adversarial tests:**

- `crates/corvid-cli/src/approvals_cmd/explain.rs::approvals_explain_cross_tenant_refused`

#### `approval.confused_deputy_typecheck`
- **class**: out_of_scope
- **phase**: typecheck

A reachable path from any route or job to a `@dangerous` tool must have an `approve` token whose `required_role` covers every reachable caller — otherwise typecheck rejects.

> **Why out of scope:** Lexical-scope approve-presence check ships (`approval.dangerous_call_requires_token` + `approval.token_lexical_only`). The role-coverage extension — typecheck fails when a reachable caller's role is not covered by the approve's `required_role` — needs a typechecker pass that walks the call graph from every route / job entry point. Filed as launch- readiness slice `35V2-P39-J-LR-role-coverage-reachability` — promotes this row to Static when the analysis ships.

### Connectors

#### `connector.scope_minimum_enforced`
- **class**: runtime_checked
- **phase**: runtime

A connector cannot use a scope its manifest does not declare and an actor cannot use a scope its auth state does not authorise. The runtime fires `ConnectorAuthError::MissingScope` (or `UnknownScope`) before any HTTP layer touches the network, so a leaked low-scope token cannot escalate to a higher-scope operation by guessing the scope id.

**Positive tests:**

- `crates/corvid-connector-runtime/src/runtime.rs::mock_mode_checks_auth_rate_limit_and_emits_trace`

**Adversarial tests:**

- `crates/corvid-connector-runtime/tests/threat_corpus.rs::t1_github_rejects_unauthorised_scope`
- `crates/corvid-connector-runtime/tests/threat_corpus.rs::t1_gmail_rejects_unauthorised_scope`
- `crates/corvid-connector-runtime/tests/threat_corpus.rs::t1_slack_rejects_unauthorised_scope`

#### `connector.write_requires_approval`
- **class**: out_of_scope
- **phase**: typecheck

A connector method whose effect set names a write (`gmail.send`, `slack.post`, `github.create_issue`) reaches typecheck only when its caller has a matching `approve` boundary in lexical scope.

> **Why out of scope:** Manifest declares write effects (`*.write`, `send_*`) in `shipped_manifests` and the runtime refuses unsafe effects via `ConnectorRuntimeError::ReplayWriteQuarantined` when not authorized. The source-level `connector ... uses dangerous` declaration that would let typecheck refuse a call without a lexical-scope `approve` does not exist yet — connectors are configured by Rust data, not Corvid source. Filed as post-v1.0 `35V2-P41-I-post-v1.0-connector-syntax-sugar` — the runtime behaviour (manifest enforcement at write time) ships today; the typecheck-time form is the syntax sugar that promotes this row to Static.

#### `connector.rate_limit_respects_provider`
- **class**: runtime_checked
- **phase**: runtime

A connector honors the provider's rate-limit advice (`Retry-After`, 429, 5xx). The shared `ReqwestRealClient` parses RFC 7231 `Retry-After` integer-seconds into milliseconds via `parse_retry_after_header` and surfaces it as `ConnectorRuntimeError::RateLimited { retry_after_ms }`, which the runtime forwards verbatim to the caller instead of retrying behind their back.

**Positive tests:**

- `crates/corvid-connector-runtime/src/real_client.rs::parse_retry_after_seconds_form`

**Adversarial tests:**

- `crates/corvid-connector-runtime/src/real_client.rs::parse_retry_after_returns_none_for_malformed`
- `crates/corvid-connector-runtime/src/runtime.rs::real_mode_propagates_rate_limited_from_bound_client`
- `crates/corvid-connector-runtime/tests/threat_corpus.rs::t5_rate_limited_propagates_retry_after_ms`

#### `connector.contract_drift_detected`
- **class**: out_of_scope
- **phase**: runtime

`corvid connectors check --live` compares the manifest to the live (or recorded-cassette) provider response shape and exits non-zero when fields drift.

> **Why out of scope:** Slice 41L wired `corvid connectors check`, which validates every shipped manifest against the manifest schema and reports diagnostics per connector (`shipped_manifests` → `validate_connector_manifest`). The `--live` drift-narration path that compares the manifest to a live provider response shape is gated behind `CORVID_PROVIDER_LIVE=1` and currently returns an explicit `Err` directing the caller to a future slice. Filed as launch-readiness slice `35V2-P41-D-LR-connector-drift-narration` — promotes this row to RuntimeChecked when the live drift path ships + the AI-helper narrator layer (35V2-P41-H-LR) surfaces the diff in human-readable form.

#### `connector.webhook_signature_verified`
- **class**: runtime_checked
- **phase**: runtime

Inbound webhook payloads from Slack, GitHub, and Linear are HMAC-SHA256 verified against the manifest's shared secret before any handler runs. Per-provider schemes are honored: GitHub uses `X-Hub-Signature-256: sha256=<hex>`, Slack uses `v0:<ts>:<body>` with a 5-minute replay window, and Linear uses a bare hex digest. Comparison is constant-time; a malformed header, mismatched digest, or stale Slack timestamp returns a categorical `WebhookVerificationOutcome` that the dispatcher must reject before any side effect.

**Positive tests:**

- `crates/corvid-connector-runtime/src/webhook_verify.rs::github_verifies_correct_signature`
- `crates/corvid-connector-runtime/src/webhook_verify.rs::slack_verifies_correct_signature_inside_window`
- `crates/corvid-connector-runtime/src/webhook_verify.rs::linear_verifies_correct_signature`

**Adversarial tests:**

- `crates/corvid-connector-runtime/tests/threat_corpus.rs::t7_github_webhook_forgery_rejected`
- `crates/corvid-connector-runtime/tests/threat_corpus.rs::t7_slack_webhook_replay_outside_window_rejected`
- `crates/corvid-connector-runtime/tests/threat_corpus.rs::t7_linear_webhook_wrong_secret_rejected`

#### `connector.replay_quarantine`
- **class**: runtime_checked
- **phase**: runtime

A connector running in replay mode must not perform provider writes. The runtime returns `ConnectorRuntimeError::ReplayWriteQuarantined` for any scope whose effects include a `*.write` or `send_*` effect when the active mode is `Replay`, regardless of whether a real client is bound. Read-shaped operations still complete from the recorded cassette so deterministic replay continues to work.

**Positive tests:**

- `crates/corvid-connector-runtime/src/test_kit.rs::fixture_runs_mock_and_replay_read_paths`

**Adversarial tests:**

- `crates/corvid-connector-runtime/src/runtime.rs::replay_mode_quarantines_writes`
- `crates/corvid-connector-runtime/src/test_kit.rs::fixture_proves_replay_write_quarantine`
- `crates/corvid-connector-runtime/src/calendar.rs::calendar_replay_quarantines_writes`
- `crates/corvid-connector-runtime/src/slack.rs::slack_replay_quarantines_writes`

### Observability and evals

#### `observability.otel_conformance`
- **class**: runtime_checked
- **phase**: runtime

Lineage events flow through the standard `opentelemetry` + `opentelemetry-otlp` SDK and emit OTLP/HTTP spans whose attributes carry `corvid.guarantee_id`, `corvid.cost_usd`, `corvid.approval_id`, `corvid.replay_key`. The attribute set is constructed by `corvid_runtime::otel_sdk_export::corvid_span_attributes` and the live wire path is exercised by the docker-compose Jaeger harness in `docs/operations/observability-conformance.md`.

**Positive tests:**

- `crates/corvid-runtime/src/otel_sdk_export.rs::span_attributes_include_corvid_named_keys`
- `crates/corvid-runtime/src/otel_sdk_export.rs::span_name_uses_corvid_prefix_with_kind`
- `crates/corvid-runtime/src/otel_sdk_export.rs::span_kind_maps_lineage_to_otel`

**Adversarial tests:**

- `crates/corvid-runtime/src/otel_sdk_export.rs::span_attributes_omit_missing_optional_keys`
- `crates/corvid-runtime/src/otel_sdk_export.rs::sdk_exporter_reaches_in_process_otlp_receiver`

#### `observability.lineage_completeness`
- **class**: runtime_checked
- **phase**: runtime

Every lineage event carries a (trace_id, span_id) pair plus parent linkage when a parent exists, so a SQL JOIN against the local trace store reconstructs the route → job → agent → prompt → tool → approval → DB tree. Validated on every event via `corvid_runtime::lineage::validate_lineage`.

**Positive tests:**

- `crates/corvid-runtime/src/lineage.rs::lineage_ids_are_stable_and_parented_across_backend_kinds`

**Adversarial tests:**

- `crates/corvid-runtime/src/lineage.rs::lineage_validation_fails_closed_for_missing_parent_or_duplicate_root`

#### `observability.redaction_determinism`
- **class**: runtime_checked
- **phase**: runtime

Redacting the same lineage event twice with the same `LineageRedactionPolicy` yields byte-identical output; trace topology (trace_id, span_id, parent linkage) is preserved across redaction so observe / eval / OTel keep correlating after sensitive values are removed.

**Positive tests:**

- `crates/corvid-runtime/src/lineage_redact.rs::redaction_preserves_topology_and_redacts_identifiers_deterministically`

**Adversarial tests:**

- `crates/corvid-runtime/src/lineage_redact.rs::redaction_removes_obvious_secrets_from_serialized_lineage`

#### `observability.contract_aware_grouping`
- **class**: runtime_checked
- **phase**: runtime

`corvid observe show` groups incidents by guarantee_id, effect, budget, provenance, and approval rule rather than by service.name — so an analyst's first pivot lands on the contract that broke. Implemented by `lineage_incidents::group_lineage_incidents`.

**Positive tests:**

- `crates/corvid-runtime/src/lineage_incidents.rs::incidents_group_by_guarantee_effect_budget_provenance_and_approval`

**Adversarial tests:**

- `crates/corvid-runtime/src/lineage_incidents.rs::non_incident_ok_events_are_not_grouped`

#### `eval.drift_attribution`
- **class**: runtime_checked
- **phase**: runtime

`corvid eval-drift --explain` decomposes the drift between two trace runs into the four named dimensions (model_id, prompt_hash, retrieval_index_hash, input_fingerprint) plus a residual percentage for unattributable changes. The output's `sources` array carries the trace_id + span_id of every event the analysis consulted.

**Positive tests:**

- `crates/corvid-cli/src/observe_helpers_cmd/eval_drift.rs::drift_explain_attributes_model_swap`

**Adversarial tests:**

- `crates/corvid-cli/src/observe_helpers_cmd/eval_drift.rs::drift_explain_surfaces_residual_when_status_flips_alone`

#### `eval.promotion_signed_lineage`
- **class**: runtime_checked
- **phase**: runtime

`corvid eval-from-feedback` synthesises a typed eval fixture from a 'wrong answer' feedback record, redacting the matching lineage trace via the production redaction policy before writing the fixture. The fixture's `sources` field lists every redacted event so downstream consumers can reconstruct evidence without seeing raw identifiers.

**Positive tests:**

- `crates/corvid-cli/src/observe_helpers_cmd/eval_from_feedback.rs::eval_generate_from_feedback_writes_redacted_fixture`

**Adversarial tests:**

- `crates/corvid-cli/src/observe_helpers_cmd/eval_from_feedback.rs::eval_generate_from_feedback_missing_trace_id_refused`

#### `review_queue.cost_of_being_wrong_ranking`
- **class**: runtime_checked
- **phase**: runtime

`corvid review-queue list --rank=cost-of-being-wrong` surfaces low-confidence + high-risk outputs ranked by the `cost_of_being_wrong` policy.

**Positive tests:**

- `crates/corvid-cli/src/review_queue_cmd.rs::rank_cost_of_being_wrong_sorts_highest_first`

**Adversarial tests:**

- `crates/corvid-cli/src/review_queue_cmd.rs::rank_unknown_policy_refused`

### Deploy packaging

#### `deploy.reproducible_build`
- **class**: out_of_scope
- **phase**: platform

Building the same `corvid deploy package` input twice on two different hosts produces bit-identical signed artifacts (binary + SBOM + DSSE attestation envelope). A second build that differs is a build-environment leak — timestamps, hostnames, paths — and the verification CI must reject it.

> **Why out of scope:** Slice 43R landed the reproducible-build CI workflow at `.github/workflows/reproducible-build.yml`. The workflow builds the corvid CLI twice on Ubuntu 22.04 with SOURCE_DATE_EPOCH pinned to the commit time and two separate target directories, SHA-256s the outputs, and fails if the hashes differ. The workflow has not yet completed a green run on `main` — until that first run lands green (or we close the determinism gap it surfaces), the row stays OutOfScope. Promotion is mechanical at that point: change class to RuntimeChecked + add the workflow run URL as the test reference. Cross-platform (Ubuntu / macOS / Windows) and cross-host reproducibility are explicit non-scope for this row — they need deterministic-toolchain work beyond the v1.0 surface.

#### `deploy.attestation_chain`
- **class**: runtime_checked
- **phase**: platform

`corvid deploy package --cdylib <path>` binds the deploy attestation to the SHA-256 of the cdylib's bytes; the cdylib itself carries its `corvid claim --explain` embedded attestation, so the chain `claim --explain → cdylib bytes → deploy attestation` cannot drift without changing one of the digests. The attestation payload carries `chain_status: "complete"` + `cdylib_sha256: <hex>` when `--cdylib` is provided; `chain_status: "incomplete"` + `cdylib_sha256: null` when omitted so downstream verification can refuse an unchained deploy.

**Positive tests:**

- `crates/corvid-cli/src/deploy_cmd.rs::deploy_attestation_binds_to_cdylib_digest_when_provided`

**Adversarial tests:**

- `crates/corvid-cli/src/deploy_cmd.rs::deploy_attestation_marks_chain_incomplete_without_cdylib`

#### `deploy.sbom_completeness`
- **class**: runtime_checked
- **phase**: platform

`corvid deploy package` emits an SPDX 2.3 JSON SBOM (`sbom.spdx.json`) naming the app's Corvid source (by SHA-256) and the Corvid runtime the image links against, with the relationship between them declared. A future slice expands this to enumerate every transitively-linked Rust dependency via `cargo metadata` — the full-dep-enumeration completeness check tracks separately at the dep-enumeration registry row that lands when 43V wires `cargo metadata` into the SBOM.

**Positive tests:**

- `crates/corvid-cli/src/deploy_cmd.rs::deploy_sbom_is_structurally_valid_spdx_2_3`
- `crates/corvid-cli/src/deploy_cmd.rs::deploy_sbom_names_app_source_and_corvid_runtime`

**Adversarial tests:**

- `crates/corvid-cli/src/deploy_cmd.rs::deploy_sbom_names_app_source_and_corvid_runtime`

### Release channels

#### `release.signed_artifact`
- **class**: runtime_checked
- **phase**: platform

Every artifact emitted by `corvid release nightly/beta/stable` is signed with the release key + paired with a `SHA256SUMS.txt` whose contents the user can verify with `sha256sum -c`. The signed manifest is a DSSE envelope over the release contents, with payload type `application/vnd.corvid.release.manifest.v1+json`. The channel + version pair must satisfy the channel's naming convention (`-nightly.` / `-beta.` / plain MAJOR.MINOR.PATCH) — a stable-shaped version cannot be published to the nightly channel and vice versa.

**Positive tests:**

- `crates/corvid-cli/src/release_cmd.rs::release_validate_version_accepts_each_channel_shape`
- `crates/corvid-cli/src/release_cmd.rs::sign_release_manifest_emits_v1_payload_type`

**Adversarial tests:**

- `crates/corvid-cli/src/release_cmd.rs::release_validate_version_refuses_channel_version_mismatch`

### Upgrade compatibility

#### `upgrade.claim_regression_check`
- **class**: runtime_checked
- **phase**: platform

`corvid upgrade check --claims-current <path> --claims-target <path>` compares two claim manifests and refuses (exit 1) if the upgrade target removes any registered guarantee id OR downgrades any class (Static → RuntimeChecked / OutOfScope, RuntimeChecked → OutOfScope). Upgrades (OutOfScope → RuntimeChecked, etc.) are NOT regressions. The two manifests are JSON arrays of `{id, class}` rows the operator produces via `corvid claim --explain --json <cdylib>` against the current and target binaries. The `--json` mode of `claim --explain` itself lands as a sibling launch- readiness slice — the comparison + rejection is what this row promises.

**Positive tests:**

- `crates/corvid-cli/src/upgrade_cmd.rs::claim_regression_check_passes_when_manifests_match`

**Adversarial tests:**

- `crates/corvid-cli/src/upgrade_cmd.rs::claim_regression_check_flags_removed_guarantee`
- `crates/corvid-cli/src/upgrade_cmd.rs::claim_regression_check_flags_class_downgrades_only`
- `crates/corvid-cli/src/upgrade_cmd.rs::upgrade_check_refuses_unpaired_claim_manifest_flag`

### Live ops introspection

#### `ops.live_introspection_signed`
- **class**: out_of_scope
- **phase**: runtime

`corvid ops show <prod-url>` returns the live binary's signed claim manifest + cost-since-start + approvals- pending. The response is signed by the binary's signing key (matching the cdylib's DSSE envelope key); a response whose signature doesn't match the expected key means either a man-in-the-middle or the wrong binary is running at the URL.

> **Why out of scope:** `corvid ops show` CLI subcommand does not exist yet (verified by `corvid ops --help` → unrecognised). The Phase 36-generated axum server has no `/__ops` introspection endpoint. Filed as `43P-ops-show` — promotes this row to RuntimeChecked when both the CLI + the server endpoint ship + the signature-match test lands.

### Claim audit

#### `claim.audit_runnable_artifacts`
- **class**: runtime_checked
- **phase**: platform

Every claim listed in `docs/meta/launch-claim-audit.md` points at either a runnable command (backticked code), a linked artifact (`[link]`-style markdown), or an explicit `blocked` / `non-scope` status. `corvid claim audit` exits 0 only when every claim has evidence; aspirational wording flagged at audit time fails the check unless the row carries an explicit blocked/non-scope status.

**Positive tests:**

- `crates/corvid-cli/src/claim_cmd.rs::audit_passes_when_every_claim_resolves`

**Adversarial tests:**

- `crates/corvid-cli/src/claim_cmd.rs::audit_fails_when_a_claim_lacks_evidence`

### Platform — explicit non-defenses

#### `platform.host_kernel_compromise`
- **class**: out_of_scope
- **phase**: platform

Defending against a compromised host kernel or privileged-process tampering with the running Corvid binary's memory.

> **Why out of scope:** Outside Corvid's trust boundary — a kernel that can rewrite user-space memory can defeat any user-space invariant. The security model assumes a non-malicious kernel; otherwise the host is responsible.

#### `platform.signing_key_compromise`
- **class**: out_of_scope
- **phase**: platform

Defending against compromise of the ed25519 signing key used to attest a cdylib or sign a receipt.

> **Why out of scope:** Key management is a host responsibility. Corvid signs and verifies; rotating, revoking, and protecting keys is outside the language's scope and explicitly delegated to the host's key-management practice.

#### `platform.toolchain_compromise`
- **class**: out_of_scope
- **phase**: platform

Defending against a compromised Rust toolchain, Cranelift release, or system linker producing a Corvid binary that does not match its source.

> **Why out of scope:** Reproducible builds across heterogeneous toolchains are a post-v1.0 hardening goal. Today Corvid trusts the rustc and Cranelift releases the user installs; the bilateral verifier (Slice 35-H) is the closest approximation of toolchain-independence available pre-v1.0.

#### `package.hosted_registry_available`
- **class**: out_of_scope
- **phase**: platform

A Corvid-operated public package registry service that serves the published index format and source artifacts.

> **Why out of scope:** No hosted Corvid package registry service runs yet; The CLI ships the published index format + signed-publish tooling (`corvid package publish`, `verify-registry`, `verify-lock`) and `--url-base` accepts file:// and any http endpoint a user runs themselves. A hosted public registry is post-v1.0 work; see `docs/internals/package-manager-scope.md` for the full boundary.

## Updating this document

This file is generated. To change a description, add a new guarantee, or move an entry between `static` /
`runtime_checked` / `out_of_scope`, edit `crates/corvid-guarantees/src/lib.rs` and run:

```
cargo run -q -p corvid-cli -- contract regen-doc docs/reference/core-semantics.md
```

Then commit the regenerated file together with the registry change. CI fails if the committed text drifts from the registry — there is no quiet way to evolve the spec away from the implementation.
