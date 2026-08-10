//! Typed callers for the migration-0030 database chokepoints.
//!
//! These queries intentionally use `sqlx::query` rather than `query!`: the
//! gate accepts pgvector's `vector(1024)` type and is introduced in the same
//! change, so there is no checked-in offline description until migration
//! integration runs.  Every value remains parameter-bound.

use crate::StorageError;
use pgvector::Vector;
use sqlx::{PgPool, Row};
use std::collections::HashMap;
use time::OffsetDateTime;
use uuid::Uuid;

/// Per-transaction statement_timeout for the gate INSERT path only.
///
/// Overrides any lower global/session statement_timeout for the duration of
/// this transaction (`set_config(..., true)` is local). Sized for durable
/// gated insert of large content without relying on a synchronous embedder;
/// embedding is async via pending_embeddings + worker drain.
/// Must stay under the MCP capture total deadline (1s) with room for commit.
const CAPTURE_GATE_STATEMENT_TIMEOUT: &str = "800ms";

#[derive(Debug, Clone)]
pub struct GatedCaptureRequest<'a> {
    pub scope: &'a str,
    pub content: &'a str,
    pub source: &'a str,
    pub metadata: &'a serde_json::Value,
    /// Raw caller-supplied instant. It is validated against the one
    /// transaction timestamp before cited-origin ages enter the minimum fold.
    pub raw_source_created_at: Option<OffsetDateTime>,
    /// Distinct citation origins in first-occurrence order.
    pub citation_origin_ids: &'a [Uuid],
    pub candidate_embedding: Option<&'a [f32]>,
    pub embedding_model_id: Option<&'a str>,
    pub embedding_model_version: Option<i32>,
    pub bypass_reason: Option<&'a serde_json::Value>,
    pub source_event_namespace: Option<&'a str>,
    pub source_event_ref: Option<&'a str>,
    pub source_event_payload_hash: Option<&'a str>,
    pub source_event_metadata: Option<&'a serde_json::Value>,
    pub relation_intents: &'a serde_json::Value,
    pub tagger_model_id: Option<&'a str>,
    pub claimed_producer_class: Option<&'a str>,
    pub correlation_id: Option<&'a str>,
    pub force_keep_token: Option<&'a str>,
}

#[derive(Debug, Clone)]
pub struct GatedCaptureResult {
    pub thought_id: Option<Uuid>,
    pub action: String,
    pub matched_thought_id: Option<Uuid>,
    pub similarity: Option<f64>,
    pub threshold: f64,
    /// Source age derived for this request. On completed source-event replay,
    /// this is the already-persisted thought age: replay does not revalidate
    /// current origin liveness.
    pub derived_created_at: OffsetDateTime,
    pub effective_created_at: OffsetDateTime,
    pub persisted_created_at: OffsetDateTime,
    pub observed_at: OffsetDateTime,
    pub source_event_status: Option<String>,
    pub source_event_action: Option<String>,
    pub relation_results: serde_json::Value,
    pub gate_event_id: Option<Uuid>,
}

#[derive(Debug)]
struct SourceEventPreflight {
    /// Existing source-event identity must be classified by the gate before
    /// any liveness-dependent citation decision. This includes payload or
    /// canonical-relation conflicts as well as exact completed events.
    precedes_origin_validation: bool,
    /// A completed, exact, relation-free event can be returned read-only from
    /// the durable source-event receipt. Relation-bearing requests still go
    /// through the security-definer gate because runtime callers cannot read
    /// the canonical relation ledger directly.
    replay: Option<GatedCaptureResult>,
}

async fn inspect_source_event_before_origin_validation(
    tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
    request: &GatedCaptureRequest<'_>,
    observed_at: OffsetDateTime,
) -> Result<SourceEventPreflight, StorageError> {
    let (Some(namespace), Some(source_ref), Some(payload_hash)) = (
        request.source_event_namespace,
        request.source_event_ref,
        request.source_event_payload_hash,
    ) else {
        return Ok(SourceEventPreflight {
            precedes_origin_validation: false,
            replay: None,
        });
    };

    let existing: Option<(String, String, Option<Uuid>, serde_json::Value)> = sqlx::query_as(
        r#"
        SELECT payload_hash, status, thought_id, metadata
        FROM public.argus_source_events
        WHERE namespace = $1 AND source_ref = $2
        "#,
    )
    .bind(namespace)
    .bind(source_ref)
    .fetch_optional(&mut **tx)
    .await?;
    let Some((stored_hash, status, thought_id, metadata)) = existing else {
        return Ok(SourceEventPreflight {
            precedes_origin_validation: false,
            replay: None,
        });
    };

    // A mismatched payload is authoritative source-event conflict evidence,
    // even when an interrupted producer left the row without a thought.
    if stored_hash != payload_hash {
        return Ok(SourceEventPreflight {
            precedes_origin_validation: true,
            replay: None,
        });
    }
    let Some(thought_id) = thought_id else {
        return Ok(SourceEventPreflight {
            precedes_origin_validation: false,
            replay: None,
        });
    };

    let corpus_hygiene = metadata.get("corpus_hygiene");
    let relation_results = corpus_hygiene.and_then(|receipt| receipt.get("relation_results"));
    let gate_event_id = corpus_hygiene
        .and_then(|receipt| receipt.get("gate_event_id"))
        .and_then(serde_json::Value::as_str)
        .and_then(|value| Uuid::parse_str(value).ok());
    let request_has_no_relations = request
        .relation_intents
        .as_array()
        .is_some_and(Vec::is_empty);
    let receipt_has_no_relations = relation_results
        .and_then(serde_json::Value::as_array)
        .is_some_and(Vec::is_empty);

    // Runtime roles deliberately cannot read thought_relation_request_events.
    // Only the unambiguous no-relation receipt can therefore return without
    // invoking the security-definer gate. Relation-bearing (or legacy/invalid)
    // receipts retain the gate's canonical relation-identity authority, but
    // still bypass origin liveness below.
    if !request_has_no_relations || !receipt_has_no_relations || gate_event_id.is_none() {
        return Ok(SourceEventPreflight {
            precedes_origin_validation: true,
            replay: None,
        });
    }

    let persisted_created_at: OffsetDateTime =
        sqlx::query_scalar("SELECT created_at FROM public.thoughts WHERE id = $1")
            .bind(thought_id)
            .fetch_one(&mut **tx)
            .await?;
    let action = if status == "skipped" {
        "semantic_duplicate"
    } else {
        "exact_duplicate"
    };
    Ok(SourceEventPreflight {
        precedes_origin_validation: true,
        replay: Some(GatedCaptureResult {
            thought_id: Some(thought_id),
            action: action.to_string(),
            matched_thought_id: Some(thought_id),
            similarity: None,
            // Threshold is not an MCP response field. The durable replay path
            // deliberately avoids privileged settings-table reads.
            threshold: 0.0,
            derived_created_at: persisted_created_at,
            effective_created_at: persisted_created_at,
            persisted_created_at,
            observed_at,
            source_event_status: Some(status),
            source_event_action: Some("replay".to_string()),
            relation_results: serde_json::Value::Array(Vec::new()),
            gate_event_id,
        }),
    })
}

pub async fn capture_thought_gated(
    pool: &PgPool,
    request: GatedCaptureRequest<'_>,
) -> Result<GatedCaptureResult, StorageError> {
    let vector = request
        .candidate_embedding
        .map(|values| Vector::from(values.to_vec()));
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT set_config('statement_timeout', $1, true)")
        .bind(CAPTURE_GATE_STATEMENT_TIMEOUT)
        .execute(&mut *tx)
        .await?;

    let observed_at: OffsetDateTime = sqlx::query_scalar("SELECT transaction_timestamp()")
        .fetch_one(&mut *tx)
        .await?;

    // This is deliberately a plain SELECT: runtime callers have table SELECT
    // but neither FOR UPDATE nor FOR SHARE privilege on argus_source_events.
    // The no-relation completed replay path is fully read-only; every other
    // existing identity still reaches the security-definer gate for canonical
    // replay/conflict classification.
    let mut source_event_preflight =
        inspect_source_event_before_origin_validation(&mut tx, &request, observed_at).await?;
    if let Some(replay) = source_event_preflight.replay.take() {
        tx.commit().await?;
        return Ok(replay);
    }
    let mut source_event_precedes_origin_validation =
        source_event_preflight.precedes_origin_validation;

    if !source_event_precedes_origin_validation
        && let Some(explicit) = request.raw_source_created_at
        && explicit > observed_at + time::Duration::minutes(5)
    {
        // Close the small read-committed race where a concurrent claimant
        // completed this exact event after the first preflight. A completion
        // that now exists retains precedence over raw-time validation.
        let mut refreshed =
            inspect_source_event_before_origin_validation(&mut tx, &request, observed_at).await?;
        if let Some(replay) = refreshed.replay.take() {
            tx.commit().await?;
            return Ok(replay);
        }
        source_event_precedes_origin_validation = refreshed.precedes_origin_validation;
        if !source_event_precedes_origin_validation {
            return Err(StorageError::SourceCreatedAtTooFarInFuture {
                source_created_at: explicit,
                observed_at,
            });
        }
    }

    let mut effective_source_created_at = if source_event_precedes_origin_validation {
        // The frozen SQL gate validates producer-required source time before
        // it classifies an existing source-event conflict. Use this same
        // transaction clock as the neutral value; the adapter replaces the
        // derived replay/conflict age with the persisted thought age below.
        Some(observed_at)
    } else {
        request.raw_source_created_at
    };
    if !source_event_precedes_origin_validation && !request.citation_origin_ids.is_empty() {
        let rows: Vec<(Uuid, String, OffsetDateTime, Option<OffsetDateTime>)> = sqlx::query_as(
            r#"
            SELECT id, scope, created_at, retracted_at
            FROM public.thoughts
            WHERE id = ANY($1::uuid[])
            FOR SHARE
            "#,
        )
        .bind(request.citation_origin_ids)
        .fetch_all(&mut *tx)
        .await?;
        let by_id: HashMap<Uuid, (String, OffsetDateTime, Option<OffsetDateTime>)> = rows
            .into_iter()
            .map(|(id, scope, created_at, retracted_at)| (id, (scope, created_at, retracted_at)))
            .collect();

        // Origin row locks serialize retraction, while this second source-
        // event read closes a concurrent-completion window before any
        // liveness failure can be returned.
        let mut refreshed =
            inspect_source_event_before_origin_validation(&mut tx, &request, observed_at).await?;
        if let Some(replay) = refreshed.replay.take() {
            tx.commit().await?;
            return Ok(replay);
        }
        source_event_precedes_origin_validation = refreshed.precedes_origin_validation;
        if source_event_precedes_origin_validation {
            effective_source_created_at = Some(observed_at);
        }

        if !source_event_precedes_origin_validation {
            for origin_id in request.citation_origin_ids {
                let Some((actual_scope, created_at, retracted_at)) = by_id.get(origin_id) else {
                    return Err(StorageError::CitationOriginNotFound(*origin_id));
                };
                if actual_scope != request.scope {
                    return Err(StorageError::CitationOriginScopeMismatch {
                        thought_id: *origin_id,
                        expected_scope: request.scope.to_string(),
                        actual_scope: actual_scope.clone(),
                    });
                }
                if retracted_at.is_some() {
                    return Err(StorageError::CitationOriginRetracted(*origin_id));
                }
                effective_source_created_at = Some(match effective_source_created_at {
                    Some(current) => current.min(*created_at),
                    None => *created_at,
                });
            }
        }
    }
    if !source_event_precedes_origin_validation && effective_source_created_at.is_none() {
        effective_source_created_at = Some(observed_at);
    }

    let row = sqlx::query(
        r#"
        SELECT *
        FROM public.capture_thought_gated(
            $1, $2, $3, $4, $5, $6::vector, $7, $8, $9,
            $10, $11, $12, $13, $14, $15, $16, $17, $18
        )
        "#,
    )
    .bind(request.scope)
    .bind(request.content)
    .bind(request.source)
    .bind(request.metadata)
    .bind(effective_source_created_at)
    .bind(vector)
    .bind(request.embedding_model_id)
    .bind(request.embedding_model_version)
    .bind(request.bypass_reason)
    .bind(request.source_event_namespace)
    .bind(request.source_event_ref)
    .bind(request.source_event_payload_hash)
    .bind(request.source_event_metadata)
    .bind(request.relation_intents)
    .bind(request.tagger_model_id)
    .bind(request.claimed_producer_class)
    .bind(request.correlation_id)
    .bind(request.force_keep_token)
    .fetch_one(&mut *tx)
    .await?;

    let thought_id: Option<Uuid> = row.try_get("thought_id")?;
    let matched_thought_id: Option<Uuid> = row.try_get("matched_thought_id")?;
    let effective_created_at: OffsetDateTime = row.try_get("effective_created_at")?;
    let persisted_id = thought_id.or(matched_thought_id).ok_or_else(|| {
        StorageError::Database(sqlx::Error::Protocol(
            "capture gate returned no persisted thought identity".to_string(),
        ))
    })?;
    let persisted_created_at: OffsetDateTime =
        sqlx::query_scalar("SELECT created_at FROM public.thoughts WHERE id = $1")
            .bind(persisted_id)
            .fetch_one(&mut *tx)
            .await?;

    let result = GatedCaptureResult {
        thought_id,
        action: row.try_get("action")?,
        matched_thought_id,
        similarity: row.try_get("similarity")?,
        threshold: row.try_get("threshold")?,
        derived_created_at: if source_event_precedes_origin_validation {
            persisted_created_at
        } else {
            effective_created_at
        },
        effective_created_at,
        persisted_created_at,
        observed_at: row.try_get("observed_at")?,
        source_event_status: row.try_get("source_event_status")?,
        source_event_action: row.try_get("source_event_action")?,
        relation_results: row.try_get("relation_results")?,
        gate_event_id: row.try_get("gate_event_id")?,
    };
    tx.commit().await?;
    Ok(result)
}

#[derive(Debug, Clone)]
pub struct RelationMutationRequest<'a> {
    pub operations: &'a serde_json::Value,
    pub source_event_namespace: &'a str,
    pub source_event_ref: &'a str,
    pub source_event_payload_hash: &'a str,
    pub request_metadata: &'a serde_json::Value,
    pub claimed_producer_class: Option<&'a str>,
}

pub async fn mutate_thought_relations_serialized(
    pool: &PgPool,
    request: RelationMutationRequest<'_>,
) -> Result<serde_json::Value, StorageError> {
    let row = sqlx::query_scalar::<_, serde_json::Value>(
        r#"
        SELECT public.mutate_thought_relations_serialized($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(request.operations)
    .bind(request.source_event_namespace)
    .bind(request.source_event_ref)
    .bind(request.source_event_payload_hash)
    .bind(request.request_metadata)
    .bind(request.claimed_producer_class)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn retract_thought_serialized(
    pool: &PgPool,
    thought_id: Uuid,
    reason: Option<&str>,
    claimed_producer_class: Option<&str>,
) -> Result<serde_json::Value, StorageError> {
    let result = sqlx::query_scalar::<_, serde_json::Value>(
        "SELECT public.retract_thought_serialized($1, $2, $3)",
    )
    .bind(thought_id)
    .bind(reason)
    .bind(claimed_producer_class)
    .fetch_one(pool)
    .await?;
    Ok(result)
}
