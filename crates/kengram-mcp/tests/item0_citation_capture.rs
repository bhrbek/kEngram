//! item0 watched obligations (a2), (b), (b2), (c), (d) — spec seal a5f639da.
//!
//! Response-only provenance: nothing here asserts a persisted origin column
//! (the live schema has none and item0 adds no migration). Zero-write claims
//! are asserted by before/after DB reads, never by timestamps alone.
//!
//! Run (per main's AGENTS contract, non-production DB):
//!   export SQLX_OFFLINE=true
//!   export DATABASE_URL="postgres://kengram:kengram@localhost:5432/kengram"
//!   cargo test -p kengram-mcp --test item0_citation_capture -- --test-threads=1

use kengram_core::{Scope, Source, ThoughtId};
use kengram_mcp::capture::{
    ArgusSourceEventRequest, CaptureError, CaptureGateOptions, CaptureRequest, CaptureResponse,
    capture_with_gate_options,
};
use kengram_mcp::citation::format_citation;
use kengram_mcp::{RetractThoughtRequest, retract_thought};
use sqlx::PgPool;
use time::OffsetDateTime;

const EMBEDDER: &str = "bge-m3:1024";

async fn cap(
    pool: &PgPool,
    scope: &str,
    content: String,
    source_created_at: Option<OffsetDateTime>,
    event: Option<ArgusSourceEventRequest>,
) -> Result<CaptureResponse, CaptureError> {
    capture_with_gate_options(
        pool,
        EMBEDDER,
        None,
        CaptureRequest {
            content,
            source: Source::new("item0-test").unwrap(),
            scope: Some(Scope::new(scope).unwrap()),
            metadata: None,
            argus_source_event: event,
        },
        CaptureGateOptions {
            source_created_at,
            candidate_embedding: None,
            bypass_reason: None,
            relation_intents: vec![],
            claimed_producer_class: None,
            correlation_id: None,
        },
    )
    .await
}

/// One row of write-surface counts. Equality before/after a replay or a
/// rejected capture IS the zero-write assertion (b2).
#[derive(Debug, PartialEq, Clone)]
struct WriteSurface {
    thoughts: i64,
    source_events: i64,
    gate_events: i64,
    links: i64,
    pending_embeddings: i64,
    last_seen_at: Option<OffsetDateTime>,
}

async fn write_surface(pool: &PgPool, namespace: &str, source_ref: &str) -> WriteSurface {
    let (thoughts, source_events, gate_events, links, pending_embeddings): (i64, i64, i64, i64, i64) =
        sqlx::query_as(
            "SELECT (SELECT COUNT(*) FROM thoughts),
                    (SELECT COUNT(*) FROM argus_source_events),
                    (SELECT COUNT(*) FROM thought_ingest_gate_events),
                    (SELECT COUNT(*) FROM thought_links),
                    (SELECT COUNT(*) FROM pending_embeddings)",
        )
        .fetch_one(pool)
        .await
        .unwrap();
    let last_seen_at: Option<OffsetDateTime> = sqlx::query_scalar(
        "SELECT last_seen_at FROM argus_source_events WHERE namespace = $1 AND source_ref = $2",
    )
    .bind(namespace)
    .bind(source_ref)
    .fetch_optional(pool)
    .await
    .unwrap();
    WriteSurface {
        thoughts,
        source_events,
        gate_events,
        links,
        pending_embeddings,
        last_seen_at,
    }
}

// (a2) ORDER watch: first-occurrence distinct order, read from the RESPONSE.
#[sqlx::test(migrations = "../../migrations")]
async fn a2_resolved_origin_ids_first_occurrence_distinct_order(pool: PgPool) {
    let scope = "agents/item0-a2";
    let o1 = cap(&pool, scope, "origin one".into(), None, None).await.unwrap();
    let o2 = cap(&pool, scope, "origin two".into(), None, None).await.unwrap();
    // Name them so uuid(a) < uuid(b) regardless of which capture got which id.
    let (a, b) = if o1.thought_id.into_uuid() < o2.thought_id.into_uuid() {
        (o1.thought_id, o2.thought_id)
    } else {
        (o2.thought_id, o1.thought_id)
    };
    // B first, interleaved repeats, out of UUID sort order: B A B A.
    let raw = format!(
        "cites {} then {} then {} then {} end",
        format_citation(b),
        format_citation(a),
        format_citation(b),
        format_citation(a),
    );
    let resp = cap(&pool, scope, raw, None, None).await.unwrap();
    assert_eq!(
        resp.resolved_origin_ids,
        vec![b, a],
        "response must carry first-occurrence distinct order, not UUID order"
    );
}

// (b) origin validation fail-closed: unknown / cross-scope / retracted, each
// with its named error and no thought written.
#[sqlx::test(migrations = "../../migrations")]
async fn b_origin_validation_fails_closed_with_named_errors(pool: PgPool) {
    let scope = "agents/item0-b";
    let origin = cap(&pool, scope, "live origin".into(), None, None).await.unwrap();
    let other_scope_origin = cap(&pool, "agents/item0-b-other", "other-scope origin".into(), None, None)
        .await
        .unwrap();
    let retracted_origin = cap(&pool, scope, "doomed origin".into(), None, None).await.unwrap();
    retract_thought(
        &pool,
        RetractThoughtRequest {
            thought_id: retracted_origin.thought_id,
            reason: Some("item0 (b) retracted-origin arm".into()),
        },
    )
    .await
    .unwrap();
    let thoughts_before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts")
        .fetch_one(&pool)
        .await
        .unwrap();

    let unknown = ThoughtId::from(uuid::Uuid::from_u128(0xdead_beef_dead_beef_dead_beef_u128));
    let err = cap(&pool, scope, format!("x {}", format_citation(unknown)), None, None)
        .await
        .unwrap_err();
    assert!(
        matches!(err, CaptureError::CitationOriginNotFound(id) if id == unknown),
        "unknown origin must reject with CitationOriginNotFound, got: {err}"
    );

    let err = cap(
        &pool,
        scope,
        format!("x {}", format_citation(other_scope_origin.thought_id)),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(&err, CaptureError::CitationOriginScopeMismatch { thought_id, .. }
            if *thought_id == other_scope_origin.thought_id),
        "cross-scope origin must reject with CitationOriginScopeMismatch, got: {err}"
    );

    let err = cap(
        &pool,
        scope,
        format!("x {}", format_citation(retracted_origin.thought_id)),
        None,
        None,
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, CaptureError::CitationOriginRetracted(id) if id == retracted_origin.thought_id),
        "retracted origin must reject with CitationOriginRetracted, got: {err}"
    );

    let thoughts_after: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM thoughts")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(thoughts_before, thoughts_after, "rejected captures must write no thought");
    let _ = origin;
}

// (b2) POSITIVE: an exact completed source-event identity replayed AFTER its
// cited origin is retracted returns the original result read-only with zero
// writes. NEGATIVE: a NEW identity citing the same retracted origin rejects
// with zero writes.
#[sqlx::test(migrations = "../../migrations")]
async fn b2_completed_replay_precedes_origin_liveness_with_zero_writes(pool: PgPool) {
    let scope = "agents/item0-b2";
    let ns = "tests/item0-b2";
    let sref = "replay-after-retraction";
    let phash = "a".repeat(64);

    let origin = cap(&pool, scope, "cited origin for replay".into(), None, None)
        .await
        .unwrap();
    let raw = format!("derived note {}", format_citation(origin.thought_id));
    let first = cap(
        &pool,
        scope,
        raw.clone(),
        None,
        Some(ArgusSourceEventRequest {
            namespace: ns.into(),
            source_ref: sref.into(),
            payload_hash: phash.clone(),
            metadata: None,
        }),
    )
    .await
    .unwrap();

    retract_thought(
        &pool,
        RetractThoughtRequest {
            thought_id: origin.thought_id,
            reason: Some("item0 (b2) retraction between completion and replay".into()),
        },
    )
    .await
    .unwrap();

    let before = write_surface(&pool, ns, sref).await;
    let replay = cap(
        &pool,
        scope,
        raw.clone(),
        None,
        Some(ArgusSourceEventRequest {
            namespace: ns.into(),
            source_ref: sref.into(),
            payload_hash: phash.clone(),
            metadata: None,
        }),
    )
    .await
    .unwrap();
    let after = write_surface(&pool, ns, sref).await;

    assert_eq!(replay.thought_id, first.thought_id, "replay must return the original thought");
    assert_eq!(replay.born_on, first.born_on, "replay must return the original born_on");
    assert!(replay.is_duplicate, "replay is a duplicate disposition, not a new row");
    assert_eq!(
        before, after,
        "completed replay must be read-only: no count and no last_seen_at change"
    );

    // NEGATIVE arm: a NEW source identity citing the retracted origin gets no
    // replay exemption — fail closed, zero writes.
    let before_neg = write_surface(&pool, ns, sref).await;
    let err = cap(
        &pool,
        scope,
        format!("fresh derivation {}", format_citation(origin.thought_id)),
        None,
        Some(ArgusSourceEventRequest {
            namespace: ns.into(),
            source_ref: "new-identity-same-retracted-origin".into(),
            payload_hash: "b".repeat(64),
            metadata: None,
        }),
    )
    .await
    .unwrap_err();
    assert!(
        matches!(err, CaptureError::CitationOriginRetracted(id) if id == origin.thought_id),
        "new identity citing retracted origin must reject, got: {err}"
    );
    let after_neg = write_surface(&pool, ns, sref).await;
    // The rejected identity must also leave no source-event row of its own.
    let new_identity_rows: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM argus_source_events WHERE source_ref = 'new-identity-same-retracted-origin'",
    )
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(new_identity_rows, 0);
    assert_eq!(before_neg, after_neg, "rejected new identity must write nothing");
}

// (c) inheritance: gate-row effective_created_at equals the fold minimum —
// asserted from the gate event row the driver wrote (driver-ran evidence).
#[sqlx::test(migrations = "../../migrations")]
async fn c_gate_row_effective_created_at_is_fold_minimum(pool: PgPool) {
    let scope = "agents/item0-c";
    let old = OffsetDateTime::from_unix_timestamp(1_650_000_000).unwrap();
    let newer = old + time::Duration::days(30);
    let origin = cap(&pool, scope, "old origin".into(), Some(old), None).await.unwrap();
    let resp = cap(
        &pool,
        scope,
        format!("derived {}", format_citation(origin.thought_id)),
        Some(newer),
        None,
    )
    .await
    .unwrap();

    let gate_event_id = resp.gate_event_id.expect("gated capture must return its gate event id");
    let effective: OffsetDateTime = sqlx::query_scalar(
        "SELECT effective_created_at FROM thought_ingest_gate_events WHERE id = $1",
    )
    .bind(gate_event_id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(
        effective, old,
        "gate row effective_created_at must equal the fold minimum (cited origin age)"
    );
    assert_eq!(resp.born_on, old);
}

// (d) wire names: born_on == persisted_source_age == persisted created_at.
#[sqlx::test(migrations = "../../migrations")]
async fn d_wire_names_round_trip_persisted_created_at(pool: PgPool) {
    let scope = "agents/item0-d";
    let resp = cap(&pool, scope, "wire-name round trip".into(), None, None)
        .await
        .unwrap();
    let persisted: OffsetDateTime =
        sqlx::query_scalar("SELECT created_at FROM thoughts WHERE id = $1")
            .bind(resp.thought_id.into_uuid())
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(resp.born_on, persisted);
    assert_eq!(resp.persisted_source_age, persisted);
    assert_eq!(resp.born_on, resp.persisted_source_age);
}
