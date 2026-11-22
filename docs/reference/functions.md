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

- `fn text(value: String) -> View`
- `fn stack(content: List<View>) -> View`
- `fn row(content: List<View>) -> View`
- `fn grid(content: List<View>) -> View`
- `fn text_field(label: String, name: String, initial: String) -> View`
- `fn submit(label: String) -> View`
- `fn search_form(controls: List<View>) -> View`
- `fn disclosure(label: String, content: List<View>) -> View`
- `fn page(title: String, content: List<View>) -> String`
- `fn action(label: String, handler: String) -> View`
- `fn assistant_action(label: String, handler: String, name: String, description: String) -> View` opts an integer-state button into WebMCP. Use a unique tool name per page.
- `fn assistant_text_state(label: String, initial: String, handler: String, name: String, description: String) -> View` exposes a single-line text control. Its tool requires one string field named for the handler parameter and returns the handler's string result.
- `fn int_state(label: String, initial: Int, controls: List<View>) -> View`
- `fn shared_int_state(name: String, label: String, initial: Int, controls: List<View>) -> View`
- `fn state_output(name: String, label: String) -> View`
- `fn interactive_page(title: String, content: List<View>) -> String`
- `fn text_state(label: String, initial: String, handler: String) -> View`
- `fn query_form(path: String, controls: List<View>) -> View`
- `fn agent_query_form(name: String, description: String, path: String, controls: List<View>) -> View`
- `fn link(label: String, path: String) -> View`
- `fn stream_text(label: String, path: String) -> View`
