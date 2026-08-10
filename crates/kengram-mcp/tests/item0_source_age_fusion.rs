//! item0 watched obligations (e1), (e2), (e3) — spec seal a5f639da.
//!
//! e1: successful-rerank arm — a planted adjacent-rank near-tie pair (older
//!     source vs fresh, rerank scores 0.90001 vs 0.90000) flips by exactly the
//!     source-age component.
//! e2: reranker-off/fallback arm (repaired per neo review 29311d17 F1) — a
//!     controlled planted pair whose OBSERVED pre-item0 fallback order (the
//!     preserved `rrf_score` field is the recency-boosted RRF sort key the
//!     fallback path orders by) is old-before-fresh, and whose response order
//!     is fresh-before-old. The flip window is engineered: 40 fresh fillers
//!     push the pair to deep ranks where the adjacent relevance gap
//!     4/((60+r)(61+r)) is smaller than the age term; the old member's age
//!     (22d ≈ decay 0.601) sits inside the (0.598, 0.604) window where
//!     recency leaves it ABOVE fresh pre-fusion while the age gap still
//!     bridges the deep-rank gap. The zero-age-effect mutant
//!     (source_age_component = age_factor * 0.0) turns THIS selector RED —
//!     the watched negative control neo's F1 requires.
//! e3: pre-limit arm — the fresh adjacent candidate starts just OUTSIDE the
//!     requested `limit` and enters the result set only because the fusion
//!     runs before truncation.
//!
//! The near-tie pair is planted at ranks 2/3 under a rank-1 anchor: the frozen
//! constant 2.0/1891.0 equals the rank-1<->2 relevance gap exactly, so the top
//! pair ties-but-never-flips; rank 2/3's gap (4/(62*63)) is smaller and a
//! maximally fresh source bridges it.
//!
//! MUTANT control (executed at freeze, receipts in the freeze note): breaking
//! the exact constant to 2.0/189.1 turns e1 RED; production file sha256s are
//! frozen before the mutation and GREEN is credited only after byte-identical
//! restore.
//!
//! Run:
//!   export SQLX_OFFLINE=true
//!   export DATABASE_URL="postgres://kengram:kengram@localhost:5432/kengram"
//!   cargo test -p kengram-mcp --test item0_source_age_fusion -- --test-threads=1

use async_trait::async_trait;
use kengram_core::{Scope, Source};
use kengram_embed::{FakeEmbedder, RerankScore, Reranker, RerankerError};
use kengram_mcp::capture::{CaptureGateOptions, CaptureRequest, capture_with_gate_options};
use kengram_mcp::{SearchRequest, search_thoughts};
use sqlx::PgPool;

const EMBEDDER: &str = "bge-m3:1024";
const SCOPE: &str = "agents/item0-fusion";

/// Exact per-candidate scores by substring — full control of the post-rerank
/// rank order, which is the only rerank signal item0's fusion consumes.
struct ScriptedReranker {
    rules: Vec<(&'static str, f32)>,
}

#[async_trait]
impl Reranker for ScriptedReranker {
    fn model_id(&self) -> &str {
        "item0/scripted-reranker"
    }
    async fn rerank(
        &self,
        _query: &str,
        candidates: &[&str],
    ) -> Result<Vec<RerankScore>, RerankerError> {
        Ok(candidates
            .iter()
            .enumerate()
            .map(|(index, text)| {
                let score = self
                    .rules
                    .iter()
                    .find(|(needle, _)| text.contains(needle))
                    .map(|(_, score)| *score)
                    .unwrap_or(0.01);
                RerankScore { index, score }
            })
            .collect())
    }
}

async fn plant(pool: &PgPool, content: &str, age_days: i64) -> kengram_core::ThoughtId {
    let resp = capture_with_gate_options(
        pool,
        EMBEDDER,
        None,
        CaptureRequest {
            content: content.to_string(),
            source: Source::new("item0-fusion-test").unwrap(),
            scope: Some(Scope::new(SCOPE).unwrap()),
            metadata: None,
            argus_source_event: None,
        },
        CaptureGateOptions::default(),
    )
    .await
    .unwrap();
    // Plant the valid-time clock directly: fusion reads thoughts.created_at.
    sqlx::query("UPDATE thoughts SET created_at = now() - make_interval(days => $1) WHERE id = $2")
        .bind(age_days as i32)
        .bind(resp.thought_id.into_uuid())
        .execute(pool)
        .await
        .unwrap();
    resp.thought_id
}

fn request(limit: usize, rerank: bool) -> SearchRequest {
    SearchRequest {
        query: "fusionprobe".to_string(),
        scope: Some(Scope::new(SCOPE).unwrap()),
        scope_prefix: None,
        limit: Some(limit),
        recency_half_life_days: Some(30.0),
        rerank: Some(rerank),
        candidate_pool: None,
        chunk_serving_enabled: false,
        full_pipeline_enabled: false,
        tag_domain_routing_enabled: false,
        include_profile: false,
        tag_filter: None,
    }
}

// (e1) successful-rerank arm: adjacent near-tie pair flips for the fresh source.
#[sqlx::test(migrations = "../../migrations")]
async fn e1_rerank_arm_adjacent_near_tie_flips_for_fresh_source(pool: PgPool) {
    let _anchor = plant(&pool, "fusionprobe anchor stays on top", 3650).await;
    let old_pair = plant(&pool, "fusionprobe pair-old ten-year source", 3650).await;
    let fresh_pair = plant(&pool, "fusionprobe pair-fresh brand new source", 0).await;

    let reranker = ScriptedReranker {
        rules: vec![("anchor", 0.95), ("pair-old", 0.900_01), ("pair-fresh", 0.900_00)],
    };
    let embedder = FakeEmbedder::new();
    let resp = search_thoughts(&pool, &embedder, Some(&reranker), request(10, true))
        .await
        .unwrap();
    assert!(resp.rerank_used, "e1 requires the successful-rerank arm");
    let order: Vec<_> = resp.results.iter().map(|h| h.thought_id).collect();
    assert_eq!(order.len(), 3);
    assert!(resp.results[0].content.contains("anchor"), "rank-1 anchor holds (gap == max age term)");
    assert_eq!(
        order[1], fresh_pair,
        "fresh source must flip above the old near-tie at ranks 2/3"
    );
    assert_eq!(order[2], old_pair);
    let fresh_af = resp.results[1].age_factor.expect("age_factor must be set by fusion");
    let old_af = resp.results[2].age_factor.expect("age_factor must be set by fusion");
    assert!(fresh_af > 0.99, "fresh age_factor ~ 1, got {fresh_af}");
    assert!(old_af < 0.001, "ten-year age_factor ~ 0, got {old_af}");
}

/// Plant with sub-day precision (the e2 window is hours wide).
async fn plant_at_minutes(pool: &PgPool, content: &str, age_minutes: i64) -> kengram_core::ThoughtId {
    let resp = capture_with_gate_options(
        pool,
        EMBEDDER,
        None,
        CaptureRequest {
            content: content.to_string(),
            source: Source::new("item0-fusion-test").unwrap(),
            scope: Some(Scope::new(SCOPE).unwrap()),
            metadata: None,
            argus_source_event: None,
        },
        CaptureGateOptions::default(),
    )
    .await
    .unwrap();
    sqlx::query("UPDATE thoughts SET created_at = now() - make_interval(mins => $1) WHERE id = $2")
        .bind(age_minutes as i32)
        .bind(resp.thought_id.into_uuid())
        .execute(pool)
        .await
        .unwrap();
    resp.thought_id
}

// (e2) fallback arm: the same planted pair flips on the fallback path, with
// the pre-item0 order OBSERVED old-before-fresh (neo F1 repair condition).
#[sqlx::test(migrations = "../../migrations")]
async fn e2_fallback_arm_applies_same_term_to_fused_order(pool: PgPool) {
    // 40 fresh fillers (2x query term) occupy the top ranks; the pair sits
    // below them all: old (3x term -> strongest FTS, recency-decayed by
    // 0.601 to JUST above fresh) and fresh (1x term -> weakest FTS).
    for i in 0..40 {
        plant_at_minutes(
            &pool,
            &format!("fusionprobe filler fusionprobe number {i}"),
            0,
        )
        .await;
    }
    // 22 days 59 minutes: decay 2^(-22.04/30) ~ 0.6014, inside the
    // (0.5980, 0.6040) window where m/61 lands between 1/102 and 1/101.
    let old = plant_at_minutes(
        &pool,
        "fusionprobe old fusionprobe pair fusionprobe source",
        22 * 24 * 60 + 59,
    )
    .await;
    let fresh = plant_at_minutes(&pool, "fusionprobe fresh pair source", 0).await;

    let embedder = FakeEmbedder::new();
    let resp = search_thoughts(&pool, &embedder, None, request(50, false))
        .await
        .unwrap();
    assert!(!resp.rerank_used, "e2 requires the reranker-off/fallback arm");
    assert_eq!(resp.results.len(), 42, "all planted rows must return");

    // OBSERVED pre-item0 fallback order: rrf_score is the recency-boosted RRF
    // key the fallback path sorted by before the fusion ran. The pair must be
    // the bottom two of that order with OLD ABOVE FRESH.
    let mut pre_order: Vec<_> = resp
        .results
        .iter()
        .map(|h| (h.thought_id, h.rrf_score.expect("fused fallback hits carry rrf_score")))
        .collect();
    pre_order.sort_by(|a, b| b.1.total_cmp(&a.1));
    let pre_ids: Vec<_> = pre_order.iter().map(|(id, _)| *id).collect();
    assert_eq!(
        &pre_ids[40..],
        &[old, fresh],
        "pre-item0 fallback order must be ... old, fresh (old ABOVE fresh, both below fillers)"
    );

    // POST-item0 response order: the fusion flips the pair — fresh overtakes
    // old across the deep-rank gap on the age term alone.
    let post_ids: Vec<_> = resp.results.iter().map(|h| h.thought_id).collect();
    let old_pos = post_ids.iter().position(|id| *id == old).unwrap();
    let fresh_pos = post_ids.iter().position(|id| *id == fresh).unwrap();
    assert!(
        fresh_pos < old_pos,
        "post-item0 the fresh pair member must rank above old (fresh_pos={fresh_pos}, old_pos={old_pos}) — \
         this is the assertion the zero-age-effect mutant must turn RED"
    );
    // Fresh must NOT have leapfrogged the filler block — the flip is the
    // adjacent-pair effect, not a wholesale reorder.
    assert_eq!(fresh_pos, 40, "fresh enters exactly one rank above old");
    assert_eq!(old_pos, 41);

    // The term ran with the planted decay values.
    let by_id: std::collections::HashMap<_, _> = resp
        .results
        .iter()
        .map(|h| (h.thought_id, h.age_factor))
        .collect();
    let old_af = by_id[&old].expect("age_factor populated on the fallback path");
    let fresh_af = by_id[&fresh].expect("age_factor populated on the fallback path");
    assert!((0.55..0.65).contains(&old_af), "old decay ~0.60, got {old_af}");
    assert!(fresh_af > 0.99, "fresh decay ~1, got {fresh_af}");
}

// (e3) pre-limit arm: fresh candidate outside the requested limit enters ONLY
// because fusion runs before truncation.
#[sqlx::test(migrations = "../../migrations")]
async fn e3_fresh_candidate_enters_from_outside_the_limit(pool: PgPool) {
    let anchor = plant(&pool, "fusionprobe anchor stays on top", 3650).await;
    let old_pair = plant(&pool, "fusionprobe pair-old ten-year source", 3650).await;
    let fresh_pair = plant(&pool, "fusionprobe pair-fresh brand new source", 0).await;

    let reranker = ScriptedReranker {
        rules: vec![("anchor", 0.95), ("pair-old", 0.900_01), ("pair-fresh", 0.900_00)],
    };
    let embedder = FakeEmbedder::new();
    // limit=2: pre-fusion the fresh candidate sits at rank 3, OUTSIDE the
    // limit. It must appear in the 2-row response purely on the age term.
    let resp = search_thoughts(&pool, &embedder, Some(&reranker), request(2, true))
        .await
        .unwrap();
    assert!(resp.rerank_used);
    let order: Vec<_> = resp.results.iter().map(|h| h.thought_id).collect();
    assert_eq!(order.len(), 2);
    assert_eq!(order[0], anchor);
    assert_eq!(
        order[1], fresh_pair,
        "the fresh rank-3 candidate must enter the top-2 because fusion precedes take(limit)"
    );
    assert!(!order.contains(&old_pair));
}
