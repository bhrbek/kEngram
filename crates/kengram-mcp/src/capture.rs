//! Capture orchestration for migration 0030's database gate.
//!
//! The database derives producer policy from `session_user`, computes the
//! fingerprint from stored content, performs exact/semantic decisions, and
//! atomically records source-event, queue, relation, and gate evidence.

use kengram_core::{EmbeddingStatus, Metadata, Scope, Source, ThoughtId};
use sqlx::PgPool;
use time::OffsetDateTime;

use crate::citation::{self, CitationError};

/// Hard upper bound on a single thought's content. Enforced before the DB
/// write so callers get a clean rejection.
pub const MAX_CONTENT_LEN: usize = 1_048_576; // 1 MiB

#[derive(Debug, Clone)]
pub struct CaptureRequest {
    pub content: String,
    pub source: Source,
    pub scope: Option<Scope>,
    pub metadata: Option<Metadata>,
    pub argus_source_event: Option<ArgusSourceEventRequest>,
}

/// Gate-only inputs used by callers that can supply source time, a
/// synchronous semantic vector, or atomic relation intents. Keeping these
/// separate preserves the established basic capture request for internal
/// queue/search fixtures.
#[derive(Debug, Clone, Default)]
pub struct CaptureGateOptions {
    pub source_created_at: Option<OffsetDateTime>,
    pub candidate_embedding: Option<Vec<f32>>,
    pub bypass_reason: Option<serde_json::Value>,
    pub relation_intents: Vec<serde_json::Value>,
    pub claimed_producer_class: Option<String>,
    pub correlation_id: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ArgusSourceEventRequest {
    pub namespace: String,
    pub source_ref: String,
    pub payload_hash: String,
    pub metadata: Option<Metadata>,
}

#[derive(Debug, Clone)]
pub struct ArgusSourceEventResponse {
    pub action: String,
    pub namespace: String,
    pub source_ref: String,
    pub payload_hash: String,
    pub status: String,
    pub thought_id: Option<ThoughtId>,
}

#[derive(Debug, Clone)]
pub struct CaptureResponse {
    pub thought_id: ThoughtId,
    pub born_on: OffsetDateTime,
    pub citation: String,
    pub embedding_status: EmbeddingStatus,
    /// `true` when the inserted fingerprint conflicted with an existing
    /// row — the returned `thought_id` belongs to the pre-existing row and
    /// no new embedding/tag jobs were enqueued. `false` when a fresh row
    /// was inserted.
    pub is_duplicate: bool,
    pub argus_source_event: Option<ArgusSourceEventResponse>,
    pub dedup_kind: Option<String>,
    pub matched_thought_id: Option<ThoughtId>,
    pub similarity: Option<f64>,
    pub resolved_origin_ids: Vec<ThoughtId>,
    pub derived_source_age: OffsetDateTime,
    pub persisted_source_age: OffsetDateTime,
    pub source_age_outcome: String,
    pub relation_results: serde_json::Value,
    pub gate_event_id: Option<uuid::Uuid>,
}

fn source_age_outcome(
    resolved_origin_ids: &[ThoughtId],
    is_duplicate: bool,
    derived_source_age: OffsetDateTime,
    persisted_source_age: OffsetDateTime,
) -> String {
    if resolved_origin_ids.is_empty() {
        "no_citation"
    } else if is_duplicate && derived_source_age == persisted_source_age {
        "duplicate_agrees"
    } else if is_duplicate {
        "duplicate_conflict"
    } else {
        "new_propagated"
    }
    .to_string()
}

/// Wall-clock budget for fingerprint-unique re-entry after a 23505.
/// Uncapped attempt counts (neo) stay schedule-independent; a time budget
/// converts a pathological hang (winner row invisible while unique index
/// still rejects) into a clean named error the caller can retry.
pub const FINGERPRINT_RACE_BUDGET: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("content must be non-empty")]
    EmptyContent,

    #[error("content is too long: {got} bytes (max {max})")]
    ContentTooLong { got: usize, max: usize },

    #[error("citation error: {0}")]
    Citation(#[from] CitationError),

    #[error("citation origin not found: {0}")]
    CitationOriginNotFound(ThoughtId),

    #[error(
        "citation origin scope mismatch: thought_id={thought_id} expected_scope={expected_scope} actual_scope={actual_scope}"
    )]
    CitationOriginScopeMismatch {
        thought_id: ThoughtId,
        expected_scope: String,
        actual_scope: String,
    },

    #[error("citation origin is retracted: {0}")]
    CitationOriginRetracted(ThoughtId),

    #[error(
        "source_created_at_too_far_in_future: source_created_at={source_created_at} observed_at={observed_at}"
    )]
    SourceCreatedAtTooFarInFuture {
        source_created_at: OffsetDateTime,
        observed_at: OffsetDateTime,
    },

    #[error("invalid argus_source_event: {0}")]
    InvalidArgusSourceEvent(&'static str),

    /// Fingerprint-unique race did not resolve within [`FINGERPRINT_RACE_BUDGET`].
    /// Not a raw SQLSTATE leak — callers may retry once (fleet read-result mitigation).
    #[error(
        "fingerprint_race_budget_exceeded: content-fingerprint unique race did not resolve within {budget_ms}ms"
    )]
    FingerprintRaceBudgetExceeded { budget_ms: u64 },

    #[error("storage error: {0}")]
    Storage(#[from] kengram_storage::StorageError),
}

/// Capture through the one database chokepoint.  A caller without a
/// synchronous candidate embedding must provide (or receives) a structured
/// bypass reason; shadow/enforce failures then keep and queue the thought.
///
/// `embedder_model_id` is the active embedder's identity (e.g.
/// `"bge-m3:1024"`). The worker uses it to pair the row with the right
/// embedder on drain.
///
/// `tagger_model_id` is the active tagger's identity (e.g.
/// `"vllm/qwen3-coder:30b"`). `None` silent-disables the tag-job
/// enqueue — captures still work, the thought just stays with `tags = '{}'`
/// until a tagger is configured and the operator runs `kengram tag --rerun`.

/// Constraint created in migrations/0006_collapse_to_thoughts.sql.
const THOUGHTS_CONTENT_FINGERPRINT_UNIQUE: &str = "thoughts_content_fingerprint_unique";

/// Pure parts of the fingerprint-unique classifier. Exposed for unit tests so
/// classification can fail without constructing sqlx::DatabaseError.
///
/// When `constraint` is `Some`, only exact equality to
/// `thoughts_content_fingerprint_unique` matches — never fall through to the
/// message. Online reindex renames (…_v2 alongside the old name) can leave the
/// old name in the message while constraint() reports the new one; substring
/// matching would misclassify that 23505 as the fingerprint race.
pub(crate) fn is_fingerprint_unique_violation_parts(
    code: Option<&str>,
    constraint: Option<&str>,
    message: &str,
) -> bool {
    if code != Some("23505") {
        return false;
    }
    match constraint {
        Some(name) => name == THOUGHTS_CONTENT_FINGERPRINT_UNIQUE,
        // Drivers sometimes omit constraint(); message still names it.
        None => message.contains(THOUGHTS_CONTENT_FINGERPRINT_UNIQUE),
    }
}

/// True only for the content-fingerprint unique race (sqlstate 23505 on
/// `thoughts_content_fingerprint_unique`). Other unique/check/FK failures stay
/// genuine errors.
pub(crate) fn is_thoughts_content_fingerprint_unique_violation(err: &sqlx::Error) -> bool {
    let sqlx::Error::Database(db) = err else {
        return false;
    };
    is_fingerprint_unique_violation_parts(db.code().as_deref(), db.constraint(), db.message())
}

fn storage_is_fingerprint_unique_violation(err: &kengram_storage::StorageError) -> bool {
    matches!(
        err,
        kengram_storage::StorageError::Database(e) if is_thoughts_content_fingerprint_unique_violation(e)
    )
}

pub async fn capture(
    pool: &PgPool,
    embedder_model_id: &str,
    tagger_model_id: Option<&str>,
    request: CaptureRequest,
) -> Result<CaptureResponse, CaptureError> {
    capture_with_gate_options(
        pool,
        embedder_model_id,
        tagger_model_id,
        request,
        CaptureGateOptions::default(),
    )
    .await
}

pub async fn capture_with_gate_options(
    pool: &PgPool,
    embedder_model_id: &str,
    tagger_model_id: Option<&str>,
    request: CaptureRequest,
    options: CaptureGateOptions,
) -> Result<CaptureResponse, CaptureError> {
    if request.content.is_empty() {
        return Err(CaptureError::EmptyContent);
    }
    if request.content.len() > MAX_CONTENT_LEN {
        return Err(CaptureError::ContentTooLong {
            got: request.content.len(),
            max: MAX_CONTENT_LEN,
        });
    }

    let prepared = citation::prepare_content(&request.content)?;
    if prepared.stripped_content.is_empty() {
        return Err(CaptureError::EmptyContent);
    }
    if prepared.stripped_content.len() > MAX_CONTENT_LEN {
        return Err(CaptureError::ContentTooLong {
            got: prepared.stripped_content.len(),
            max: MAX_CONTENT_LEN,
        });
    }
    let stored_content = prepared.stripped_content;
    let resolved_origin_ids = prepared.distinct_origin_ids;
    let citation_origin_ids: Vec<uuid::Uuid> = resolved_origin_ids
        .iter()
        .map(|id| id.into_uuid())
        .collect();

    let scope = request.scope.unwrap_or_default();
    let metadata = request.metadata.unwrap_or_default();
    let source_event = request.argus_source_event;
    if let Some(event) = &source_event {
        if event.namespace.trim().is_empty() {
            return Err(CaptureError::InvalidArgusSourceEvent(
                "namespace is required",
            ));
        }
        if event.source_ref.trim().is_empty() {
            return Err(CaptureError::InvalidArgusSourceEvent(
                "source_ref is required",
            ));
        }
        if event.payload_hash.trim().is_empty() {
            return Err(CaptureError::InvalidArgusSourceEvent(
                "payload_hash is required",
            ));
        }
    }

    let source_event_metadata = source_event
        .as_ref()
        .and_then(|event| event.metadata.as_ref())
        .map(Metadata::as_value);
    let relation_intents = serde_json::Value::Array(options.relation_intents);
    let default_bypass = serde_json::json!({
        "code": "candidate_embedding_unavailable",
        "detail": "capture caller did not provide a synchronous bge-m3 vector"
    });
    let bypass_reason = if options.candidate_embedding.is_none() {
        Some(options.bypass_reason.as_ref().unwrap_or(&default_bypass))
    } else {
        options.bypass_reason.as_ref()
    };
    // Test-only: hang BEFORE any gate SQL so the outer deadline can fire with
    // zero durable rows (carl coverage: persisted=false path).
    #[cfg(test)]
    {
        if request
            .content
            .starts_with(test_hooks::HANG_BEFORE_GATE_PREFIX)
        {
            std::future::pending::<()>().await;
        }
    }

    // Race: concurrent same-content captures can both miss the pre-insert
    // fingerprint SELECT; the loser INSERT hits thoughts_content_fingerprint_unique
    // (23505) and the gate transaction rolls back. Do NOT fabricate a CaptureResponse
    // after rollback (that produced "stored" receipts without durable ledger rows).
    // Re-enter the normal gated path so exact_duplicate (and source-event / relation
    // disposition) are executed and committed. Other 23505s rethrow.
    let run_gate = || {
        kengram_storage::corpus_hygiene::capture_thought_gated(
            pool,
            kengram_storage::corpus_hygiene::GatedCaptureRequest {
                scope: scope.as_str(),
                content: &stored_content,
                source: request.source.as_str(),
                metadata: metadata.as_value(),
                raw_source_created_at: options.source_created_at,
                citation_origin_ids: &citation_origin_ids,
                candidate_embedding: options.candidate_embedding.as_deref(),
                embedding_model_id: Some(embedder_model_id),
                embedding_model_version: Some(1),
                bypass_reason,
                source_event_namespace: source_event.as_ref().map(|event| event.namespace.as_str()),
                source_event_ref: source_event.as_ref().map(|event| event.source_ref.as_str()),
                source_event_payload_hash: source_event
                    .as_ref()
                    .map(|event| event.payload_hash.as_str()),
                source_event_metadata,
                relation_intents: &relation_intents,
                tagger_model_id,
                claimed_producer_class: options.claimed_producer_class.as_deref(),
                correlation_id: options.correlation_id.as_deref(),
                force_keep_token: None,
            },
        )
    };

    // DETERMINISTIC resolution for fingerprint-unique races (knox R3 / neo +
    // hunter safety consolidation):
    // after a thoughts_content_fingerprint_unique 23505 the gate transaction
    // has rolled back — re-enter until the gate resolves (exact_duplicate once
    // the winner row is visible, or a later insert wins). No fixed attempt
    // cap: counts are schedule-dependent (neo). Wall-clock budget
    // FINGERPRINT_RACE_BUDGET bounds the loop so a pathological state where
    // the unique index rejects while SELECT still misses cannot hang the
    // fleet-memory hot path forever (statement_timeout is per-transaction
    // only). On expiry: FingerprintRaceBudgetExceeded (named, not raw 23505).
    //
    // Foreign uniques and exact_content_requires_adjudication rethrow
    // immediately (classifier false → break). Only fingerprint-unique loops.
    let deadline = tokio::time::Instant::now() + FINGERPRINT_RACE_BUDGET;
    let mut gated = run_gate().await;
    let mut backoff_ms: u64 = 1;
    loop {
        match gated {
            Ok(_) => break,
            Err(ref err) if storage_is_fingerprint_unique_violation(err) => {
                let now = tokio::time::Instant::now();
                if now >= deadline {
                    return Err(CaptureError::FingerprintRaceBudgetExceeded {
                        budget_ms: FINGERPRINT_RACE_BUDGET.as_millis() as u64,
                    });
                }
                let remaining = deadline - now;
                let sleep_for = std::time::Duration::from_millis(backoff_ms).min(remaining);
                tokio::time::sleep(sleep_for).await;
                backoff_ms = (backoff_ms.saturating_mul(2)).min(32);
                gated = run_gate().await;
            }
            Err(_) => break,
        }
    }
    let result = match gated {
        Ok(result) => result,
        Err(kengram_storage::StorageError::SourceCreatedAtTooFarInFuture {
            source_created_at,
            observed_at,
        }) => {
            return Err(CaptureError::SourceCreatedAtTooFarInFuture {
                source_created_at,
                observed_at,
            });
        }
        Err(kengram_storage::StorageError::CitationOriginNotFound(id)) => {
            return Err(CaptureError::CitationOriginNotFound(ThoughtId::from(id)));
        }
        Err(kengram_storage::StorageError::CitationOriginScopeMismatch {
            thought_id,
            expected_scope,
            actual_scope,
        }) => {
            return Err(CaptureError::CitationOriginScopeMismatch {
                thought_id: ThoughtId::from(thought_id),
                expected_scope,
                actual_scope,
            });
        }
        Err(kengram_storage::StorageError::CitationOriginRetracted(id)) => {
            return Err(CaptureError::CitationOriginRetracted(ThoughtId::from(id)));
        }
        Err(error) => return Err(CaptureError::Storage(error)),
    };

    // A conflicting replay does not select a new corpus row, but the source
    // event ledger still identifies the original thought for the established
    // MCP conflict response.
    let thought_id = result
        .thought_id
        .or(result.matched_thought_id)
        .ok_or_else(|| {
            CaptureError::Storage(kengram_storage::StorageError::Database(
                sqlx::Error::Protocol(format!(
                    "capture gate returned action={} without thought_id",
                    result.action
                )),
            ))
        })?;
    let thought_id = ThoughtId::from(thought_id);
    let is_duplicate = matches!(
        result.action.as_str(),
        "exact_duplicate" | "semantic_duplicate"
    );
    // A source-event payload conflict preserves the established public
    // `is_duplicate=false` and `action=conflict` contract, but it still
    // returns the prior matched thought without applying this request's
    // derived age.  Never describe that prior-row disposition as a newly
    // propagated write.
    let source_event_conflict_selected_prior = result.source_event_action.as_deref()
        == Some("conflict")
        && result.matched_thought_id.is_some();
    let source_age_outcome =
        if !resolved_origin_ids.is_empty() && source_event_conflict_selected_prior {
            "duplicate_conflict".to_string()
        } else {
            source_age_outcome(
                &resolved_origin_ids,
                is_duplicate,
                result.derived_created_at,
                result.persisted_created_at,
            )
        };
    let argus_source_event = source_event.map(|event| {
        let action = match result.source_event_action.as_deref() {
            // Preserve the established MCP response contract while the gate
            // records the more precise replay disposition in its ledger.
            Some("replay") => "duplicate_skip".to_string(),
            Some(action) => action.to_string(),
            None => result.action.clone(),
        };
        ArgusSourceEventResponse {
            action,
            namespace: event.namespace,
            source_ref: event.source_ref,
            payload_hash: event.payload_hash,
            status: result
                .source_event_status
                .clone()
                .unwrap_or_else(|| "stored".to_string()),
            thought_id: Some(thought_id),
        }
    });

    let response = CaptureResponse {
        thought_id,
        born_on: result.persisted_created_at,
        citation: citation::format_citation(thought_id),
        embedding_status: EmbeddingStatus::Pending,
        is_duplicate,
        argus_source_event,
        dedup_kind: is_duplicate.then(|| result.action.clone()),
        matched_thought_id: result.matched_thought_id.map(ThoughtId::from),
        similarity: result.similarity,
        resolved_origin_ids,
        derived_source_age: result.derived_created_at,
        persisted_source_age: result.persisted_created_at,
        source_age_outcome,
        relation_results: result.relation_results,
        gate_event_id: result.gate_event_id,
    };

    // Test-only: hang AFTER durable gate commit when content carries the
    // marker prefix. Content-keyed (not a global flag) so parallel sqlx::test
    // workers cannot poison each other.
    #[cfg(test)]
    {
        // Only hang on first durable insert; exact_duplicate must return so
        // the re-capture oracle can observe is_duplicate=true without another
        // timeout recovery (which would force is_duplicate=false).
        if !is_duplicate
            && request
                .content
                .starts_with(test_hooks::HANG_AFTER_GATE_PREFIX)
        {
            std::future::pending::<()>().await;
        }
    }

    Ok(response)
}

/// Per-statement timeout inside the probe transaction.
pub const CAPTURE_PROBE_STATEMENT_TIMEOUT_MS: u64 = 150;

/// Worst-case sequential statements on Path A (ASE present):
/// begin + set_config + exact-triple SELECT + ns/ref SELECT + commit.
/// Live 2026-07-28: 200ms outer starved this path (persisted_probe_error).
pub const CAPTURE_PROBE_PATH_A_SEQUENTIAL_STATEMENTS: u64 = 5;

/// Probe budget for post-deadline honesty lookup.
/// Must cover Path A statement budget (5 x 150ms = 750ms) plus slack.
pub const CAPTURE_PERSISTENCE_PROBE_TIMEOUT: std::time::Duration =
    std::time::Duration::from_millis(
        CAPTURE_PROBE_STATEMENT_TIMEOUT_MS * CAPTURE_PROBE_PATH_A_SEQUENTIAL_STATEMENTS + 50,
    );

/// Local statement_timeout for the persistence probe transaction.
const CAPTURE_PROBE_STATEMENT_TIMEOUT: &str = "150ms";

/// Result of post-deadline recovery (jones P1: content-only synthesis lies).
#[derive(Debug, Clone)]
pub enum DeadlineRecovery {
    /// This request (or its content/source identity) durably landed; fields
    /// are hydrated from the ledger so receipts match DB rows.
    Recovered(CaptureResponse),
    /// No durable evidence that *this* request completed the gate.
    NotPersisted,
}

/// Look up a durable thought by content fingerprint (SHA-256 of content bytes
/// as stored by the gate).
pub async fn find_thought_id_by_content(
    pool: &PgPool,
    content: &str,
) -> Result<Option<ThoughtId>, CaptureError> {
    let mut tx = begin_probe_tx(pool).await?;
    let row: Option<(uuid::Uuid,)> = sqlx::query_as(
        r#"
        SELECT id
        FROM thoughts
        WHERE content_fingerprint = digest($1::text, 'sha256')
          AND retracted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(content)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
    tx.commit()
        .await
        .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
    Ok(row.map(|(id,)| ThoughtId::from(id)))
}

async fn begin_probe_tx(
    pool: &PgPool,
) -> Result<sqlx::Transaction<'_, sqlx::Postgres>, CaptureError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
    sqlx::query("SELECT set_config('statement_timeout', $1, true)")
        .bind(CAPTURE_PROBE_STATEMENT_TIMEOUT)
        .execute(&mut *tx)
        .await
        .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
    Ok(tx)
}

/// Honest post-deadline recovery.
///
/// Invariants (jones attack legs at b2418da):
/// 1. Never synthesize success with null `gate_event_id` / `argus_source_event`
///    when the ledger has those rows for this request.
/// 2. Never claim success for a pre-existing content match when this request's
///    `argus_source_event` identity did not land (hang-before-gate + preseed).
///
/// Structural rule (knox design / jones 506841+506848): recovery keys on
/// THIS call's identity — never content/history inference across keys.
///
/// When `argus_source_event` is present, the probe keys **strictly** on the
/// ASE triple (namespace, source_ref, payload_hash). Content fingerprint is
/// only a secondary check *within* that key, never a substitute for it.
/// Same ns/ref with a different stored payload_hash is the established MCP
/// **conflict** response (not old success, not a blank deadline silence).
///
/// When ASE is absent, content-only inference is forbidden without a
/// **server-minted** call-window binder: a non-empty `correlation_id` that
/// matches a gate row for this fingerprint. Callers may supply a correlation
/// for forensics, but the MCP capture path mints a unique attempt id per call
/// so reused client correlations cannot prove a later attempt (jones 509399).
/// Unkeyed / mismatched prior gates cannot prove hang-before-gate success.
pub async fn recover_after_deadline(
    pool: &PgPool,
    content: &str,
    argus_source_event: Option<&ArgusSourceEventRequest>,
    correlation_id: Option<&str>,
    resolved_origin_ids: &[ThoughtId],
) -> Result<DeadlineRecovery, CaptureError> {
    let mut tx = begin_probe_tx(pool).await?;

    // --- Path A: ASE triple is the sole recovery key when present ---
    if let Some(ev) = argus_source_event {
        // Success path: exact triple match only (never ns/ref alone).
        let se_exact: Option<(
            uuid::Uuid,
            String,
            String,
            String,
            String,
            Option<uuid::Uuid>,
            Option<OffsetDateTime>,
        )> = sqlx::query_as(
            r#"
                SELECT se.id, se.namespace, se.source_ref, se.payload_hash,
                       se.status, se.thought_id, t.created_at
                FROM argus_source_events se
                LEFT JOIN thoughts t ON t.id = se.thought_id
                WHERE se.namespace = $1
                  AND se.source_ref = $2
                  AND se.payload_hash = $3
                LIMIT 1
                "#,
        )
        .bind(&ev.namespace)
        .bind(&ev.source_ref)
        .bind(&ev.payload_hash)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;

        if se_exact.is_none() {
            // No triple match. Same ns/ref with a *different* payload is a
            // payload_hash conflict — honest MCP conflict, not inferred success
            // and not NotPersisted (knox: conflict error, not old success).
            let se_ns_ref: Option<(
                uuid::Uuid,
                String,
                String,
                String,
                String,
                Option<uuid::Uuid>,
                Option<OffsetDateTime>,
            )> = sqlx::query_as(
                r#"
                SELECT se.id, se.namespace, se.source_ref, se.payload_hash,
                       se.status, se.thought_id, t.created_at
                FROM argus_source_events se
                LEFT JOIN thoughts t ON t.id = se.thought_id
                WHERE se.namespace = $1 AND se.source_ref = $2
                LIMIT 1
                "#,
            )
            .bind(&ev.namespace)
            .bind(&ev.source_ref)
            .fetch_optional(&mut *tx)
            .await
            .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;

            if let Some((
                _id,
                ns,
                sref,
                stored_phash,
                _status,
                thought_id_opt,
                persisted_created_at,
            )) = se_ns_ref
            {
                if stored_phash != ev.payload_hash {
                    tx.commit().await.map_err(|e| {
                        CaptureError::Storage(kengram_storage::StorageError::Database(e))
                    })?;
                    // Match live gate contract: action/status=conflict, surface
                    // stored payload + original thought (no new write claimed).
                    let thought_id = match thought_id_opt {
                        Some(u) => ThoughtId::from(u),
                        None => {
                            // Conflict row without thought is still not success
                            // for *this* payload; refuse inferred recovery.
                            return Ok(DeadlineRecovery::NotPersisted);
                        }
                    };
                    let Some(persisted_created_at) = persisted_created_at else {
                        return Ok(DeadlineRecovery::NotPersisted);
                    };
                    let argus = ArgusSourceEventResponse {
                        action: "conflict".to_string(),
                        namespace: ns,
                        source_ref: sref,
                        // Stored (original) payload — caller sees the conflict
                        // against what is durably bound to this identity.
                        payload_hash: stored_phash,
                        status: "conflict".to_string(),
                        thought_id: Some(thought_id),
                    };
                    return Ok(DeadlineRecovery::Recovered(CaptureResponse {
                        thought_id,
                        born_on: persisted_created_at,
                        citation: citation::format_citation(thought_id),
                        embedding_status: EmbeddingStatus::Pending,
                        is_duplicate: false,
                        argus_source_event: Some(argus),
                        dedup_kind: None,
                        matched_thought_id: Some(thought_id),
                        similarity: None,
                        resolved_origin_ids: resolved_origin_ids.to_vec(),
                        derived_source_age: persisted_created_at,
                        persisted_source_age: persisted_created_at,
                        source_age_outcome: if resolved_origin_ids.is_empty() {
                            "no_citation"
                        } else {
                            "duplicate_conflict"
                        }
                        .to_string(),
                        relation_results: serde_json::json!([]),
                        gate_event_id: None,
                    }));
                }
            }

            tx.commit()
                .await
                .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
            // No ASE row for this identity at all → this attempt did not land.
            return Ok(DeadlineRecovery::NotPersisted);
        }

        let (_se_id, ns, sref, phash, status, thought_id_opt, persisted_created_at) =
            se_exact.expect("checked");

        let Some(thought_uuid) = thought_id_opt else {
            tx.commit()
                .await
                .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
            return Ok(DeadlineRecovery::NotPersisted);
        };
        let thought_id = ThoughtId::from(thought_uuid);
        let Some(persisted_created_at) = persisted_created_at else {
            tx.commit()
                .await
                .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
            return Ok(DeadlineRecovery::NotPersisted);
        };

        // Secondary receipts *within* the ASE triple only — never cross keys.
        let gate: Option<(
            uuid::Uuid,
            String,
            Option<uuid::Uuid>,
            Option<f64>,
            OffsetDateTime,
        )> = sqlx::query_as(
            r#"
            SELECT id, action, matched_thought_id, similarity, effective_created_at
            FROM thought_ingest_gate_events
            WHERE source_event_namespace = $1
              AND source_event_ref = $2
              AND (
                    source_event_payload_hash IS NULL
                    OR source_event_payload_hash = $3
                  )
              AND ($4::text IS NULL OR correlation_id = $4)
            ORDER BY created_at DESC
            LIMIT 1
            "#,
        )
        .bind(&ns)
        .bind(&sref)
        .bind(&phash)
        .bind(correlation_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;

        // Optional content secondary check within the ASE key: if a thought is
        // bound but content fingerprint disagrees, still trust ASE ledger
        // (conflict/replay identity is ASE-keyed, not content-keyed).
        let _ = content;

        tx.commit()
            .await
            .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;

        let (gate_event_id, action, matched, similarity, derived_source_age) = match gate {
            Some((id, action, matched, sim, effective)) => {
                (Some(id), action, matched, sim, effective)
            }
            None => {
                // Source event triple landed but gate ledger row missing —
                // durable ASE identity still proves this request's payload.
                (None, "stored".to_string(), None, None, persisted_created_at)
            }
        };
        let is_duplicate = matches!(
            action.as_str(),
            "exact_duplicate" | "semantic_duplicate" | "replay"
        );

        let argus = ArgusSourceEventResponse {
            action: if action == "replay" {
                "duplicate_skip".to_string()
            } else {
                action.clone()
            },
            namespace: ns,
            source_ref: sref,
            payload_hash: phash,
            status,
            thought_id: Some(thought_id),
        };

        return Ok(DeadlineRecovery::Recovered(CaptureResponse {
            thought_id,
            born_on: persisted_created_at,
            citation: citation::format_citation(thought_id),
            embedding_status: EmbeddingStatus::Pending,
            is_duplicate,
            argus_source_event: Some(argus),
            dedup_kind: is_duplicate.then_some(action),
            matched_thought_id: matched.map(ThoughtId::from),
            similarity,
            resolved_origin_ids: resolved_origin_ids.to_vec(),
            derived_source_age,
            persisted_source_age: persisted_created_at,
            source_age_outcome: source_age_outcome(
                resolved_origin_ids,
                is_duplicate,
                derived_source_age,
                persisted_created_at,
            ),
            relation_results: serde_json::json!([]),
            gate_event_id,
        }));
    }

    // --- Path B: no ASE — forbid content/history inference without call binder ---
    // correlation_id must be the server-minted attempt id written on THIS call's
    // gate row. Caller-reused correlation strings must not match a prior gate
    // for a hang-before-gate attempt (that gate was never written for this call).
    // Absent or empty → NotPersisted.
    let Some(corr) = correlation_id.map(str::trim).filter(|c| !c.is_empty()) else {
        tx.commit()
            .await
            .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
        return Ok(DeadlineRecovery::NotPersisted);
    };

    let gate: Option<(
        uuid::Uuid,
        String,
        Option<uuid::Uuid>,
        Option<f64>,
        OffsetDateTime,
    )> = sqlx::query_as(
        r#"
        SELECT g.id, g.action, g.matched_thought_id, g.similarity,
               g.effective_created_at
        FROM thought_ingest_gate_events g
        WHERE g.candidate_fingerprint = digest($1::text, 'sha256')
          AND g.correlation_id = $2
        ORDER BY created_at DESC
        LIMIT 1
        "#,
    )
    .bind(content)
    .bind(corr)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;

    let Some((gate_id, action, matched, similarity, derived_source_age)) = gate else {
        // Content thought without gate row is ambiguous (preseed / hang-before).
        // Do not claim this request succeeded (jones attack 2 class).
        tx.commit()
            .await
            .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
        return Ok(DeadlineRecovery::NotPersisted);
    };

    let thought_row: Option<(uuid::Uuid, OffsetDateTime)> = sqlx::query_as(
        r#"
        SELECT id, created_at FROM thoughts
        WHERE content_fingerprint = digest($1::text, 'sha256')
          AND retracted_at IS NULL
        LIMIT 1
        "#,
    )
    .bind(content)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;

    let (thought_uuid, persisted_created_at) = match (thought_row, matched) {
        (Some((id, created_at)), _) => (id, created_at),
        (None, Some(id)) => {
            let created_at: OffsetDateTime =
                sqlx::query_scalar("SELECT created_at FROM thoughts WHERE id = $1")
                    .bind(id)
                    .fetch_one(&mut *tx)
                    .await
                    .map_err(|e| {
                        CaptureError::Storage(kengram_storage::StorageError::Database(e))
                    })?;
            (id, created_at)
        }
        (None, None) => {
            tx.commit()
                .await
                .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;
            return Ok(DeadlineRecovery::NotPersisted);
        }
    };

    tx.commit()
        .await
        .map_err(|e| CaptureError::Storage(kengram_storage::StorageError::Database(e)))?;

    let thought_id = ThoughtId::from(thought_uuid);
    let is_duplicate = matches!(
        action.as_str(),
        "exact_duplicate" | "semantic_duplicate" | "replay"
    );

    Ok(DeadlineRecovery::Recovered(CaptureResponse {
        thought_id,
        born_on: persisted_created_at,
        citation: citation::format_citation(thought_id),
        embedding_status: EmbeddingStatus::Pending,
        is_duplicate,
        argus_source_event: None,
        dedup_kind: is_duplicate.then_some(action.clone()),
        matched_thought_id: matched
            .map(ThoughtId::from)
            .or_else(|| if is_duplicate { Some(thought_id) } else { None }),
        similarity,
        resolved_origin_ids: resolved_origin_ids.to_vec(),
        derived_source_age,
        persisted_source_age: persisted_created_at,
        source_age_outcome: source_age_outcome(
            resolved_origin_ids,
            is_duplicate,
            derived_source_age,
            persisted_created_at,
        ),
        relation_results: serde_json::json!([]),
        gate_event_id: Some(gate_id),
    }))
}

#[cfg(test)]
pub mod test_hooks {
    /// Prefix content with this string to hang after durable gate commit
    /// (carl deadline oracle). Parallel-safe: only that capture hangs.
    pub const HANG_AFTER_GATE_PREFIX: &str = "__KENGRAM_TEST_HANG_AFTER_GATE__\n";

    /// Prefix content to hang *before* any gate SQL (carl coverage: deadline
    /// with persisted=false / no durable row). Content-keyed for parallel tests.
    pub const HANG_BEFORE_GATE_PREFIX: &str = "__KENGRAM_TEST_HANG_BEFORE_GATE__\n";
}

#[cfg(test)]
mod tests {
    #[test]
    fn probe_outer_covers_path_a_statement_budget() {
        let outer = super::CAPTURE_PERSISTENCE_PROBE_TIMEOUT.as_millis() as u64;
        let need = super::CAPTURE_PROBE_STATEMENT_TIMEOUT_MS
            * super::CAPTURE_PROBE_PATH_A_SEQUENTIAL_STATEMENTS;
        assert!(
            outer >= need,
            "outer probe timeout {outer}ms must cover Path A {need}ms (5 x 150ms statements)"
        );
        // The 200ms budget that lost 2026-07-28 must stay illegal.
        assert!(
            outer > 200,
            "outer {outer}ms must exceed the starved 200ms budget"
        );
    }

    use super::*;
    use kengram_core::EmbeddingModel;
    use serde_json::json;
    use sqlx::{PgConnection, Row, postgres::PgRow};
    use std::time::Duration;

    const TEST_EMBEDDER_MODEL_ID: &str = "bge-m3:1024";
    const TEST_TAGGER_MODEL_ID: &str = "fake/tagger";

    fn req(content: &str, source: &str) -> CaptureRequest {
        CaptureRequest {
            content: content.to_string(),
            source: Source::new(source).unwrap(),
            scope: None,
            metadata: None,
            argus_source_event: None,
        }
    }

    fn unit_vector_literal() -> String {
        format!("[{}]", vec!["0.03125"; 1024].join(","))
    }

    const SOURCE_EVENT_CLAIM_TEST_LOCK: i64 = 7_301_001;

    async fn install_source_event_claim_barrier(pool: &PgPool) {
        sqlx::query(
            r#"
            CREATE FUNCTION public.test_block_source_event_claim()
            RETURNS trigger
            LANGUAGE plpgsql
            AS $test$
            BEGIN
                PERFORM pg_catalog.pg_advisory_xact_lock(7301001);
                RETURN NEW;
            END
            $test$
            "#,
        )
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            CREATE TRIGGER test_block_source_event_claim
            BEFORE INSERT ON public.argus_source_events
            FOR EACH ROW EXECUTE FUNCTION public.test_block_source_event_claim()
            "#,
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn wait_for_active_test_clients(pool: &PgPool, prefix: &str, expected: i64) {
        let pattern = format!("{prefix}%");
        for _ in 0..200 {
            let active: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM pg_catalog.pg_stat_activity WHERE application_name LIKE $1 AND state = 'active'",
            )
            .bind(&pattern)
            .fetch_one(pool)
            .await
            .unwrap();
            if active >= expected {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("timed out waiting for {expected} concurrent {prefix} clients");
    }

    async fn gated_capture_on_connection(
        connection: &mut PgConnection,
        scope: &str,
        content: &str,
        source_event: Option<(&str, &str, &str)>,
        relation_intents: &serde_json::Value,
        tagger_model_id: Option<&str>,
    ) -> PgRow {
        let (namespace, source_ref, payload_hash) = source_event
            .map(|(namespace, source_ref, payload_hash)| {
                (Some(namespace), Some(source_ref), Some(payload_hash))
            })
            .unwrap_or((None, None, None));
        sqlx::query(
            r#"
            SELECT *
            FROM public.capture_thought_gated(
                p_scope => $1,
                p_content => $2,
                p_source => 'test',
                p_metadata => '{}'::jsonb,
                p_source_created_at => NULL::timestamptz,
                p_candidate_embedding => $3::vector(1024),
                p_embedding_model_id => 'bge-m3:1024',
                p_embedding_model_version => 1,
                p_bypass_reason => NULL::jsonb,
                p_source_event_namespace => $4,
                p_source_event_ref => $5,
                p_source_event_payload_hash => $6,
                p_source_event_metadata => '{}'::jsonb,
                p_relation_intents => $7,
                p_tagger_model_id => $8,
                p_claimed_producer_class => NULL::text,
                p_correlation_id => NULL::text,
                p_force_keep_token => NULL::text
            )
            "#,
        )
        .bind(scope)
        .bind(content)
        .bind(unit_vector_literal())
        .bind(namespace)
        .bind(source_ref)
        .bind(payload_hash)
        .bind(relation_intents)
        .bind(tagger_model_id)
        .fetch_one(&mut *connection)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn writes_thought_and_enqueues_returns_pending(pool: PgPool) {
        let resp = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("first thought", "manual"),
        )
        .await
        .unwrap();

        assert_eq!(resp.embedding_status, EmbeddingStatus::Pending);
        assert!(!resp.is_duplicate);

        let fetched = kengram_storage::fetch_thought(&pool, resp.thought_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.content, "first thought");

        // Queue rows exist; no embedding row yet.
        assert_eq!(kengram_storage::count_pending(&pool).await.unwrap(), 1);
        let tag_jobs = kengram_storage::fetch_pending_tag_jobs(&pool, 10)
            .await
            .unwrap();
        assert_eq!(tag_jobs.len(), 1);
        assert!(
            !kengram_storage::thought_has_embedding(
                &pool,
                resp.thought_id,
                &EmbeddingModel::bge_m3(),
            )
            .await
            .unwrap()
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn empty_content_returns_error(pool: PgPool) {
        let err = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("", "manual"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CaptureError::EmptyContent));
        // Errored before the insert; queues stay empty.
        assert_eq!(kengram_storage::count_pending(&pool).await.unwrap(), 0);
        let tag_jobs = kengram_storage::fetch_pending_tag_jobs(&pool, 10)
            .await
            .unwrap();
        assert!(tag_jobs.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn overlong_content_returns_error(pool: PgPool) {
        let big = "x".repeat(MAX_CONTENT_LEN + 1);
        let err = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req(&big, "manual"),
        )
        .await
        .unwrap_err();
        assert!(matches!(err, CaptureError::ContentTooLong { got, max } if got > max));
        assert_eq!(kengram_storage::count_pending(&pool).await.unwrap(), 0);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn defaults_scope_to_global_when_missing(pool: PgPool) {
        let resp = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("hello", "manual"),
        )
        .await
        .unwrap();
        let fetched = kengram_storage::fetch_thought(&pool, resp.thought_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.scope, Scope::global());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn defaults_metadata_to_empty_when_missing(pool: PgPool) {
        let resp = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("hello", "manual"),
        )
        .await
        .unwrap();
        let fetched = kengram_storage::fetch_thought(&pool, resp.thought_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.metadata, Metadata::empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn preserves_scope_source_metadata(pool: PgPool) {
        let request = CaptureRequest {
            content: "remember this".to_string(),
            source: Source::new("agent:claude-code").unwrap(),
            scope: Some(Scope::new("work.tcgplayer").unwrap()),
            metadata: Some(Metadata::from(
                json!({"session_id": "abc", "tool_name": "TodoWrite"}),
            )),
            argus_source_event: None,
        };
        let resp = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            request.clone(),
        )
        .await
        .unwrap();

        let fetched = kengram_storage::fetch_thought(&pool, resp.thought_id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.scope, request.scope.unwrap());
        assert_eq!(fetched.source, request.source);
        assert_eq!(fetched.metadata, request.metadata.unwrap());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn argus_source_event_gates_store_duplicate_and_conflict(pool: PgPool) {
        let source_event = ArgusSourceEventRequest {
            namespace: "agents/trinity".to_string(),
            source_ref: "mem_save:agents/trinity:source-event-test".to_string(),
            payload_hash: "payload-a".to_string(),
            metadata: Some(Metadata::from(json!({"legacy_tool": "mem_save"}))),
        };

        let request = CaptureRequest {
            content: "Argus source-event capture test v1".to_string(),
            source: Source::new("agent:trinity").unwrap(),
            scope: Some(Scope::new("agents/trinity").unwrap()),
            metadata: Some(Metadata::from(json!({"title": "Argus source-event test"}))),
            argus_source_event: Some(source_event.clone()),
        };

        let first = capture(&pool, TEST_EMBEDDER_MODEL_ID, None, request.clone())
            .await
            .unwrap();
        assert!(!first.is_duplicate);
        let first_event = first.argus_source_event.as_ref().unwrap();
        assert_eq!(first_event.action, "stored");
        assert_eq!(first_event.status, "stored");
        assert_eq!(first_event.thought_id, Some(first.thought_id));

        let dup = capture(&pool, TEST_EMBEDDER_MODEL_ID, None, request.clone())
            .await
            .unwrap();
        assert!(dup.is_duplicate);
        let dup_event = dup.argus_source_event.as_ref().unwrap();
        assert_eq!(dup_event.action, "duplicate_skip");
        assert_eq!(dup_event.thought_id, Some(first.thought_id));

        let thoughts_before_conflict: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts")
            .fetch_one(&pool)
            .await
            .unwrap();

        let mut conflict_request = request;
        conflict_request.content = "Argus source-event capture test v2".to_string();
        conflict_request.argus_source_event = Some(ArgusSourceEventRequest {
            payload_hash: "payload-b".to_string(),
            ..source_event
        });
        let conflict = capture(&pool, TEST_EMBEDDER_MODEL_ID, None, conflict_request)
            .await
            .unwrap();
        let conflict_event = conflict.argus_source_event.as_ref().unwrap();
        assert_eq!(conflict_event.action, "conflict");
        assert_eq!(conflict_event.status, "conflict");
        assert_eq!(conflict_event.thought_id, Some(first.thought_id));

        let thoughts_after_conflict: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(thoughts_after_conflict, thoughts_before_conflict);

        let row = sqlx::query(
            "SELECT status, error, payload_hash FROM argus_source_events WHERE namespace = $1 AND source_ref = $2",
        )
        .bind("agents/trinity")
        .bind("mem_save:agents/trinity:source-event-test")
        .fetch_one(&pool)
        .await
        .unwrap();
        let status: String = row.try_get("status").unwrap();
        let error: Option<String> = row.try_get("error").unwrap();
        let payload_hash: String = row.try_get("payload_hash").unwrap();
        assert_eq!(status, "conflict");
        assert_eq!(error.as_deref(), Some("payload_hash_conflict"));
        assert_eq!(payload_hash, "payload-a");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn concurrent_identical_source_event_first_delivery_replays_without_error(pool: PgPool) {
        install_source_event_claim_barrier(&pool).await;

        let mut blocker = pool.acquire().await.unwrap();
        sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
            .bind(SOURCE_EVENT_CLAIM_TEST_LOCK)
            .execute(&mut *blocker)
            .await
            .unwrap();

        let spawn_capture = |pool: PgPool, application_name: &'static str| {
            tokio::spawn(async move {
                let mut connection = pool.acquire().await.unwrap();
                sqlx::query("SET SESSION AUTHORIZATION kengram_rt_native_mcp")
                    .execute(&mut *connection)
                    .await
                    .unwrap();
                sqlx::query("SELECT pg_catalog.set_config('application_name', $1, false)")
                    .bind(application_name)
                    .execute(&mut *connection)
                    .await
                    .unwrap();
                let row = gated_capture_on_connection(
                    &mut connection,
                    "agents/concurrent-source-claim",
                    "Concurrent byte-identical first deliveries share one durable source claim.",
                    Some((
                        "tests/concurrent-source-claim",
                        "same-first-delivery",
                        "same-payload",
                    )),
                    &json!([]),
                    None,
                )
                .await;
                let result = (
                    row.try_get::<uuid::Uuid, _>("thought_id").unwrap(),
                    row.try_get::<String, _>("action").unwrap(),
                    row.try_get::<String, _>("source_event_status").unwrap(),
                    row.try_get::<String, _>("source_event_action").unwrap(),
                );
                sqlx::query("RESET SESSION AUTHORIZATION")
                    .execute(&mut *connection)
                    .await
                    .unwrap();
                result
            })
        };

        let first = spawn_capture(pool.clone(), "source_claim_race_first");
        let second = spawn_capture(pool.clone(), "source_claim_race_second");
        wait_for_active_test_clients(&pool, "source_claim_race_", 2).await;
        sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
            .bind(SOURCE_EVENT_CLAIM_TEST_LOCK)
            .execute(&mut *blocker)
            .await
            .unwrap();

        let first = first.await.unwrap();
        let second = second.await.unwrap();
        assert_eq!(first.0, second.0);
        assert_eq!(first.2, "stored");
        assert_eq!(second.2, "stored");
        let mut dispositions = [(first.1, first.3), (second.1, second.3)];
        dispositions.sort();
        assert_eq!(
            dispositions,
            [
                ("exact_duplicate".to_string(), "replay".to_string()),
                ("inserted".to_string(), "stored".to_string()),
            ]
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn enqueue_targets_thought_kind_with_active_model(pool: PgPool) {
        let resp = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("queue me", "manual"),
        )
        .await
        .unwrap();

        // Inspect the queue row directly.
        let row =
            sqlx::query!(r#"SELECT target_kind, target_id, model_id FROM pending_embeddings"#,)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.target_kind, "thought");
        assert_eq!(row.target_id, resp.thought_id.into_uuid());
        assert_eq!(row.model_id, TEST_EMBEDDER_MODEL_ID);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn returns_existing_id_on_duplicate_content(pool: PgPool) {
        let first = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("same content", "manual"),
        )
        .await
        .unwrap();
        assert!(!first.is_duplicate);
        assert_eq!(kengram_storage::count_pending(&pool).await.unwrap(), 1);
        let tag_jobs = kengram_storage::fetch_pending_tag_jobs(&pool, 10)
            .await
            .unwrap();
        assert_eq!(tag_jobs.len(), 1);

        // Second capture with same content returns the existing id + duplicate flag.
        let second = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("same content", "manual"),
        )
        .await
        .unwrap();
        assert!(second.is_duplicate);
        assert_eq!(first.thought_id, second.thought_id);

        // No new jobs were enqueued — queues unchanged.
        assert_eq!(kengram_storage::count_pending(&pool).await.unwrap(), 1);
        let tag_jobs = kengram_storage::fetch_pending_tag_jobs(&pool, 10)
            .await
            .unwrap();
        assert_eq!(tag_jobs.len(), 1);
    }

    #[test]
    fn fingerprint_race_budget_is_two_seconds() {
        assert_eq!(FINGERPRINT_RACE_BUDGET, std::time::Duration::from_secs(2));
        let err = CaptureError::FingerprintRaceBudgetExceeded { budget_ms: 2000 };
        let s = err.to_string();
        assert!(
            s.contains("fingerprint_race_budget_exceeded"),
            "named error must be stable for callers: {s}"
        );
    }

    #[test]
    fn classifies_fingerprint_constraint_exact() {
        assert!(is_fingerprint_unique_violation_parts(
            Some("23505"),
            Some(THOUGHTS_CONTENT_FINGERPRINT_UNIQUE),
            "irrelevant message when constraint is present",
        ));
    }

    #[test]
    fn rejects_foreign_constraint_name_even_if_message_mentions_fingerprint() {
        // Online reindex: create …_unique_v2 alongside the old name, swap.
        // constraint() reports the new name; message may still contain the old.
        assert!(!is_fingerprint_unique_violation_parts(
            Some("23505"),
            Some("thoughts_content_fingerprint_unique_v2"),
            "duplicate key value violates unique constraint \"thoughts_content_fingerprint_unique\"",
        ));
        assert!(!is_fingerprint_unique_violation_parts(
            Some("23505"),
            Some("reviewer_other_unique_value"),
            "duplicate key value violates unique constraint \"reviewer_other_unique_value\"",
        ));
    }

    #[test]
    fn message_fallback_only_when_constraint_absent() {
        assert!(is_fingerprint_unique_violation_parts(
            Some("23505"),
            None,
            "duplicate key value violates unique constraint \"thoughts_content_fingerprint_unique\"",
        ));
        assert!(!is_fingerprint_unique_violation_parts(
            Some("23505"),
            None,
            "duplicate key value violates unique constraint \"reviewer_other_unique_value\"",
        ));
        assert!(!is_fingerprint_unique_violation_parts(
            Some("23503"),
            None,
            "thoughts_content_fingerprint_unique",
        ));
    }

    #[test]
    fn migration_declares_thoughts_content_fingerprint_unique() {
        // Assert the constant still names the constraint migration 0006 creates.
        // Renaming the migration constraint without this constant would silently
        // disable the classifier (the whole race fix reverts) while a hard-coded
        // self-compare of the constant would stay green.
        let migration = include_str!("../../../migrations/0006_collapse_to_thoughts.sql");
        assert!(
            migration.contains(THOUGHTS_CONTENT_FINGERPRINT_UNIQUE),
            "migration 0006 must declare constraint name matched by the classifier"
        );
        assert!(
            migration.contains("UNIQUE (content_fingerprint)"),
            "migration 0006 must unique-index content_fingerprint"
        );
    }

    /// Positive control: genuine validation failures still error (not swallowed
    /// as duplicates).
    #[sqlx::test(migrations = "../../migrations")]
    async fn genuine_failures_still_error(pool: PgPool) {
        let empty = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("", "manual"),
        )
        .await;
        assert!(matches!(empty, Err(CaptureError::EmptyContent)));

        let too_long = "x".repeat(MAX_CONTENT_LEN + 1);
        let err = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req(&too_long, "manual"),
        )
        .await;
        assert!(matches!(err, Err(CaptureError::ContentTooLong { .. })));
    }

    /// Concurrent same-content captures must never surface a raw unique
    /// violation to the caller; all must resolve to the same thought_id.
    /// Barrier forces a simultaneous start so the race is not schedule-lucky.
    #[sqlx::test(migrations = "../../migrations")]
    async fn concurrent_same_content_captures_are_idempotent(pool: PgPool) {
        let content = "concurrent fingerprint race content";
        let n = 8usize;
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for _ in 0..n {
            let pool = pool.clone();
            let content = content.to_string();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                capture(
                    &pool,
                    TEST_EMBEDDER_MODEL_ID,
                    Some(TEST_TAGGER_MODEL_ID),
                    req(&content, "manual"),
                )
                .await
            }));
        }
        let mut ids = Vec::new();
        let mut dup_flags = 0usize;
        for h in handles {
            let resp = h
                .await
                .expect("join")
                .expect("capture must not return raw DB unique error");
            if resp.is_duplicate {
                dup_flags += 1;
            }
            ids.push(resp.thought_id);
        }
        assert!(ids.iter().all(|id| *id == ids[0]));
        let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts WHERE content = $1")
            .bind(content)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(n, 1);
        // With 8 racers, typically >0 duplicates; allow all-new only if
        // serialization made the gate always win (still one row).
        let _ = dup_flags;
    }

    /// Neo CN probe: N same-content captures with DISTINCT source events must
    /// each leave a durable argus_source_events row when they report stored.
    /// Catches post-rollback fabrication (receipts without ledger rows).
    /// Barrier sync-starts all racers so fingerprint 23505 is forced, not flaky.
    #[sqlx::test(migrations = "../../migrations")]
    async fn distinct_source_event_racers_have_durable_ledger(pool: PgPool) {
        let content = "distinct source-event racer content v1";
        let n = 16usize;
        let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(n));
        let mut handles = Vec::new();
        for i in 0..n {
            let pool = pool.clone();
            let content = content.to_string();
            let barrier = barrier.clone();
            handles.push(tokio::spawn(async move {
                barrier.wait().await;
                let req = CaptureRequest {
                    content: content.clone(),
                    source: Source::new("manual").unwrap(),
                    scope: Some(Scope::new("agents/diesel").unwrap()),
                    metadata: None,
                    argus_source_event: Some(ArgusSourceEventRequest {
                        namespace: "tests/diesel-idempotency".to_string(),
                        source_ref: format!("racer-{i}"),
                        payload_hash: format!("hash-racer-{i}"),
                        metadata: None,
                    }),
                };
                capture(
                    &pool,
                    TEST_EMBEDDER_MODEL_ID,
                    Some(TEST_TAGGER_MODEL_ID),
                    req,
                )
                .await
            }));
        }
        let mut stored_receipts = 0usize;
        let mut ok = 0usize;
        for h in handles {
            let resp = h.await.expect("join").expect("capture must succeed");
            ok += 1;
            if let Some(ev) = &resp.argus_source_event {
                if ev.status == "stored" || ev.action == "stored" || ev.action == "duplicate_skip" {
                    // any successful disposition still needs a durable row
                    stored_receipts += 1;
                }
            }
        }
        assert_eq!(ok, n, "all racers must return Ok");
        let thoughts: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM thoughts WHERE content = $1 AND retracted_at IS NULL",
        )
        .bind(content)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(thoughts, 1, "exactly one thought for content");

        let ledger: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM argus_source_events WHERE namespace = $1")
                .bind("tests/diesel-idempotency")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            ledger as usize, n,
            "durable source-event rows must equal racer count (got {ledger} vs {n})"
        );
        assert_eq!(
            stored_receipts, n,
            "every racer must carry source-event disposition"
        );

        // Relation intents: one racer with a no-op empty intent still needs gate durability.
        // Bookkeeping: gate events should exist for at least the winner path.
        let _ = stored_receipts;
    }

    /// A 23505 on a DIFFERENT unique constraint must not be classified as fingerprint
    /// duplicate success — rethrow / surface as storage error.
    #[sqlx::test(migrations = "../../migrations")]
    async fn foreign_unique_23505_is_not_swallowed(pool: PgPool) {
        // Isolate a non-fingerprint unique index for the probe.
        sqlx::query(
            r#"
            CREATE UNIQUE INDEX IF NOT EXISTS reviewer_other_unique_value
            ON public.thoughts ((metadata->>'reviewer_probe'))
            WHERE metadata ? 'reviewer_probe'
            "#,
        )
        .execute(&pool)
        .await
        .unwrap();

        let first = CaptureRequest {
            content: "foreign unique A".to_string(),
            source: Source::new("manual").unwrap(),
            scope: None,
            metadata: Some(Metadata::from(json!({"reviewer_probe": "same-key"}))),
            argus_source_event: None,
        };
        capture(&pool, TEST_EMBEDDER_MODEL_ID, None, first)
            .await
            .expect("first insert ok");

        let second = CaptureRequest {
            content: "foreign unique B different content".to_string(),
            source: Source::new("manual").unwrap(),
            scope: None,
            metadata: Some(Metadata::from(json!({"reviewer_probe": "same-key"}))),
            argus_source_event: None,
        };
        let err = capture(&pool, TEST_EMBEDDER_MODEL_ID, None, second)
            .await
            .expect_err("must not treat foreign unique as fingerprint duplicate");
        match err {
            CaptureError::Storage(kengram_storage::StorageError::Database(ref e)) => {
                // Standalone classifier assert (not OR'd with always-true
                // message/sqlstate terms). Under the re-enter-gate path, a
                // misclassified foreign 23505 still returns Err after retry, so
                // expect_err alone cannot catch a gutting of the classifier.
                assert!(
                    !is_thoughts_content_fingerprint_unique_violation(e),
                    "foreign unique must not classify as fingerprint race: {e}"
                );
                if let sqlx::Error::Database(db) = e {
                    assert_eq!(db.code().as_deref(), Some("23505"));
                    let c = db.constraint().unwrap_or("");
                    assert_ne!(c, THOUGHTS_CONTENT_FINGERPRINT_UNIQUE);
                    assert!(
                        c.contains("reviewer_other_unique")
                            || e.to_string().contains("reviewer_other_unique"),
                        "expected foreign index name in error: constraint={c:?} err={e}"
                    );
                }
            }
            other => panic!("expected Storage Database error, got {other:?}"),
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn enqueues_embedding_and_tag_jobs_on_new_insert(pool: PgPool) {
        let resp = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            Some(TEST_TAGGER_MODEL_ID),
            req("dual-enqueue", "manual"),
        )
        .await
        .unwrap();

        let pending_embeds = kengram_storage::count_pending(&pool).await.unwrap();
        assert_eq!(pending_embeds, 1);

        let pending_tag_jobs = kengram_storage::fetch_pending_tag_jobs(&pool, 10)
            .await
            .unwrap();
        assert_eq!(pending_tag_jobs.len(), 1);
        assert_eq!(pending_tag_jobs[0].thought_id, resp.thought_id);
        assert_eq!(pending_tag_jobs[0].tagger_model_id, TEST_TAGGER_MODEL_ID);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn skips_tag_enqueue_when_tagger_disabled(pool: PgPool) {
        // tagger_model_id = None silent-disables the tag enqueue. Embedding
        // job still goes through.
        let resp = capture(
            &pool,
            TEST_EMBEDDER_MODEL_ID,
            None,
            req("no-tagger", "manual"),
        )
        .await
        .unwrap();

        assert!(!resp.is_duplicate);
        assert_eq!(kengram_storage::count_pending(&pool).await.unwrap(), 1);
        let tag_jobs = kengram_storage::fetch_pending_tag_jobs(&pool, 10)
            .await
            .unwrap();
        assert!(tag_jobs.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn invalid_candidate_model_queues_bge_and_audits_derived_bypass(pool: PgPool) {
        let metadata = json!({});
        let intents = json!([]);
        let candidate = vec![0.25_f32; 1024];
        let result = kengram_storage::corpus_hygiene::capture_thought_gated(
            &pool,
            kengram_storage::corpus_hygiene::GatedCaptureRequest {
                scope: "agents/model-probe",
                content: "A candidate with an unauthenticated model assertion must queue the reviewed BGE model.",
                source: "test",
                metadata: &metadata,
                raw_source_created_at: Some(OffsetDateTime::now_utc()),
                citation_origin_ids: &[],
                candidate_embedding: Some(&candidate),
                embedding_model_id: Some("wrong-model"),
                embedding_model_version: Some(7),
                bypass_reason: None,
                source_event_namespace: None,
                source_event_ref: None,
                source_event_payload_hash: None,
                source_event_metadata: None,
                relation_intents: &intents,
                tagger_model_id: None,
                claimed_producer_class: None,
                correlation_id: Some("wrong-model-probe"),
                force_keep_token: None,
            },
        )
        .await
        .unwrap();
        let thought_id = result.thought_id.unwrap();
        let queued_model: String = sqlx::query_scalar(
            "SELECT model_id FROM pending_embeddings WHERE target_kind='thought' AND target_id=$1",
        )
        .bind(thought_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(queued_model, "bge-m3:1024");
        let bypass_code: String = sqlx::query_scalar(
            "SELECT bypass_reason->>'code' FROM thought_ingest_gate_events WHERE id=$1",
        )
        .bind(result.gate_event_id.unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(bypass_code, "candidate_embedding_contract_mismatch");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn capture_relation_intent_cannot_override_from_endpoint(pool: PgPool) {
        let metadata = json!({});
        let bypass = json!({"code": "fixture"});
        let event_metadata = json!({});
        let intents = json!([{
            "action": "create",
            "from_thought_id": uuid::Uuid::new_v4(),
            "relation": "supports",
            "to_kind": "thought",
            "to_value": uuid::Uuid::new_v4(),
            "source": "agent"
        }]);
        let error = kengram_storage::corpus_hygiene::capture_thought_gated(
            &pool,
            kengram_storage::corpus_hygiene::GatedCaptureRequest {
                scope: "agents/relation-probe",
                content: "A capture caller cannot redirect its atomic relation away from the gated thought.",
                source: "test",
                metadata: &metadata,
                raw_source_created_at: Some(OffsetDateTime::now_utc()),
                citation_origin_ids: &[],
                candidate_embedding: None,
                embedding_model_id: Some("bge-m3:1024"),
                embedding_model_version: None,
                bypass_reason: Some(&bypass),
                source_event_namespace: Some("tests/capture-relations"),
                source_event_ref: Some("forged-from-endpoint"),
                source_event_payload_hash: Some("forged-from-endpoint"),
                source_event_metadata: Some(&event_metadata),
                relation_intents: &intents,
                tagger_model_id: None,
                claimed_producer_class: None,
                correlation_id: None,
                force_keep_token: None,
            },
        )
        .await
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("invalid_capture_relation_intent")
        );
        let counts: (i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM thoughts), (SELECT COUNT(*) FROM argus_source_events), (SELECT COUNT(*) FROM thought_ingest_gate_events)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(counts, (0, 0, 0));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn protected_atoms_use_exact_canonical_membership(pool: PgPool) {
        sqlx::query(
            "UPDATE corpus_hygiene_gate_settings SET mode='enforce' WHERE principal_name='kengram_rt_session'",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("SET SESSION AUTHORIZATION kengram_rt_session")
            .execute(&mut *connection)
            .await
            .unwrap();

        let mut outcomes = Vec::new();
        for (case_name, keeper_atom, candidate_atom) in [
            ("bare-count", "12", "1"),
            ("issue-number", "#12", "#1"),
            ("currency-amount", "$12", "$1"),
        ] {
            let scope = format!("agents/protected-{case_name}");
            let keeper = format!(
                "The operational queue reports {keeper_atom} pending items and this deliberately long shared description repeats the same stable words so semantic similarity stays exact while the protected fact differs."
            );
            let candidate = format!(
                "The operational queue reports {candidate_atom} pending items and this deliberately long shared description repeats the same stable words so semantic similarity stays exact while the protected fact differs."
            );
            let empty_intents = json!([]);
            let first = gated_capture_on_connection(
                &mut connection,
                &scope,
                &keeper,
                None,
                &empty_intents,
                None,
            )
            .await;
            let second = gated_capture_on_connection(
                &mut connection,
                &scope,
                &candidate,
                None,
                &empty_intents,
                None,
            )
            .await;
            outcomes.push((
                case_name,
                first.try_get::<String, _>("action").unwrap(),
                second.try_get::<String, _>("action").unwrap(),
                second.try_get::<uuid::Uuid, _>("gate_event_id").unwrap(),
                candidate_atom.to_string(),
            ));
        }

        sqlx::query("RESET SESSION AUTHORIZATION")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);

        for (case_name, first_action, second_action, gate_event_id, candidate_atom) in outcomes {
            assert_eq!(first_action, "inserted", "{case_name} keeper");
            assert_eq!(second_action, "inserted", "{case_name} candidate");
            let missing: Vec<String> = sqlx::query_scalar(
                "SELECT unnest(missing_protected_atoms) FROM thought_ingest_gate_events WHERE id=$1",
            )
            .bind(gate_event_id)
            .fetch_all(&pool)
            .await
            .unwrap();
            assert_eq!(missing, vec![candidate_atom], "{case_name}");
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn capture_replay_rejects_changed_canonical_relation_intent(pool: PgPool) {
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("SET SESSION AUTHORIZATION kengram_rt_native_mcp")
            .execute(&mut *connection)
            .await
            .unwrap();

        let empty_intents = json!([]);
        let target = gated_capture_on_connection(
            &mut connection,
            "agents/relation-replay",
            "Target thought for changed relation intent replay validation.",
            None,
            &empty_intents,
            None,
        )
        .await
        .try_get::<uuid::Uuid, _>("thought_id")
        .unwrap();
        let supports = json!([{
            "action": "create",
            "relation": "supports",
            "to_kind": "thought",
            "to_value": target,
            "source": "agent",
        }]);
        let references = json!([{
            "action": "create",
            "relation": "references",
            "to_kind": "thought",
            "to_value": target,
            "source": "agent",
        }]);
        let source_identity = Some(("tests/capture-replay", "changed-intent", "payload-a"));
        let first = gated_capture_on_connection(
            &mut connection,
            "agents/relation-replay",
            "Source thought whose durable replay intent must remain supports.",
            source_identity,
            &supports,
            None,
        )
        .await;
        let source_thought_id = first.try_get::<uuid::Uuid, _>("thought_id").unwrap();
        let replay = gated_capture_on_connection(
            &mut connection,
            "agents/relation-replay",
            "Source thought whose durable replay intent must remain supports.",
            source_identity,
            &references,
            None,
        )
        .await;
        let replay_action = replay.try_get::<String, _>("action").unwrap();

        sqlx::query("RESET SESSION AUTHORIZATION")
            .execute(&mut *connection)
            .await
            .unwrap();
        drop(connection);

        assert_eq!(replay_action, "source_event_conflict");
        let relations: Vec<String> = sqlx::query_scalar(
            "SELECT relation FROM thought_links WHERE from_thought_id=$1 AND deleted_at IS NULL ORDER BY relation",
        )
        .bind(source_thought_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(relations, vec!["supports"]);
        let recorded_relation: String = sqlx::query_scalar(
            "SELECT operations->0->>'relation' FROM thought_relation_request_events WHERE source_event_namespace='tests/capture-replay' AND source_event_ref='changed-intent'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(recorded_relation, "supports");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn runtime_role_duplicate_is_classified_by_gate_without_table_update(pool: PgPool) {
        let runtime_can_update: bool = sqlx::query_scalar(
            "SELECT has_table_privilege('kengram_rt_session', 'argus_source_events', 'UPDATE')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(!runtime_can_update);

        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("SET SESSION AUTHORIZATION kengram_rt_session")
            .execute(&mut *connection)
            .await
            .unwrap();
        let identity = Some(("tests/runtime-adapter", "duplicate-1", "payload-1"));
        let content = "An adapter-equivalent duplicate must be accepted and classified through the security-definer gate without direct source-event UPDATE privilege.";
        let first = gated_capture_on_connection(
            &mut connection,
            "agents/runtime-adapter",
            content,
            identity,
            &json!([]),
            None,
        )
        .await;
        let replay = gated_capture_on_connection(
            &mut connection,
            "agents/runtime-adapter",
            content,
            identity,
            &json!([]),
            None,
        )
        .await;
        let dispositions = (
            first.try_get::<String, _>("action").unwrap(),
            first.try_get::<String, _>("source_event_action").unwrap(),
            replay.try_get::<String, _>("action").unwrap(),
            replay.try_get::<String, _>("source_event_action").unwrap(),
        );
        sqlx::query("RESET SESSION AUTHORIZATION")
            .execute(&mut *connection)
            .await
            .unwrap();

        assert_eq!(
            dispositions,
            (
                "inserted".into(),
                "stored".into(),
                "exact_duplicate".into(),
                "replay".into()
            )
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn contended_family_lock_fails_open_and_queues_with_signal(pool: PgPool) {
        sqlx::query(
            "UPDATE corpus_hygiene_gate_settings SET mode='enforce' WHERE principal_name='kengram_rt_session'",
        )
        .execute(&pool)
        .await
        .unwrap();

        let agent_key = "blocked-lock-agent";
        let mut lock_connection = pool.acquire().await.unwrap();
        sqlx::query("BEGIN")
            .execute(&mut *lock_connection)
            .await
            .unwrap();
        sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1, 727061667331::bigint))")
            .bind(agent_key)
            .execute(&mut *lock_connection)
            .await
            .unwrap();

        let mut capture_connection = pool.acquire().await.unwrap();
        sqlx::query("SET SESSION AUTHORIZATION kengram_rt_session")
            .execute(&mut *capture_connection)
            .await
            .unwrap();
        let started = std::time::Instant::now();
        let row = gated_capture_on_connection(
            &mut capture_connection,
            &format!("agents/{agent_key}"),
            "A contended family lock must keep this capture and queue ordinary asynchronous embedding and tag work instead of waiting.",
            None,
            &json!([]),
            Some(TEST_TAGGER_MODEL_ID),
        )
        .await;
        let elapsed = started.elapsed();
        let thought_id = row.try_get::<uuid::Uuid, _>("thought_id").unwrap();
        let action = row.try_get::<String, _>("action").unwrap();
        let gate_event_id = row.try_get::<uuid::Uuid, _>("gate_event_id").unwrap();
        sqlx::query("RESET SESSION AUTHORIZATION")
            .execute(&mut *capture_connection)
            .await
            .unwrap();
        sqlx::query("ROLLBACK")
            .execute(&mut *lock_connection)
            .await
            .unwrap();
        drop(capture_connection);
        drop(lock_connection);

        assert_eq!(action, "fail_open_insert");
        assert!(elapsed < std::time::Duration::from_secs(1));
        let pending_embedding: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pending_embeddings WHERE target_kind='thought' AND target_id=$1 AND model_id='bge-m3:1024'",
        )
        .bind(thought_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let pending_tag: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pending_tags WHERE thought_id=$1")
                .bind(thought_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let signal: (String, String) = sqlx::query_as(
            "SELECT action, bypass_reason->>'code' FROM thought_ingest_gate_events WHERE id=$1",
        )
        .bind(gate_event_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending_embedding, 1);
        assert_eq!(pending_tag, 1);
        assert_eq!(
            signal,
            (
                "fail_open_insert".into(),
                "similarity_lock_contended".into()
            )
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn comparison_statement_timeout_fails_open_and_queues(pool: PgPool) {
        let mut lock_connection = pool.acquire().await.unwrap();
        sqlx::query("BEGIN")
            .execute(&mut *lock_connection)
            .await
            .unwrap();
        sqlx::query("LOCK TABLE thought_embeddings_bge_m3 IN ACCESS EXCLUSIVE MODE")
            .execute(&mut *lock_connection)
            .await
            .unwrap();

        let metadata = json!({});
        let intents = json!([]);
        let candidate = vec![0.03125_f32; 1024];
        let started = std::time::Instant::now();
        let result = kengram_storage::corpus_hygiene::capture_thought_gated(
            &pool,
            kengram_storage::corpus_hygiene::GatedCaptureRequest {
                scope: "agents/comparison-timeout",
                content: "A blocked similarity table read must become a durable queued fail-open capture before the total caller deadline.",
                source: "test",
                metadata: &metadata,
                raw_source_created_at: Some(OffsetDateTime::now_utc()),
                citation_origin_ids: &[],
                candidate_embedding: Some(&candidate),
                embedding_model_id: Some("bge-m3:1024"),
                embedding_model_version: Some(1),
                bypass_reason: None,
                source_event_namespace: None,
                source_event_ref: None,
                source_event_payload_hash: None,
                source_event_metadata: None,
                relation_intents: &intents,
                tagger_model_id: Some(TEST_TAGGER_MODEL_ID),
                claimed_producer_class: None,
                correlation_id: Some("comparison-timeout"),
                force_keep_token: None,
            },
        )
        .await
        .unwrap();
        let elapsed = started.elapsed();
        sqlx::query("ROLLBACK")
            .execute(&mut *lock_connection)
            .await
            .unwrap();
        drop(lock_connection);

        assert_eq!(result.action, "fail_open_insert");
        assert!(elapsed < std::time::Duration::from_secs(1));
        let thought_id = result.thought_id.unwrap();
        let pending_embedding: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM pending_embeddings WHERE target_kind='thought' AND target_id=$1",
        )
        .bind(thought_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let pending_tag: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM pending_tags WHERE thought_id=$1")
                .bind(thought_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        let bypass: (String, String) = sqlx::query_as(
            "SELECT bypass_reason->>'code', bypass_reason->>'sqlstate' FROM thought_ingest_gate_events WHERE id=$1",
        )
        .bind(result.gate_event_id.unwrap())
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(pending_embedding, 1);
        assert_eq!(pending_tag, 1);
        assert_eq!(
            bypass,
            ("similarity_comparison_unavailable".into(), "57014".into())
        );
    }

    async fn capture_at(
        pool: &PgPool,
        scope: &str,
        content: String,
        source_created_at: OffsetDateTime,
        candidate_embedding: Option<Vec<f32>>,
    ) -> CaptureResponse {
        capture_with_gate_options(
            pool,
            TEST_EMBEDDER_MODEL_ID,
            None,
            CaptureRequest {
                content,
                source: Source::new("test").unwrap(),
                scope: Some(Scope::new(scope).unwrap()),
                metadata: None,
                argus_source_event: None,
            },
            CaptureGateOptions {
                source_created_at: Some(source_created_at),
                candidate_embedding,
                bypass_reason: None,
                relation_intents: vec![],
                claimed_producer_class: None,
                correlation_id: None,
            },
        )
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cited_capture_strips_content_and_uses_oldest_origin(pool: PgPool) {
        let scope = "agents/cited-oldest";
        let oldest = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let newer = oldest + time::Duration::days(4);
        let explicit = oldest + time::Duration::days(8);
        let first = capture_at(
            &pool,
            scope,
            "oldest cited origin".to_string(),
            oldest,
            None,
        )
        .await;
        let second = capture_at(&pool, scope, "newer cited origin".to_string(), newer, None).await;
        let stored = "A paraphrase carries both source ages without citation bytes";
        let raw = format!(
            "{stored} {} {}",
            crate::citation::format_citation(second.thought_id),
            crate::citation::format_citation(first.thought_id),
        );
        let response = capture_at(&pool, scope, raw, explicit, None).await;

        assert_eq!(
            response.resolved_origin_ids,
            vec![second.thought_id, first.thought_id]
        );
        assert_eq!(response.derived_source_age, oldest);
        assert_eq!(response.persisted_source_age, oldest);
        assert_eq!(response.born_on, oldest);
        assert_eq!(response.source_age_outcome, "new_propagated");
        assert_eq!(
            response.citation,
            crate::citation::format_citation(response.thought_id)
        );

        let persisted: (String, OffsetDateTime) =
            sqlx::query_as("SELECT content, created_at FROM thoughts WHERE id = $1")
                .bind(response.thought_id.into_uuid())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(persisted.0, stored);
        assert_eq!(persisted.1, oldest);
        assert!(!persisted.0.contains("[kg:"));
        let side_effects: (i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM thought_links), (SELECT COUNT(*) FROM thoughts WHERE retracted_at IS NOT NULL)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(side_effects, (0, 0));

        // Transactional revalidation is the linearization point, not an
        // optimistic pre-read.  Hold the source-event claim after the Rust
        // adapter has locked the cited origin, then prove the real retraction
        // caller cannot pass that row lock until capture commits.
        install_source_event_claim_barrier(&pool).await;
        let mut blocker = pool.acquire().await.unwrap();
        sqlx::query("SELECT pg_catalog.pg_advisory_lock($1)")
            .bind(SOURCE_EVENT_CLAIM_TEST_LOCK)
            .execute(&mut *blocker)
            .await
            .unwrap();

        let connect_options = pool.connect_options().as_ref().clone();
        let capture_pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(1)
            .after_connect(|connection, _metadata| {
                Box::pin(async move {
                    sqlx::query("SELECT pg_catalog.set_config('application_name', $1, false)")
                        .bind("cited_origin_revalidation_capture")
                        .execute(&mut *connection)
                        .await?;
                    Ok(())
                })
            })
            .connect_with(connect_options)
            .await
            .unwrap();
        let concurrent_origin = capture_at(
            &pool,
            scope,
            "origin held live through cited capture commit".to_string(),
            newer,
            None,
        )
        .await;
        let capture_pool_for_task = capture_pool.clone();
        let concurrent_citation = crate::citation::format_citation(concurrent_origin.thought_id);
        let capture_task = tokio::spawn(async move {
            capture(
                &capture_pool_for_task,
                TEST_EMBEDDER_MODEL_ID,
                None,
                CaptureRequest {
                    content: format!(
                        "transactionally revalidated cited body {concurrent_citation}"
                    ),
                    source: Source::new("test").unwrap(),
                    scope: Some(Scope::new("agents/cited-oldest").unwrap()),
                    metadata: None,
                    argus_source_event: Some(ArgusSourceEventRequest {
                        namespace: "tests/cited-origin-revalidation".to_string(),
                        source_ref: "capture-while-retracting".to_string(),
                        payload_hash: "f".repeat(64),
                        metadata: None,
                    }),
                },
            )
            .await
        });
        wait_for_active_test_clients(&pool, "cited_origin_revalidation_capture", 1).await;

        let retract_pool = pool.clone();
        let concurrent_origin_id = concurrent_origin.thought_id;
        let retract_task = tokio::spawn(async move {
            crate::retract::retract_thought(
                &retract_pool,
                crate::retract::RetractThoughtRequest {
                    thought_id: concurrent_origin_id,
                    reason: Some("transactional origin revalidation control".to_string()),
                },
            )
            .await
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let retraction_waited_for_capture = !retract_task.is_finished();

        sqlx::query("SELECT pg_catalog.pg_advisory_unlock($1)")
            .bind(SOURCE_EVENT_CLAIM_TEST_LOCK)
            .execute(&mut *blocker)
            .await
            .unwrap();
        let concurrent_capture = capture_task.await.unwrap().unwrap();
        let concurrent_retraction = retract_task.await.unwrap().unwrap();
        capture_pool.close().await;
        assert!(
            retraction_waited_for_capture,
            "origin retraction must wait for the cited capture's FOR SHARE lock"
        );
        assert_eq!(concurrent_capture.derived_source_age, newer);
        assert!(concurrent_retraction.retracted);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cited_duplicate_reports_age_conflict_without_update(pool: PgPool) {
        let scope = "agents/cited-duplicate";
        let old = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let fresh = old + time::Duration::days(5);
        let origin = capture_at(
            &pool,
            scope,
            "duplicate conflict origin".to_string(),
            old,
            None,
        )
        .await;
        let content = "The persisted duplicate body must remain byte and timestamp immutable";
        let existing = capture_at(&pool, scope, content.to_string(), fresh, None).await;
        let before: (String, OffsetDateTime) =
            sqlx::query_as("SELECT content, created_at FROM thoughts WHERE id = $1")
                .bind(existing.thought_id.into_uuid())
                .fetch_one(&pool)
                .await
                .unwrap();

        let duplicate = capture_at(
            &pool,
            scope,
            format!(
                "{content} {}",
                crate::citation::format_citation(origin.thought_id)
            ),
            fresh + time::Duration::days(1),
            None,
        )
        .await;
        let after: (String, OffsetDateTime) =
            sqlx::query_as("SELECT content, created_at FROM thoughts WHERE id = $1")
                .bind(existing.thought_id.into_uuid())
                .fetch_one(&pool)
                .await
                .unwrap();

        assert!(duplicate.is_duplicate);
        assert_eq!(duplicate.dedup_kind.as_deref(), Some("exact_duplicate"));
        assert_eq!(duplicate.thought_id, existing.thought_id);
        assert_eq!(duplicate.derived_source_age, old);
        assert_eq!(duplicate.persisted_source_age, fresh);
        assert_eq!(duplicate.source_age_outcome, "duplicate_conflict");
        assert_eq!(before, after, "duplicate must not mutate content or age");
        let thought_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(thought_count, 2, "origin plus immutable duplicate row");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn source_event_payload_conflict_does_not_claim_new_propagation(pool: PgPool) {
        let scope = "agents/source-event-outcome-review";
        let old = OffsetDateTime::from_unix_timestamp(1_700_000_000).unwrap();
        let newer = old + time::Duration::days(5);
        let old_origin = capture_at(
            &pool,
            scope,
            "source event old origin".to_string(),
            old,
            None,
        )
        .await;
        let new_origin = capture_at(
            &pool,
            scope,
            "source event new origin".to_string(),
            newer,
            None,
        )
        .await;

        let run = |origin: ThoughtId, payload_hash: String| {
            capture_with_gate_options(
                &pool,
                TEST_EMBEDDER_MODEL_ID,
                None,
                CaptureRequest {
                    content: format!(
                        "source event conflict body {}",
                        crate::citation::format_citation(origin)
                    ),
                    source: Source::new("test").unwrap(),
                    scope: Some(Scope::new(scope).unwrap()),
                    metadata: None,
                    argus_source_event: Some(ArgusSourceEventRequest {
                        namespace: "tests/source-event-outcome-review".to_string(),
                        source_ref: "same-identity".to_string(),
                        payload_hash,
                        metadata: None,
                    }),
                },
                CaptureGateOptions {
                    source_created_at: None,
                    candidate_embedding: None,
                    bypass_reason: None,
                    relation_intents: vec![],
                    claimed_producer_class: None,
                    correlation_id: None,
                },
            )
        };

        let first = run(old_origin.thought_id, "a".repeat(64)).await.unwrap();
        assert_eq!(first.source_age_outcome, "new_propagated");
        assert!(!first.is_duplicate);
        let thought_count_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts")
            .fetch_one(&pool)
            .await
            .unwrap();

        let conflict = run(new_origin.thought_id, "b".repeat(64)).await.unwrap();
        let conflict_event = conflict.argus_source_event.as_ref().unwrap();
        assert_eq!(conflict_event.action, "conflict");
        assert_eq!(conflict_event.status, "conflict");
        assert_eq!(conflict.thought_id, first.thought_id);
        assert_eq!(conflict.matched_thought_id, Some(first.thought_id));
        assert!(!conflict.is_duplicate, "preserve the conflict contract");
        assert_eq!(conflict.dedup_kind, None);
        assert_eq!(
            conflict.source_age_outcome, "duplicate_conflict",
            "a payload conflict selecting the prior thought must not claim a new propagated row"
        );
        let thought_count_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(thought_count_after, thought_count_before);
    }

    async fn enable_semantic_gate_for_current_user(pool: &PgPool) {
        let principal: String = sqlx::query_scalar("SELECT session_user::text")
            .fetch_one(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM corpus_hygiene_gate_settings WHERE principal_name = $1")
            .bind(&principal)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            r#"
            UPDATE corpus_hygiene_producer_principals
            SET producer_class = 'test_semantic_capture',
                requires_source_created_at = false,
                keep_only = false,
                enforce_eligible = true
            WHERE principal_name = $1
            "#,
        )
        .bind(&principal)
        .execute(pool)
        .await
        .unwrap();
        sqlx::query(
            r#"
            INSERT INTO corpus_hygiene_gate_settings (
                principal_name, producer_class, profile_revision, mode
            ) VALUES ($1, 'test_semantic_capture', 1, 'enforce')
            "#,
        )
        .bind(&principal)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn cited_capture_internal_vector_semantic_duplicate_agreement_and_conflict(pool: PgPool) {
        enable_semantic_gate_for_current_user(&pool).await;
        let scope = "agents/cited-semantic";
        let agreed_age = OffsetDateTime::from_unix_timestamp(1_736_467_200).unwrap();
        let conflict_age = agreed_age + time::Duration::days(1);
        let agreed_origin = capture_at(
            &pool,
            scope,
            "semantic agreement origin".to_string(),
            agreed_age,
            None,
        )
        .await;
        let conflict_origin = capture_at(
            &pool,
            scope,
            "semantic conflict origin".to_string(),
            conflict_age,
            None,
        )
        .await;
        let vector = vec![0.03125_f32; 1024];
        assert!(vector.iter().all(|value| value.is_finite()));

        let keeper_text = "The operational handbook records a durable source age for this deliberately long semantic duplicate fixture, while every meaningful protected token and polarity remains unchanged across equivalent restatements.";
        let keeper = capture_at(
            &pool,
            scope,
            format!(
                "{keeper_text} {}",
                crate::citation::format_citation(agreed_origin.thought_id)
            ),
            agreed_age + time::Duration::days(2),
            Some(vector.clone()),
        )
        .await;
        assert!(!keeper.is_duplicate);
        let counts_before: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM thoughts), (SELECT COUNT(*) FROM thought_ingest_gate_events), (SELECT COUNT(*) FROM pending_embeddings), (SELECT COUNT(*) FROM thought_links)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let agreement = capture_at(
            &pool,
            scope,
            format!(
                "The operational handbook records a durable source age for this deliberately long semantic duplicate fixture while every meaningful protected token and polarity remains unchanged across equivalent restatements. {}",
                crate::citation::format_citation(agreed_origin.thought_id)
            ),
            agreed_age + time::Duration::days(3),
            Some(vector.clone()),
        )
        .await;
        assert!(agreement.is_duplicate);
        assert_eq!(agreement.dedup_kind.as_deref(), Some("semantic_duplicate"));
        assert_eq!(agreement.thought_id, keeper.thought_id);
        assert_eq!(agreement.source_age_outcome, "duplicate_agrees");
        assert!(agreement.gate_event_id.is_some());
        assert!(agreement.similarity.is_some_and(f64::is_finite));

        let conflict = capture_at(
            &pool,
            scope,
            format!(
                "The operational handbook records a durable source age for this deliberately long semantic duplicate fixture while every meaningful protected token and polarity remains unchanged across equivalent restatements. {}",
                crate::citation::format_citation(conflict_origin.thought_id)
            ),
            agreed_age + time::Duration::days(3),
            Some(vector),
        )
        .await;
        assert!(conflict.is_duplicate, "{conflict:?}");
        assert_eq!(conflict.dedup_kind.as_deref(), Some("semantic_duplicate"));
        assert_eq!(conflict.thought_id, keeper.thought_id);
        assert_eq!(conflict.source_age_outcome, "duplicate_conflict");
        assert_eq!(conflict.derived_source_age, conflict_age);
        assert_eq!(conflict.persisted_source_age, agreed_age);
        assert!(conflict.gate_event_id.is_some());
        assert_ne!(conflict.gate_event_id, agreement.gate_event_id);
        assert!(conflict.similarity.is_some_and(f64::is_finite));

        let counts_after: (i64, i64, i64, i64) = sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM thoughts), (SELECT COUNT(*) FROM thought_ingest_gate_events), (SELECT COUNT(*) FROM pending_embeddings), (SELECT COUNT(*) FROM thought_links)",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(counts_after.0, counts_before.0);
        assert_eq!(counts_after.2, counts_before.2);
        assert_eq!(counts_after.3, counts_before.3);
        assert_eq!(
            counts_after.1,
            counts_before.1 + 2,
            "each semantic decision records one gate event but no corpus row/job/relation"
        );
        let persisted: (String, OffsetDateTime) =
            sqlx::query_as("SELECT content, created_at FROM thoughts WHERE id = $1")
                .bind(keeper.thought_id.into_uuid())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(persisted.0, keeper_text);
        assert_eq!(persisted.1, agreed_age);
        let decisions: Vec<(uuid::Uuid, String, Option<String>, Option<f64>, String)> =
            sqlx::query_as(
                r#"
            SELECT id, action, bypass_reason->>'code', similarity, embedding_model_id
            FROM thought_ingest_gate_events
            WHERE id = ANY($1::uuid[])
            ORDER BY id
            "#,
            )
            .bind(vec![
                agreement.gate_event_id.unwrap(),
                conflict.gate_event_id.unwrap(),
            ])
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(decisions.len(), 2);
        for (_id, action, bypass_code, similarity, model_id) in decisions {
            assert_eq!(action, "semantic_duplicate");
            assert_eq!(bypass_code, None, "a real vector must not take bypass");
            assert!(similarity.is_some_and(f64::is_finite));
            assert_eq!(model_id, "bge-m3:1024");
        }
    }
}
