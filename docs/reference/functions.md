# Corvid function reference

Look up a method by receiver type or a standard-library call by module. Each signature gives the argument order and return type. For example, `List<T>.first() -> Option<T>` means that calling `names.first()` on a `List<String>` returns an `Option<String>`.

For worked examples, see [Types and collections](../book/05-types.md), [Pattern matching](../book/13-pattern-matching.md), and the [standard library guide](stdlib/README.md).

## String

- `String.length() -> Int`
- `String.to_upper() -> String`
- `String.to_lower() -> String`
- `String.trim() -> String`
- `String.contains(needle: String) -> Bool`
- `String.starts_with(prefix: String) -> Bool`
- `String.ends_with(suffix: String) -> Bool`
- `String.split(sep: String) -> List<String>`
- `String.replace(from: String, to: String) -> String`
- `String.substring(start: Int, end: Int) -> String`
- `String.parse_int() -> Result<Int, String>`
- `String.parse_float() -> Result<Float, String>`

## Int

- `Int.to_string() -> String`
- `Int.to_float() -> Float`
- `Int.abs() -> Int`
- `Int.min(other: Int) -> Int`
- `Int.max(other: Int) -> Int`
- `Int.pow(exp: Int) -> Int`

## Float

- `Float.to_string() -> String`
- `Float.to_int_truncated() -> Int`
- `Float.abs() -> Float`
- `Float.min(other: Float) -> Float`
- `Float.max(other: Float) -> Float`
- `Float.pow(exp: Float) -> Float`
- `Float.sqrt() -> Float`
- `Float.floor() -> Int`
- `Float.ceil() -> Int`
- `Float.round() -> Int`

## Bool

- `Bool.to_string() -> String`

## List

- `List<T>.length() -> Int`
- `List<T>.append(item: T) -> Nothing`
- `List<T>.contains(item: T) -> Bool`
- `List<T>.first() -> Option<T>`
- `List<T>.last() -> Option<T>`
- `List<T>.slice(start: Int, end: Int) -> List<T>`
- `List<T>.reverse() -> Nothing`
- `List<T>.sort() -> Nothing`
- `List<String>.join(sep: String) -> String`
- `List<T>.map(f: (T) -> U) -> List<U>`
- `List<T>.filter(pred: (T) -> Bool) -> List<T>`
- `List<T>.fold(init: U, f: (U, T) -> U) -> U`
- `List<T>.any(pred: (T) -> Bool) -> Bool`
- `List<T>.all(pred: (T) -> Bool) -> Bool`

## Map

- `Map<K,V>.length() -> Int`
- `Map<K,V>.get(key: K) -> Option<V>`
- `Map<K,V>.contains_key(key: K) -> Bool`
- `Map<K,V>.keys() -> List<K>`
- `Map<K,V>.values() -> List<V>`
- `Map<K,V>.remove(key: K) -> Option<V>`

## Option

- `Option<T>.unwrap_or(default: T) -> T`
- `Option<T>.is_some() -> Bool`
- `Option<T>.is_none() -> Bool`
- `Option<T>.ok_or(err: E) -> Result<T, E>`

## Result

- `Result<T, E>.unwrap_or(default: T) -> T`
- `Result<T, E>.is_ok() -> Bool`
- `Result<T, E>.is_err() -> Bool`
- `Result<T, E>.map_err(f: (E) -> F) -> Result<T, F>`

## Built-in functions

- `range(start: Int, end: Int) -> List<Int>`
- `form_for(target: String, initial: Record, submit_label: String) -> String`
- `Ok(value: T) -> Result<T, E>`
- `Err(error: E) -> Result<T, E>`
- `Some(value: T) -> Option<T>`
- `Weak::new(value: T) -> Weak<T>`
- `Weak::upgrade(reference: Weak<T>) -> Option<T>`
- `merge(streams: List<Stream<T>>) -> Stream<T>`
- `resume(streaming_prompt, token: ResumeToken<T>) -> Stream<T>`
- `resume_token(stream: Stream<T>) -> ResumeToken<T>`
- `ask(question: String, T) -> T`
- `choose(options: List<T>) -> T`
- `Page(items: List<T>, next_cursor: Option<String>) -> Page<T>`

Use `None` for an absent option. Pass a type name as the second argument to `ask`, for example `ask("How many?", Int)`. `Weak::new` accepts strings, records and lists. `resume` takes the name of a streaming prompt.

## Upload

- `Upload<Format>.text() -> String`
- `Upload<Format>.bytes() -> List<Int>`
- `Upload<Format>.filename() -> String`
- `Upload<Format>.content_type() -> String`
- `Upload<Format>.size() -> Int`

## Every type

- `T.from_json(text: String) -> Result<T, JsonError>`
- `T.from_json_strict(text: String) -> Result<T, JsonError>`
- `T.to_json() -> String`

Call `from_json` on a type name, such as `Order.from_json(text)`. Use a type alias to decode a list or map, such as `type Orders = List<Order>`. A `JsonError` has the `String` fields `path`, `expected`, `found` and `message`. See the [public JSON mapping](stdlib/json.md#public-json-mapping).

## Grounded

- `Grounded<T>.unwrap_discarding_sources() -> T`

Returns the underlying value and discards its source information.

## Standard library

Import the module that declares the call. A tool's `uses` clause lists its declared effects. Each module heading links to its source.

### [std/agent](../../std/agent.cor)

- `agent classify(label: String, confidence: Float, threshold: Float) -> Classification`
- `agent extraction_ok(schema_name: String, value: String) -> Extraction`
- `agent extraction_error(schema_name: String, error_message: String) -> Extraction`
- `agent rank(item_id: String, score: Float, reason: String) -> Ranking`
- `agent judge(winner: String, rationale: String, needs_human: Bool) -> Judgement`
- `agent plan_step(id: String, instruction: String, effect_summary: String) -> PlanStep`
- `agent tool_use(tool_name: String, argument_summary: String, approval_label: String, approved: Bool) -> ToolUse`
- `agent critique(accepted: Bool, finding: String, revision_hint: String) -> Critique`
- `agent summarize(text: String, style: String, target_audience: String) -> Summary`
- `agent route_decision(target: String, reason: String, needs_human: Bool) -> RouteDecision`
- `agent action_request(action_name: String, argument_summary: String, approval_label: String, ready: Bool) -> ActionRequest`
- `agent review_verdict(accepted: Bool, severity: String, comment: String) -> ReviewVerdict`
- `agent tool_loop_turn(turn_id: String, thought: String, tool_name: String, action_summary: String, done: Bool) -> ToolLoopTurn`
- `agent answer_with_provenance(answer: String, provenance_key: String) -> AnswerWithProvenance`

### [std/ai](../../std/ai.cor)

- `agent system_message(content: String) -> AiMessage`
- `agent user_message(content: String) -> AiMessage`
- `agent assistant_message(content: String) -> AiMessage`
- `agent start_session(id: String, title: String) -> AiSession`
- `agent next_turn(current: AiSession) -> AiSession`
- `agent tool_ok(tool_name: String, value: String) -> ToolResultEnvelope`
- `agent tool_error(tool_name: String, error_message: String) -> ToolResultEnvelope`
- `agent route_to(model_name: String, provider: String, reason: String) -> ModelRoute`
- `agent validation_ok(schema: String) -> StructuredValidation`
- `agent validation_error(schema: String, error_message: String) -> StructuredValidation`
- `agent confidence(score: Float, threshold: Float) -> Confidence`
- `agent trace_event(kind: String, name: String, detail: String) -> TraceEventSummary`
- `agent render_prompt_pair(label: String, value: String) -> String`
- `agent render_prompt_section(title: String, body: String) -> String`
- `agent render_message(message: AiMessage) -> String`
- `agent rendered_prompt(template_name: String, rendered: String) -> PromptRender`

### [std/approvals](../../std/approvals.cor)

- `agent approval_contract_ref(id: String, version: String, action: String, target_kind: String, target_id: String, tenant_id: String, required_role: String, max_cost_usd: Float, data_class: String, irreversible: Bool, expires_ms: Int, replay_key: String) -> ApprovalContractRef`
- `agent approval_queue_item(id: String, tenant_id: String, requester_actor_id: String, contract: ApprovalContractRef, risk_level: String, trace_id: String) -> ApprovalQueueItem`
- `agent approval_audit_envelope(id: String, approval_id: String, tenant_id: String, actor_id: String, event_kind: String, status_before: String, status_after: String, redacted_reason: String, trace_id: String, created_ms: Int) -> ApprovalAuditEnvelope`
- `agent approval_pending(item: ApprovalQueueItem) -> Bool`
- `agent approval_same_tenant(item: ApprovalQueueItem, tenant_id: String) -> Bool`
- `agent approval_not_expired(item: ApprovalQueueItem, now_ms: Int) -> Bool`
- `agent approval_redacted(item: ApprovalQueueItem, audit: ApprovalAuditEnvelope) -> Bool`

### [std/auth](../../std/auth.cor)

- `agent actor(id: String, tenant_id: String, display_name: String, actor_kind: String, auth_method: String, assurance_level: String, role_fingerprint: String, permission_fingerprint: String, trace_id: String) -> Actor`
- `agent user_actor(id: String, tenant_id: String, display_name: String, role_fingerprint: String, permission_fingerprint: String, trace_id: String) -> Actor`
- `agent service_actor(id: String, tenant_id: String, display_name: String, scope_fingerprint: String, trace_id: String) -> Actor`
- `agent session_ref(id: String, actor_id: String, tenant_id: String, issued_ms: Int, expires_ms: Int, rotation_counter: Int, csrf_binding_id: String, trace_id: String) -> SessionRef`
- `agent api_key_ref(id: String, service_actor_id: String, tenant_id: String, scope_fingerprint: String, hash_algorithm: String, expires_ms: Int, last_used_ms: Int, trace_id: String) -> ApiKeyRef`
- `agent auth_trace_for_session(actor: Actor, sess: SessionRef, replay_key: String) -> AuthTraceContext`
- `agent auth_trace_for_api_key(actor: Actor, key: ApiKeyRef, replay_key: String) -> AuthTraceContext`
- `agent allow_permission(actor: Actor, trace: AuthTraceContext, permission: String) -> AuthDecision`
- `agent deny_permission(actor: Actor, trace: AuthTraceContext, permission: String, reason: String) -> AuthDecision`
- `agent tenant(id: String, display_name: String, plan: String, lifecycle_state: String, data_boundary: String) -> Tenant`
- `agent role(tenant_id: String, name: String, permission_fingerprint: String) -> Role`
- `agent permission(name: String, resource_kind: String, action: String, risk_level: String) -> Permission`
- `agent actor_role_binding(tenant_id: String, actor_id: String, role_name: String, granted_by: String, expires_ms: Int) -> ActorRoleBinding`
- `agent actor_permission_set(actor: Actor, role_fingerprint: String, permission_fingerprint: String, resolved_ms: Int) -> ActorPermissionSet`
- `agent tenant_active(t: Tenant) -> Bool`
- `agent role_binding_active(binding: ActorRoleBinding, now_ms: Int) -> Bool`
- `agent permission_set_same_actor(set: ActorPermissionSet, actor: Actor) -> Bool`
- `agent actor_same_tenant(actor: Actor, tenant_id: String) -> Bool`
- `agent session_active(sess: SessionRef, now_ms: Int) -> Bool`
- `agent api_key_active(key: ApiKeyRef, now_ms: Int) -> Bool`
- `agent auth_secret_redacted(sess: SessionRef, key: ApiKeyRef) -> Bool`
- `agent auth_trace_same_tenant(trace: AuthTraceContext, actor: Actor) -> Bool`
- `agent jwt_contract(issuer: String, audience: String, jwks_url: String, algorithm: String, required_tenant_claim: String, required_subject_claim: String, clock_skew_ms: Int, replay_key: String) -> JwtVerificationContract`
- `agent jwt_verified(contract: JwtVerificationContract, subject: String, tenant_id: String, key_id: String, expires_ms: Int, not_before_ms: Int) -> JwtVerificationResult`
- `agent jwt_denied(contract: JwtVerificationContract, failure_kind: String) -> JwtVerificationResult`
- `agent jwt_contract_safe(contract: JwtVerificationContract) -> Bool`
- `agent jwt_diagnostic_redacted(result: JwtVerificationResult) -> Bool`
- `agent oauth_state(provider: String, tenant_id: String, actor_id: String, state_fingerprint: String, pkce_verifier_ref: String, nonce_fingerprint: String, expires_ms: Int, replay_key: String) -> OAuthStateRef`
- `agent oauth_token_ref(provider: String, tenant_id: String, actor_id: String, token_ref: String, scope_fingerprint: String, ciphertext_hash: String, expires_ms: Int, replay_key: String) -> OAuthTokenRef`
- `agent oauth_state_replay_safe(state: OAuthStateRef) -> Bool`
- `agent oauth_token_redacted(token: OAuthTokenRef) -> Bool`

### [std/browser](../../std/browser.cor)

- `tool browser_open_scoped(width: Int, height: Int) -> Nothing uses browser_interaction`
- `tool browser_read_text(selector: String) -> String uses browser_interaction`
- `tool browser_click(button_name: String) -> Nothing uses browser_interaction`
- `tool browser_capture() -> String uses browser_interaction`
- `tool browser_close() -> Nothing uses browser_interaction`

### [std/cache](../../std/cache.cor)

- `tool cache_put(namespace: String, subject: String, value: String, invalidation_key: String, provenance_key: String) -> Result<CacheEntryEnvelope, String> uses cache_write`
- `tool cache_get(namespace: String, subject: String) -> Result<CacheLookupEnvelope, String> uses cache_read`
- `tool cache_invalidate(invalidation_key: String) -> Result<Int, String> uses cache_write`
- `tool cache_invalidate_provenance(provenance_key: String) -> Result<Int, String> uses cache_write`
- `agent cache_key(namespace: String, subject: String, fingerprint: String, effect_key: String, provenance_key: String) -> CacheKeyEnvelope`
- `agent cache_entry(key: CacheKeyEnvelope, replay_safe: Bool, invalidation_key: String) -> CacheEntryEnvelope`
- `agent cache_hit(entry: CacheEntryEnvelope) -> Bool`

### [std/db](../../std/db.cor)

- `agent sqlite_open(dsn: String, replay_key: String) -> DbConnection`
- `agent postgres_open(dsn: String, replay_key: String) -> DbConnection`
- `agent db_param(name: String, value_kind: String, redacted: Bool) -> DbParam`
- `agent db_param_int(value: Int) -> DbParam`
- `agent db_param_float(value: Float) -> DbParam`
- `agent db_param_text(value: String) -> DbParam`
- `agent db_param_bool(value: Bool) -> DbParam`
- `agent db_param_null() -> DbParam`
- `agent db_request_query(sql: String, param_count: Int, replay_key: String) -> DbQuery`
- `agent db_request_execute(sql: String, param_count: Int, replay_key: String) -> DbQuery`
- `agent db_result(rows_affected: Int, row_count: Int, replay_key: String) -> DbResult`
- `agent db_error(query_site: String, database_message: String) -> DbError`
- `agent db_parameterized(query: DbQuery) -> Bool`
- `agent db_column(name: String, expected_type: String, value_kind: String, present: Bool) -> DbColumn`
- `agent db_decode_ok(record_type: String) -> DbRowDecode`
- `agent db_decode_missing_column(record_type: String, field_path: String, expected_type: String) -> DbRowDecode`
- `agent db_decode_wrong_kind(record_type: String, field_path: String, expected_type: String, received_kind: String) -> DbRowDecode`
- `agent db_transaction(id: String, status: String, nested: Bool, replay_key: String) -> DbTransaction`
- `agent db_transaction_commit(tx: DbTransaction) -> DbTransaction`
- `agent db_transaction_rollback(tx: DbTransaction) -> DbTransaction`
- `agent db_transaction_nested_rejected(tx: DbTransaction) -> Bool`
- `agent db_audit_record(actor: String, action: String, subject: String, route_name: String, job_id: String, agent_name: String, prompt_version: String, model_id: String, tool_name: String, approval_state: String, cost_usd: Float, trace_id: String, replay_key: String) -> DbAuditRecord`
- `agent db_audit_approved(record: DbAuditRecord) -> Bool`
- `agent db_audit_write(record: DbAuditRecord, approval_required: Bool, value_redacted: Bool) -> DbAuditWrite`
- `agent db_audit_write_safe(write: DbAuditWrite) -> Bool`
- `agent db_token_ref(provider: String, account_id: String, token_id: String, key_id: String, replay_key: String) -> DbTokenRef`
- `agent db_encrypted_token(token_ref: DbTokenRef, ciphertext_hash: String, algorithm: String) -> DbEncryptedToken`
- `agent db_token_redacted(token_ref: DbTokenRef) -> Bool`
- `agent db_migration_status(driver: String, migration: String, checksum: String, status: String, drifted: Bool, replay_key: String) -> DbMigrationStatus`
- `agent db_migration_clean(status: DbMigrationStatus) -> Bool`
- `agent db_effect_tag(operation: String, replay_key: String) -> DbEffectTag`
- `agent db_effect_is_write(tag: DbEffectTag) -> Bool`
- `agent db_replay_summary(operation: String, driver: String, query_fingerprint: String, row_count: Int, rows_affected: Int, replay_key: String) -> DbReplaySummary`
- `agent db_replay_redacted(summary: DbReplaySummary) -> Bool`
- `tool db_open(path: String) -> Result<DbHandle, String> uses db_egress_open`
- `tool db_query(handle: DbHandle, sql: String, params: List<DbParam>) -> Result<List<DbResult>, String> uses db_egress_read`
- `tool db_execute(handle: DbHandle, sql: String, params: List<DbParam>) -> Result<DbResult, String> uses db_egress_write`
- `tool db_query_rows(handle: DbHandle, sql: String, params: List<DbParam>) -> Result<List<JsonValue>, String> uses db_egress_read`

### [std/effects](../../std/effects.cor)

- `agent effect_tag(name: String, capability: String, trust: String, data: String, replay_policy: String) -> EffectTag`
- `agent effect_budget(cost_usd: Float, latency_ms: Int, token_limit: Int) -> EffectBudget`
- `agent effect_envelope(effect_name: String, provenance_key: String, approval_label: String, cache_key: String, replay_key: String) -> EffectEnvelope`
- `agent replay_safe(effect_meta: EffectEnvelope) -> Bool`
- `agent approval_required(effect_meta: EffectEnvelope) -> Bool`

### [std/env](../../std/env.cor)

- `agent env_get(name: String) -> Result<Option<String>, String>`

### [std/http](../../std/http.cor)

- `agent http_request_get(url: String) -> HttpRequestEnvelope`
- `agent http_request_post_json(url: String, body: String) -> HttpRequestEnvelope`
- `agent http_with_retry(request: HttpRequestEnvelope, max_retries: Int) -> HttpRequestEnvelope`
- `agent http_with_timeout(request: HttpRequestEnvelope, timeout_ms: Int) -> HttpRequestEnvelope`
- `agent http_ok(response: HttpResponseEnvelope) -> Bool`
- `tool http_get(url: String) -> Result<HttpResponseEnvelope, String> uses http_egress_get`
- `tool http_post_json(url: String, body: String) -> Result<HttpResponseEnvelope, String> uses http_egress_post`

### [std/io](../../std/io.cor)

- `agent path(value: String) -> PathInfo`
- `agent file_read(path_value: String, contents: String, bytes: Int) -> FileReadEnvelope`
- `agent file_write(path_value: String, bytes: Int) -> FileWriteEnvelope`
- `agent directory_entry(path_value: String, name: String, is_dir: Bool) -> DirectoryEntryEnvelope`
- `tool io_read_text(path: String) -> Result<FileReadEnvelope, String> uses io_read`
- `tool io_write_text(path: String, content: String) -> Result<FileWriteEnvelope, String> uses io_write`
- `tool io_list_dir(path: String) -> Result<List<DirectoryEntryEnvelope>, String> uses io_list`
- `tool io_read_image(path: String) -> Result<PromptImage, String> uses io_read`

### [std/jobs](../../std/jobs.cor)

- `agent job_input(job_kind: String, input_fingerprint: String, schema_name: String) -> JobInputEnvelope`
- `agent job_output(output_fingerprint: String, output_kind: String) -> JobOutputEnvelope`
- `agent retry_policy(max_attempts: Int, backoff_kind: String, base_delay_ms: Int, max_delay_ms: Int, dead_letter_queue: String) -> JobRetryPolicy`
- `agent retry_policy_valid(policy: JobRetryPolicy) -> Bool`
- `agent dead_letter(job: JobStateEnvelope, failure_kind: String, failure_fingerprint: String) -> JobDeadLetterEnvelope`
- `agent dead_letter_redacted(letter: JobDeadLetterEnvelope) -> Bool`
- `agent job_state(id: String, queue_name: String, job_kind: String, status: String, attempts: Int, max_attempts: Int, budget_usd: Float, approval_required: Bool, idempotency_key: String, replay_key: String) -> JobStateEnvelope`
- `agent pending_job_state(id: String, queue_name: String, job_kind: String, max_attempts: Int, budget_usd: Float, approval_required: Bool, idempotency_key: String, replay_key: String) -> JobStateEnvelope`
- `agent leased_job_state(job: JobStateEnvelope) -> JobStateEnvelope`
- `agent running_job_state(job: JobStateEnvelope) -> JobStateEnvelope`
- `agent succeeded_job_state(job: JobStateEnvelope) -> JobStateEnvelope`
- `agent job_can_start(job: JobStateEnvelope) -> Bool`
- `agent job_needs_approval(job: JobStateEnvelope) -> Bool`
- `agent job_metadata_redacted(input: JobInputEnvelope, output: JobOutputEnvelope) -> Bool`

### [std/json](../../std/json.cor)

- `tool json_parse(text: String) -> Result<JsonValue, String> uses json_egress_read`
- `tool json_get_int(value: JsonValue, field: String) -> Result<Int, String> uses json_egress_read`
- `tool json_get_float(value: JsonValue, field: String) -> Result<Float, String> uses json_egress_read`
- `tool json_get_string(value: JsonValue, field: String) -> Result<String, String> uses json_egress_read`
- `tool json_get_bool(value: JsonValue, field: String) -> Result<Bool, String> uses json_egress_read`
- `tool json_get_object(value: JsonValue, field: String) -> Result<JsonValue, String> uses json_egress_read`
- `tool json_get_array(value: JsonValue, field: String) -> Result<List<JsonValue>, String> uses json_egress_read`
- `tool json_object_new() -> JsonBuilder uses json_egress_build`
- `tool json_object_set_int(builder: JsonBuilder, key: String, value: Int) -> JsonBuilder uses json_egress_build`
- `tool json_object_set_float(builder: JsonBuilder, key: String, value: Float) -> JsonBuilder uses json_egress_build`
- `tool json_object_set_string(builder: JsonBuilder, key: String, value: String) -> JsonBuilder uses json_egress_build`
- `tool json_object_set_bool(builder: JsonBuilder, key: String, value: Bool) -> JsonBuilder uses json_egress_build`
- `tool json_object_finish(builder: JsonBuilder) -> String uses json_egress_build`

### [std/mcp](../../std/mcp.cor)

- `tool mcp_call(server_name: String, tool_name: String, args_json: String) -> Result<String, String> uses mcp_egress`

### [std/observe](../../std/observe.cor)

- `agent metric_counter(name: String, value: Float, unit: String) -> MetricCounter`
- `agent cost_counter(llm_calls: Int, prompt_tokens: Int, completion_tokens: Int, total_tokens: Int, cost_usd: Float) -> CostCounter`
- `agent latency_histogram(name: String, p50_ms: Int, p95_ms: Int, p99_ms: Int) -> LatencyHistogram`
- `agent routing_decision(prompt_name: String, from_model: String, to_model: String, reason: String) -> RoutingDecision`
- `agent approval_summary(label: String, approved: Int, denied: Int) -> ApprovalSummary`
- `agent runtime_summary(llm_calls: Int, local_llm_calls: Int, total_tokens: Int, cost_usd: Float, provider_count: Int, degraded_provider_count: Int) -> RuntimeObservationSummary`

### [std/path](../../std/path.cor)

- `tool path_join(base: String, child: String) -> String`
- `tool path_parent(path: String) -> Option<String>`
- `tool path_filename(path: String) -> Option<String>`
- `tool path_extension(path: String) -> Option<String>`
- `tool path_is_absolute(path: String) -> Bool`

### [std/rag](../../std/rag.cor)

- `agent rag_document(id: String, source: String, media_type: String, text: String) -> RagDocumentEnvelope`
- `agent rag_chunk(doc_id: String, chunk_id: String, source: String, text: String, start_char: Int, end_char: Int, provenance_key: String) -> RagChunkEnvelope`
- `agent retrieved_chunk(chunk: RagChunkEnvelope, source_name: String) -> RetrievedRagChunkEnvelope`
- `agent openai_embedder(model_name: String) -> EmbedderEnvelope`
- `agent ollama_embedder(model_name: String, endpoint: String) -> EmbedderEnvelope`
- `agent chunk_is_grounded(chunk: RagChunkEnvelope) -> Bool`
- `tool rag_ingest(index_path: String, doc_id: String, source: String, text: String, chunk_chars: Int) -> Result<Int, String> uses rag_write`
- `tool rag_search(index_path: String, query: String, limit: Int) -> Result<List<RagChunkEnvelope>, String> uses rag_read`

### [std/random](../../std/random.cor)

- `tool random_float() -> Float uses rand_draw`
- `tool random_int(min: Int, max: Int) -> Int uses rand_draw`

### [std/secrets](../../std/secrets.cor)

- `tool secret_read(name: String) -> Result<SecretReadEnvelope, String> uses secrets_read`
- `agent secret_present(name: String, value: String) -> SecretReadEnvelope`
- `agent secret_missing(name: String) -> SecretReadEnvelope`

### [std/time](../../std/time.cor)

- `agent time_instant(epoch_ms: Int, iso: String) -> TimeInstant`
- `tool time_now_utc() -> TimeInstant uses time_wall`
- `tool time_monotonic_ms() -> Int uses time_monotonic`
- `tool time_parse_iso(text: String) -> Result<Int, String> uses time_wall`
- `tool time_format_iso(epoch_ms: Int) -> String uses time_wall`

### [std/ui](../../std/ui.cor)

The [trail guide example](../../examples/trail_guide/src/main.cor) combines the static view functions in a server-rendered document.

- `fn text(value: String) -> View`
- `fn fragment(content: List<View>) -> View` groups siblings into one view for a component slot. It concatenates each child's existing markup in order. Nested fragments add no element or spacing. An empty fragment renders an empty string.
- `fn portal(target_id: String, content: List<View>) -> View` renders children together at the call site. On an interactive page, mounting the portal moves their container to the matching portal target in the same document or shadow root. Children stay inline if the target is absent or inside the portal. Unmounting returns the container to its owner. Reconnecting the owner looks up the target again.
- `fn portal_target(target_id: String) -> View` renders a block destination with the supplied HTML ID. Give it a unique, nonempty ID. Multiple portals can append their content here in mount order.
- `browser lifecycle name() -> Nothing:` declares a public lifecycle callback in `src/events.cor`. Import and call `name()` in a rendering module to get a `ui.ElementCallback`. In the browser, tool calls from its body invoke same-named synchronous methods on its owned element using the existing Wasm host ABI.
- `fn owned_element(element: View, mount: ElementCallback, release: ElementCallback) -> View` gives one root element a resource lifetime on an interactive page. After connection, module loading and custom-element registration the wrapper invokes `mount`. On disconnection it runs `release` with the same browser module instance and element. Reconnection acquires again. A failed acquisition also attempts release, so release must handle partial acquisition. Errors appear in the wrapper's alert.
- `fn recover_view(result: Result<View, String>, render_error: (String) -> View) -> View` returns the child view on success. On error it calls `render_error` with the message and returns that view in the child's place. Use `ui.text` inside the callback to escape error text.
- `fn inline_text(value: String) -> View` escapes text into a view without adding a surrounding element.
- `fn isolated_text(value: String) -> View` renders escaped text inside a `bdi` element. The browser determines its direction from its content and isolates it from surrounding text.
- `fn preformatted_text(value: String) -> View` renders escaped text in a `pre` element with `dir="auto"`. It preserves spaces and line breaks, including a leading newline. The browser infers the block's writing direction from its text.
- `fn abbreviation(label: String, expansion: String) -> View` renders an `abbr` element with the label as its text and the expansion in its `title` attribute. Both strings are escaped.
- `fn data_value(value: String, content: List<View>) -> View` renders an HTML `data` element. Its `value` attribute holds the escaped machine-readable string. Its children supply the visible inline content in the given order.
- `fn time_value(datetime: String, content: List<View>) -> View` renders a `time` element with an escaped `datetime` attribute and the supplied inline views in order. The caller supplies a valid HTML date, time or duration string.
- `fn trusted_markup(html: String) -> View` inserts application-authored HTML verbatim at the view's position among its siblings. It performs no escaping or sanitization. Use `text` or `inline_text` for user input.
- `fn no_script(message: String) -> View` renders escaped text in a paragraph inside a body-level `noscript` element. Browsers display the paragraph when scripting is disabled.
- `fn paragraph(content: List<View>) -> View` renders a `p` containing the supplied views in order, without adding spaces. Supply inline content such as `inline_text`, links or abbreviations. An empty list renders `<p></p>`.
- `fn strong(content: List<View>) -> View` marks inline content as important with a `strong` element. It keeps child views in order without inserting spaces. Use `inline_text` for plain text.
- `h1`, `h2`, `h3`, `h4`, `h5`, and `h6` each take `(label: String) -> View` and render the corresponding HTML heading element with the label as escaped text.
- `fn navigation(label: String, content: List<View>) -> View` renders a native `nav` with an escaped `aria-label` and its child views in list order.
- `fn aside(label: String, content: List<View>) -> View` renders a native `aside` for supplementary content. It escapes the label for `aria-label` and renders the children in list order.
- `fn section(heading_id: String, title: String, content: List<View>) -> View` renders a `section` named by its opening `h2`. Supply a unique, nonempty ID without whitespace for the heading. The function escapes the ID in both `id` and `aria-labelledby`, escapes the heading text, and renders children in list order.
- `fn data_group(name: String, value: String, content: List<View>) -> Result<View, String>` returns a `div` with one `data-{name}` attribute, its escaped value and the children in list order. Pass the suffix only. Empty names and characters outside `a-z`, `0-9` and `-` return `Err`.
- `fn image(src: String, alt: String) -> View` renders an `img` with escaped `src` and `alt` attributes. Supply a trusted image URL and alternative text. An empty string emits `alt=""` for a decorative image.
- `fn image_with_dimensions(src: String, alt: String, width: Int, height: Int) -> View` renders the same image with integer width and height attributes in CSS pixels. Each dimension accepts zero. A negative value omits that attribute.
- `fn responsive_image(src: String, alt: String, srcset: String, sizes: String) -> View` renders an image with a fallback URL, alternative text and a native HTML candidate list. All attribute values are escaped. An empty `sizes` omits that attribute. An empty `srcset` produces the same markup as `image(src, alt)`. Supply trusted candidate URLs and valid HTML descriptors.
- `fn svg_graphic(label: String, view_box: String, content: List<View>) -> View` renders inline SVG with `role="img"`, an escaped `aria-label`, an escaped `viewBox` and `width="100%"`. Supply a meaningful label, a valid SVG view box and SVG children in paint order.
- `fn svg_path(path_data: String, transform: String) -> View` renders an SVG path with an escaped `d` attribute. Supply SVG path commands and an optional SVG transform list. An empty transform string omits the attribute.
- `fn figure(caption: String, content: List<View>) -> View` renders a native `figure` with the supplied children in order, followed by a `figcaption` containing the escaped caption text.
- `fn list_item(content: List<View>) -> View` wraps the children in an `li` in the supplied order.
- `fn numbered_list_item(value: Int, content: List<View>) -> View` renders an `li` with an integer `value` attribute and preserves child order. Use it inside an ordered list to set that item's number.
- `fn ordered_list(start: Int, items: List<View>) -> View` renders an `ol` with an integer `start` attribute. Pass list item views in display order.
- `fn unordered_list(items: List<View>) -> View` renders a `ul`. Pass list item views in display order.
- `fn table(caption: String, columns: List<String>, rows: List<List<View>>) -> View` renders a native table with an escaped caption and escaped column headers in a `thead`. Each inner list creates one body row, with one `td` per view in the supplied order. An empty row list produces an empty `tbody`.
- `fn table_with_row_headers(caption: String, columns: List<String>, rows: List<List<View>>) -> View` takes the same inputs as `table`. The first view in each nonempty body row becomes a `th` with `scope="row"`. Remaining views become `td` cells in order. An empty inner list produces an empty row.
- `fn stack(content: List<View>) -> View`
- `fn row(content: List<View>) -> View`
- `fn grid(content: List<View>) -> View`
- `fn text_field(label: String, name: String, initial: String) -> View`
- `fn required_combobox(label: String, name: String, list_id: String, suggestions: List<String>) -> View` renders a required native text field and an editable ARIA combobox. In an interactive page it filters by prefix after composition, supports pointer or keyboard selection, and associates validation feedback with the field. Selecting an option dispatches bubbling input and change events. The list ID and its derived option and error IDs must be unique within the document.
- `fn modal_dialog(id: String, opener: String, title: String, content: List<View>, close_label: String) -> View` renders a named native dialog with show-modal and close command buttons. Reserve the ID and its derived `-title` ID. Requires browser support for declarative dialog commands.
- `fn file_drop_field(label: String, name: String) -> View` renders a labeled multiple-file input. On interactive pages a file drop on its label populates the native file list and dispatches bubbling input and change events.
- `fn rich_text_editor(label: String, content: List<View>) -> View` renders a named multiline contenteditable textbox containing the supplied views. The browser manages selection and undo. Its contents are local to the DOM and are excluded from native form data.
- `fn text_area(label: String, name: String, initial: String) -> View` renders a labeled native multiline form field. It escapes the label, form name and initial text, and preserves an initial blank line when the browser parses the textarea.
- `fn text_area_with_requirement(label: String, name: String, initial: String, required: Bool) -> View` renders the same field with the HTML `required` attribute when `required` is true. Passing false omits the attribute.
- `fn text_area_with_rows(label: String, name: String, initial: String, rows: Int, required: Bool) -> View` renders the same multiline field with a `rows` attribute for positive counts. Zero and negative counts omit `rows`. The `required` flag controls native form validation independently.
- `fn checkbox_field(label: String, name: String, checked: Bool) -> View` renders a labeled native checkbox and escapes both strings. The Boolean sets the initial checked state. In a form, a checked box submits its name with the value `on`. An unchecked box contributes no entry.
- `fn checkbox_field_with_value(label: String, name: String, value: String, checked: Bool) -> View` renders a labeled checkbox with an explicit `value` attribute. It escapes all three strings. The Boolean sets the initial checked state. A checked box submits the supplied value, including an empty string. An unchecked box contributes no entry.
- `fn date_field(label: String, name: String, initial: String) -> View`
- `fn bind_field(field: View, component: View) -> View` synchronizes a native field with a custom element's string `value` on interactive pages. Removing the bound view releases its field, component and form reset listeners. Reattaching it copies the field's current value into the component and binds to the current form.
- `fn submit(label: String) -> View`
- `fn reset_button(label: String) -> View` renders a native `<button type="reset">` with an escaped text label. Activating it restores the controls in its form to their default values.
- `fn post_form(path: String, controls: List<View>) -> View`
- `fn agent_post_form(name: String, description: String, path: String, controls: List<View>) -> View`
- `fn option(value: String, label: String, selected: Bool) -> View`
- `fn option_group(label: String, options: List<View>) -> View` renders a native `optgroup` with an escaped label and the supplied option views in order. Pass the group as a child of either select helper.
- `fn select_field(label: String, name: String, options: List<View>) -> View` renders a labeled native select. It escapes the label and field name and preserves the order of options and option groups. Submitting the enclosing form sends the selected option’s value under the field name.
- `fn submit_on_change_select(label: String, name: String, options: List<View>) -> View`
- `fn keyed_item(key: String, label: String, content: List<View>) -> View`
- `fn keyed_list(label: String, items: List<View>) -> View`
- `fn quantity_field(label: String, name: String, initial: Int, handler: String, result_label: String, initial_result: Int) -> View`
- `fn search_form(controls: List<View>) -> View`
- `fn disclosure(label: String, content: List<View>) -> View`
- `fn disclosure_with_initial_state(label: String, initially_open: Bool, content: List<View>) -> View` renders a native `details` element. It emits the boolean `open` attribute only when `initially_open` is true. The escaped label appears in the first-child `summary`, followed by the content views in order.
- `fn fieldset(legend: String, controls: List<View>) -> View` renders a native `fieldset` with an escaped text `legend` as its first child, followed by the supplied views in order.
- `fn page(title: String, content: List<View>) -> String`
- `fn navigation_page(language: String, title: String, description: String, content: List<View>) -> String` escapes the document language, title and meta description and renders the views inside `main`. Links perform native document navigation. With scripting enabled, Back and Forward can restore focus by element ID or unique link destination while preserving browser scroll restoration.
- `fn redirect_page(language: String, title: String, destination: String) -> String` renders a document with an immediate HTML refresh and a visible link labeled with the title. Supply a trusted application URL including any required query string. The route serves HTML with the normal success status.
- `fn page_with_landmarks(language: String, title: String, header: List<View>, content: List<View>, footer: List<View>) -> String` renders `header`, `main` and `footer` as direct children of `body`. Each list renders in order, including an empty list which leaves its landmark empty. The language attribute and document title are HTML-escaped.
- `fn page_in_language(language: String, title: String, content: List<View>) -> String` renders a document with an escaped `lang` attribute on `html`. Supply a language tag such as `en` or `fr-CA`. The title is escaped and child views appear in order inside `main`.
- `fn localized_page(language: String, direction: String, title: String, trusted_css: String, content: List<View>) -> String` renders an interactive document with escaped root language and direction, application-authored CSS, a main landmark and the event module. Pass a BCP 47 language tag and HTML direction (`ltr`, `rtl` or `auto`).
- `fn localized_date(iso_date: String, fallback: String) -> View` renders a semantic time with escaped fallback text. At page startup the event module formats a valid YYYY-MM-DD calendar date using the nearest language and a long date format in UTC, preserving the day across time zones.
- `fn localized_quantity(amount: Int, unit: String, fallback: String) -> View` renders a data element with an integer value and escaped fallback text. At page startup the event module uses the nearest language and a browser Intl unit identifier such as `day` or `kilometer` to choose digits, grouping and plural unit names. Unsupported units and invalid language tags retain the fallback.
- `fn styled_page(language: String, title: String, trusted_css: String, content: List<View>) -> String` inserts application-authored CSS verbatim in a document-level style element. Place optional CSS `@import` rules before style rules. The browser resolves relative import URLs against the document's base URL. Keep user input out of the stylesheet.
- `fn class_group(classes: String, content: List<View>) -> View` renders a div with an escaped class attribute and its children in order. Supply space-separated CSS class names. Each wrapper participates in the normal CSS cascade and custom property inheritance.
- `fn action(label: String, handler: String) -> View`
- `fn assistant_action(label: String, handler: String, name: String, description: String) -> View`
- `fn int_state(label: String, initial: Int, controls: List<View>) -> View`
- `browser callback name(current: Int) -> Int:` declares a public integer state update in `src/events.cor`. Its indented body uses ordinary Corvid function statements supported by the browser compiler. Import the name into the rendering module and call `name()` to obtain a `ui.IntCallback` binding. Pass that value as an input to a component or button. The web build packages the function under its declared name.
- `fn callback_button(label: String, update: IntCallback) -> View` creates a button inside an integer state control. A click supplies that control's current integer to the bound browser callback and displays its return value.
- `fn shared_int_state(name: String, label: String, initial: Int, controls: List<View>) -> View`
- `fn state_output(name: String, label: String) -> View`
- `fn event_module() -> View`
- `fn interactive_page(title: String, content: List<View>) -> String`
- `fn text_state(label: String, initial: String, handler: String) -> View`
- `fn assistant_text_state(label: String, initial: String, handler: String, name: String, description: String) -> View`
- `fn query_form(path: String, controls: List<View>) -> View`
- `fn named_query_form(label: String, path: String, controls: List<View>) -> View` renders a native GET form with the escaped label in `aria-label` and the escaped application path in `action`. A nonempty descriptive label names the form landmark. Controls appear in list order.
- `fn agent_query_form(name: String, description: String, path: String, controls: List<View>) -> View`
- `fn link(label: String, path: String) -> View`
- `fn link_with_content(path: String, content: List<View>) -> View` renders a native anchor with an escaped `href` and its children in list order. Supply a trusted destination and inline views such as text, an abbreviation or an image. Keep other links and interactive controls outside the anchor.
- `fn stream_text(label: String, path: String) -> View` gives each mounted view its own stream connection. Removing the view closes its active connection and disables both controls. Mounting the same view again enables Start and preserves the last output until the next request.

`assistant_action` opts an integer-state button into WebMCP. Use a unique tool name per page. `assistant_text_state` exposes a single-line text control. Its tool requires one string field named for the handler parameter and returns the handler's string result.

## Drag components in std:ui

- `ui.assistant_drag(component: View, name: String, description: String) -> View` exposes one instance to a browser assistant under a unique name.
- `ui.drag_canvas(label: String, initial: Int, width: Int, height: Int, handler: String, draw: Bool) -> View` renders the canvas used by a component drag handler.
- `ui.drag_panel(label: String, initial: Int, minimum: Int, maximum: Int, handler: String, content: List<View>) -> View` renders the bounded panel used by a component drag handler.

For component syntax and coordinate semantics, see [Drawing and resizing with a Stroke](../book/19-browser-views.md#drawing-and-resizing-with-a-stroke).

## Record selections in std:ui

- `ui.slider(label: String, name: String, value: Int, minimum: Int, maximum: Int) -> View` creates a labelled native integer slider.
- `ui.selection(path: String, controls: View, preview: View) -> View` groups uniquely named sliders in a custom layout. A change posts their integer values to a same-origin Corvid form route. Its HTML fragment replaces the preview.
- `ui.assistant_selection(component: View, name: String, description: String) -> View` exposes one selection instance as a named browser tool. Its required integer fields and inclusive limits come from that instance's sliders. The server separately enforces the application's record type.
- `ui.color_sample(red: Int, green: Int, blue: Int) -> View` renders a decorative RGB swatch. Provide visible text in the surrounding view to identify the color.

See [Build a color picker from a record](../book/19-browser-views.md#build-a-color-picker-from-a-record).

## Browser-local record views in std:ui

- `ui.local_slider(label: String, name: String) -> View` binds a native slider to a record field. Supply one per field.
- `ui.list_editor(rows: View, new_row: View, add_label: String, remove_label: String) -> View` adds local insertion and removal to one keyed browser list. Render exactly one row with the same row function for the template. Its key is the prefix for newly generated string keys. The other fields supply the defaults. Edits and membership last until reload.
- `ui.assistant_reorder(component: View, name: String, description: String) -> View` exposes the move action for one enclosed browser list. Its inputs are a row key and a direction of `up` or `down`. For a list editor, `{}` reads the current keys in display order.
- The keyed child loop uses `ui.browser_row(key: String, key_field: String, fields: Map<String, List<String>>, content: List<View>) -> View` to mount a record editor and `ui.browser_rows(label: String, rows: List<View>) -> View` to group the rows.
- `ui.boolean_children(field: String, content: List<View>) -> View` shows its children while a browser boolean is true. Prefer `if settings.tools.web_search:` inside a child block for compile-time checking.
- `ui.choice_children(field: String, variant: String, content: List<View>) -> View` groups views whose visibility follows a browser choice. Prefer an indented `if settings.mode == Scheduled` child block so the compiler checks the field and variant.
- `ui.value(field: String) -> View` displays a browser field as live plain text inside its mounted record editor. Use `ui.value(bind settings.profile.name)` in a developer-written layout.
- `ui.record_text(pattern: String) -> View` renders text with named placeholders such as `{width}` or `{profile.name}`.
- `ui.record_swatch(red: String, green: String, blue: String) -> View` binds three named integer fields to an RGB sample.
- `ui.local_selection(state: RecordState, controls: View, preview: View) -> View` mounts one record editor in an interactive page. Field names in controls and preview bindings are resolved on mounting.
- `ui.derived_value(handler: String, fields: List<String>) -> View` displays the scalar return value of a compiled browser agent as plain text.
- `ui.conditional_view(predicate: String, fields: List<String>, content: List<View>) -> View` shows the children when the agent returns `true`. Hidden children retain their DOM state.
- `ui.projected_rows(label: String, count: String, key_at: String, fields: List<String>, row: List<View>) -> View` renders a labelled ordered list. The count agent returns a nonnegative `Int`. The key agent receives the same fields followed by a zero-based `Int` index. It returns a unique nonempty `String` or an `Int`. The renderer validates all keys before updating the list, reuses rows by key, and clones the row template for new keys.
- `ui.row_key() -> View` displays the enclosing projected row's key as plain text.

These projections read their nearest mounted record. Supply paths with `bind` and define the named agents in `src/events.cor`. On mount, the runtime checks each argument against its field type. Choices pass their variant name as a `String`. Synchronous record edits schedule one render per subscribed projection. Disconnecting a projection releases its subscription and cancels any pending render. Browser event agents can match a named integer or boolean local against literal and wildcard patterns with optional guards and an `Int`, `Bool` or `Float` result. [Reading plan walkthrough](../book/19-browser-views.md#derive-two-views-from-a-reading-plan)

- `browser state name = initial` obtains a `ui.RecordState` from a record whose editable leaves are text, booleans, named choices, or bounded integers. Use `bind settings.tools.web_search` to bind a nested leaf. A list initializer retains its record element type for keyed rows.
- `ui.checkbox(label: String, field: String) -> View` creates a native checkbox. Use `bind state.enabled` for a checked boolean field.
- `ui.segmented_choice(label: String, field: String) -> View` displays a named choice as a labelled group of native radio buttons. Use the arrow keys to select adjacent options. Each mounted group uses its own radio name.
- `ui.field(label: String, name: String) -> View` places a labelled native control in your layout. At mount time the declared type selects a text area, checkbox, choice list, or slider.
- `ui.record_state(bounds: Map<String, List<Int>>) -> RecordState` builds an integer-only descriptor from initial value, minimum, and maximum triples. The mixed binding uses the lower-level `ui.browser_record(rows: Map<String, List<String>>) -> RecordState` adapter. Prefer the binding syntax for application code.
- The raw descriptor constructor takes `ui.RecordState(fields: List<IntegerField>, descriptors: List<String>)`. Code that directly constructed an integer-only descriptor can pass `[]` as the second argument.
- Existing `local type T` declarations retain their file-local `T_state(value: T) -> ui.RecordState` helper.

See [Keep a record in browser memory](../book/19-browser-views.md#keep-a-record-in-browser-memory).

The earlier names `ui.local_field` and `ui.local_checkbox` are supported aliases for `ui.field` and `ui.checkbox`.

### JavaScript module fields

`ui.module_field(control: View, module_path: String, reader: String, handler: TextCallback) -> View` constructs the root view for a custom element backed by a JavaScript module. Supply a labelled control that emits a bubbling `change` event. A `browser callback name(text: String) -> String` declaration provides the rendering function `name() -> ui.TextCallback`. Return the view's HTML from the component entry and export it with `scripts/export_browser_component.py`. The generated `componentElement(html)` recognizes this root and returns a custom element constructor. The host registers the tag name.

On change the browser imports `module_path` relative to the generated events module, or resolves a bare specifier through the host's import map. The named export implements the generated TypeScript type `CorvidFieldReader = (value: string | File | null, convert: (text: string) => string) => string | Promise<string>`. The changing control supplies its string value. A native file input supplies its first file or `null` when cleared. The `convert` callback invokes the named Corvid `(String) -> String` event handler. The example's `readNote(file, importedText)` decodes gzip bytes as UTF-8 and returns the callback result.

The element exposes a writable string `value`, a named `hint` slot and a bubbling, composed `change` event on successful import. Property writes update the displayed value silently. A newer selection, a property write or disconnect supersedes an outstanding read. Synchronous exceptions and rejected promises leave the value intact and appear in the element's alert with the reader name and the declared callback's Corvid file, line and column. [Learning guide and example](../book/19-browser-views.md#read-a-compressed-note-with-a-typed-javascript-package)

### Browser API extension bindings

Run `python scripts/derive_browser_bindings.py INPUT.webidl OUTPUT.d.ts` to derive TypeScript interfaces for a frontend extension. Each `partial interface Name` produces an exported `NameBinding`. The generator supports operations returning `Promise<T>` with zero or more required, named scalar or buffer arguments. Its type mappings are `ArrayBuffer` to `ArrayBuffer`, `DOMString` and `USVString` to `string`, `boolean` to `boolean` and an `undefined` result to `void`. Unsupported operations or types fail generation. For other Web IDL constructs, use the browser declarations in TypeScript's DOM or WebWorker library, or supply a local structural declaration for a newer API.

In the file preview, `BlobBinding.arrayBuffer() -> Promise<ArrayBuffer>` reads a selected file and `ClipboardBinding.writeText(data: string) -> Promise<void>` writes the preview on user activation. Check each operation before calling it and handle exceptions or rejected promises. The DOM library supplies the observer callback, nullable file selection and worker transfer-list signatures. The worker receives ownership of the buffer passed in the transfer list. [IDL and adapter](../../examples/file_preview/file_access.webidl) · [Worker](../../examples/file_preview/decode_file.ts)

The example's `open_preview() -> ui.ElementCallback` and `close_preview() -> ui.ElementCallback` bindings invoke the element's synchronous `start_preview() -> Nothing` and `stop_preview() -> Nothing` methods through `ui.owned_element`. Setup installs per-element subscriptions. Release cancels those subscriptions and any live worker. Use the same lifecycle interface to own another browser extension. [Learning guide](../book/19-browser-views.md#use-a-browser-api-newer-than-corvid)

### Landscape editor viewport

The scene editor builds three example agents for its browser worker. The generated JavaScript loader accepts Corvid `Int` arguments as `bigint`.

| Example agent | Result |
| --- | --- |
| `screen_position(world: Int, scale: Int, offset: Int) -> Int` | CSS pixel coordinate along one axis, equal to `world * scale + offset`. |
| `contains_pixel(pixel: Int, start: Int, size: Int) -> Bool` | True from `start` up to, and excluding, `start + size`. |
| `zoom_level(current: Int, change: Int) -> Int` | Sum of the current level and signed change, clamped to 1 through 4 inclusive. |

[Build and use the landscape editor](../book/19-browser-views.md#edit-landscape-tiles-with-a-webgl-canvas)
