use super::*;

/// Public Corvid guarantee id this enqueue path enforces:
/// `jobs.idempotency_key_uniqueness`.
///
/// Across N concurrent workers, exactly one durable queue row
/// exists for a given non-null idempotency key. Enforced by a
/// partial UNIQUE INDEX on
/// `queue_jobs(idempotency_key) WHERE idempotency_key IS NOT NULL`
/// in the SQLite schema (see `super::sqlite_init`), plus the
/// `enqueue_typed_idempotent` collision-fallback path that returns
/// the surviving row when the insert hits the UNIQUE constraint.
/// Tagged at module level so the `corvid-guarantees` inverse-
/// coverage sentinel can confirm the enforcement site is wired
/// to the registry row.
pub const GUARANTEE_ID_IDEMPOTENCY_KEY_UNIQUENESS: &str = "jobs.idempotency_key_uniqueness";

impl DurableQueueRuntime {
    /// Stable id of the public Corvid guarantee the enqueue path
    /// enforces — see `GUARANTEE_ID_IDEMPOTENCY_KEY_UNIQUENESS`
    /// for the contract description.
    pub const fn enqueue_guarantee_id() -> &'static str {
        GUARANTEE_ID_IDEMPOTENCY_KEY_UNIQUENESS
    }
}

impl DurableQueueRuntime {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, RuntimeError> {
        let conn = Connection::open(path.as_ref()).map_err(|err| {
            RuntimeError::Other(format!(
                "failed to open durable queue `{}`: {err}",
                path.as_ref().display()
            ))
        })?;
        let runtime = Self {
            next_id: AtomicU64::new(0),
            conn: Mutex::new(conn),
        };
        runtime.init()?;
        runtime.seed_next_id()?;
        Ok(runtime)
    }

    pub fn open_in_memory() -> Result<Self, RuntimeError> {
        let conn = Connection::open_in_memory()
            .map_err(|err| RuntimeError::Other(format!("failed to open durable queue: {err}")))?;
        let runtime = Self {
            next_id: AtomicU64::new(0),
            conn: Mutex::new(conn),
        };
        runtime.init()?;
        Ok(runtime)
    }

    pub fn enqueue(
        &self,
        task: impl Into<String>,
        payload: Value,
        max_retries: u64,
        budget_usd: f64,
        effect_summary: Option<String>,
        replay_key: Option<String>,
    ) -> Result<QueueJob, RuntimeError> {
        self.enqueue_typed(
            task,
            payload,
            None,
            max_retries,
            budget_usd,
            effect_summary,
            replay_key,
        )
    }

    pub fn enqueue_typed(
        &self,
        task: impl Into<String>,
        payload: Value,
        input_schema: Option<String>,
        max_retries: u64,
        budget_usd: f64,
        effect_summary: Option<String>,
        replay_key: Option<String>,
    ) -> Result<QueueJob, RuntimeError> {
        self.enqueue_typed_at(
            task,
            payload,
            input_schema,
            max_retries,
            budget_usd,
            effect_summary,
            replay_key,
            None,
        )
    }

    pub fn enqueue_typed_at(
        &self,
        task: impl Into<String>,
        payload: Value,
        input_schema: Option<String>,
        max_retries: u64,
        budget_usd: f64,
        effect_summary: Option<String>,
        replay_key: Option<String>,
        next_run_ms: Option<u64>,
    ) -> Result<QueueJob, RuntimeError> {
        let task = task.into();
        if task.trim().is_empty() {
            return Err(RuntimeError::Other(
                "std.queue task name must not be empty".to_string(),
            ));
        }
        let now = now_ms();
        let id = format!(
            "job_{}",
            self.next_id
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1)
        );
        let budget_usd = if budget_usd.is_finite() && budget_usd > 0.0 {
            budget_usd
        } else {
            0.0
        };
        let job = QueueJob {
            id: id.clone(),
            task,
            payload,
            input_schema,
            status: QueueJobStatus::Pending,
            attempts: 0,
            max_retries,
            budget_usd,
            effect_summary,
            replay_key,
            idempotency_key: None,
            output_kind: None,
            output_fingerprint: None,
            failure_kind: None,
            failure_fingerprint: None,
            next_run_ms,
            lease_owner: None,
            lease_expires_ms: None,
            approval_id: None,
            approval_expires_ms: None,
            approval_reason: None,
            created_ms: now,
            updated_ms: now,
        };
        let payload_json = serde_json::to_string(&job.payload).map_err(|err| {
            RuntimeError::Other(format!("failed to serialize durable queue payload: {err}"))
        })?;
        self.conn
            .lock()
            .unwrap()
            .execute(
                "insert into queue_jobs
                 (id, task, payload_json, input_schema, status, attempts, max_retries, budget_usd, effect_summary, replay_key, idempotency_key, output_kind, output_fingerprint, failure_kind, failure_fingerprint, next_run_ms, lease_owner, lease_expires_ms, approval_id, approval_expires_ms, approval_reason, created_ms, updated_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                params![
                    job.id,
                    job.task,
                    payload_json,
                    job.input_schema,
                    job.status.as_str(),
                    job.attempts as i64,
                    job.max_retries as i64,
                    job.budget_usd,
                    job.effect_summary,
                    job.replay_key,
                    job.idempotency_key,
                    job.output_kind,
                    job.output_fingerprint,
                    job.failure_kind,
                    job.failure_fingerprint,
                    job.next_run_ms.map(|value| value as i64),
                    job.lease_owner,
                    job.lease_expires_ms.map(|value| value as i64),
                    job.approval_id,
                    job.approval_expires_ms.map(|value| value as i64),
                    job.approval_reason,
                    job.created_ms as i64,
                    job.updated_ms as i64,
                ],
            )
            .map_err(|err| RuntimeError::Other(format!("failed to insert durable queue job: {err}")))?;
        Ok(job)
    }

    pub fn enqueue_typed_idempotent(
        &self,
        task: impl Into<String>,
        payload: Value,
        input_schema: Option<String>,
        max_retries: u64,
        budget_usd: f64,
        effect_summary: Option<String>,
        replay_key: Option<String>,
        idempotency_key: Option<String>,
        next_run_ms: Option<u64>,
    ) -> Result<QueueJob, RuntimeError> {
        let Some(idempotency_key) = idempotency_key.filter(|key| !key.trim().is_empty()) else {
            return self.enqueue_typed_at(
                task,
                payload,
                input_schema,
                max_retries,
                budget_usd,
                effect_summary,
                replay_key,
                next_run_ms,
            );
        };
        if let Some(existing) = self.get_by_idempotency_key(&idempotency_key)? {
            return Ok(existing);
        }
        let task = task.into();
        if task.trim().is_empty() {
            return Err(RuntimeError::Other(
                "std.queue task name must not be empty".to_string(),
            ));
        }
        let now = now_ms();
        let id = format!(
            "job_{}",
            self.next_id
                .fetch_add(1, Ordering::Relaxed)
                .saturating_add(1)
        );
        let budget_usd = if budget_usd.is_finite() && budget_usd > 0.0 {
            budget_usd
        } else {
            0.0
        };
        let job = QueueJob {
            id: id.clone(),
            task,
            payload,
            input_schema,
            status: QueueJobStatus::Pending,
            attempts: 0,
            max_retries,
            budget_usd,
            effect_summary,
            replay_key,
            idempotency_key: Some(idempotency_key.clone()),
            output_kind: None,
            output_fingerprint: None,
            failure_kind: None,
            failure_fingerprint: None,
            next_run_ms,
            lease_owner: None,
            lease_expires_ms: None,
            approval_id: None,
            approval_expires_ms: None,
            approval_reason: None,
            created_ms: now,
            updated_ms: now,
        };
        let payload_json = serde_json::to_string(&job.payload).map_err(|err| {
            RuntimeError::Other(format!("failed to serialize durable queue payload: {err}"))
        })?;
        let inserted = self
            .conn
            .lock()
            .unwrap()
            .execute(
                "insert into queue_jobs
                 (id, task, payload_json, input_schema, status, attempts, max_retries, budget_usd, effect_summary, replay_key, idempotency_key, output_kind, output_fingerprint, failure_kind, failure_fingerprint, next_run_ms, lease_owner, lease_expires_ms, approval_id, approval_expires_ms, approval_reason, created_ms, updated_ms)
                 values (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23)",
                params![
                    job.id,
                    job.task,
                    payload_json,
                    job.input_schema,
                    job.status.as_str(),
                    job.attempts as i64,
                    job.max_retries as i64,
                    job.budget_usd,
                    job.effect_summary,
                    job.replay_key,
                    job.idempotency_key,
                    job.output_kind,
                    job.output_fingerprint,
                    job.failure_kind,
                    job.failure_fingerprint,
                    job.next_run_ms.map(|value| value as i64),
                    job.lease_owner,
                    job.lease_expires_ms.map(|value| value as i64),
                    job.approval_id,
                    job.approval_expires_ms.map(|value| value as i64),
                    job.approval_reason,
                    job.created_ms as i64,
                    job.updated_ms as i64,
                ],
            );
        match inserted {
            Ok(_) => Ok(job),
            Err(err) => {
                if let Some(existing) = self.get_by_idempotency_key(&idempotency_key)? {
                    Ok(existing)
                } else {
                    Err(RuntimeError::Other(format!(
                        "failed to insert idempotent durable queue job: {err}"
                    )))
                }
            }
        }
    }
}
