//! Search-leg degradation receipts, process-local counters, and health snapshot.
//! Delivery A: observe fail-open without changing fallback policy or timeout values.

use serde::Serialize;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};
use time::OffsetDateTime;

/// Closed leg inventory (spec §3). Order is the health matrix order (Knox pin:
/// full fixed matrix in enum order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum SearchLeg {
    QueryEmbedding = 0,
    ThoughtVector,
    ChunkVector,
    ContextualChunkVector,
    ThoughtFts,
    ChunkFts,
    ContextualChunkFts,
    PairwiseChunkFts,
    DomainScope,
    TagFacet,
    QueryExpansion,
    ExpansionEmbedding,
    ExpansionThoughtVector,
    ExpansionChunkVector,
    ExpansionThoughtFts,
    ExpansionChunkFts,
    Rerank,
}

impl SearchLeg {
    pub const ALL: [SearchLeg; 17] = [
        SearchLeg::QueryEmbedding,
        SearchLeg::ThoughtVector,
        SearchLeg::ChunkVector,
        SearchLeg::ContextualChunkVector,
        SearchLeg::ThoughtFts,
        SearchLeg::ChunkFts,
        SearchLeg::ContextualChunkFts,
        SearchLeg::PairwiseChunkFts,
        SearchLeg::DomainScope,
        SearchLeg::TagFacet,
        SearchLeg::QueryExpansion,
        SearchLeg::ExpansionEmbedding,
        SearchLeg::ExpansionThoughtVector,
        SearchLeg::ExpansionChunkVector,
        SearchLeg::ExpansionThoughtFts,
        SearchLeg::ExpansionChunkFts,
        SearchLeg::Rerank,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::QueryEmbedding => "query_embedding",
            Self::ThoughtVector => "thought_vector",
            Self::ChunkVector => "chunk_vector",
            Self::ContextualChunkVector => "contextual_chunk_vector",
            Self::ThoughtFts => "thought_fts",
            Self::ChunkFts => "chunk_fts",
            Self::ContextualChunkFts => "contextual_chunk_fts",
            Self::PairwiseChunkFts => "pairwise_chunk_fts",
            Self::DomainScope => "domain_scope",
            Self::TagFacet => "tag_facet",
            Self::QueryExpansion => "query_expansion",
            Self::ExpansionEmbedding => "expansion_embedding",
            Self::ExpansionThoughtVector => "expansion_thought_vector",
            Self::ExpansionChunkVector => "expansion_chunk_vector",
            Self::ExpansionThoughtFts => "expansion_thought_fts",
            Self::ExpansionChunkFts => "expansion_chunk_fts",
            Self::Rerank => "rerank",
        }
    }

    pub const fn index(self) -> usize {
        self as u8 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
#[repr(u8)]
pub enum DegradationReason {
    Timeout = 0,
    Unreachable,
    Backend,
    Malformed,
    Misconfigured,
    Storage,
    Unknown,
}

impl DegradationReason {
    pub const ALL: [DegradationReason; 7] = [
        DegradationReason::Timeout,
        DegradationReason::Unreachable,
        DegradationReason::Backend,
        DegradationReason::Malformed,
        DegradationReason::Misconfigured,
        DegradationReason::Storage,
        DegradationReason::Unknown,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Unreachable => "unreachable",
            Self::Backend => "backend",
            Self::Malformed => "malformed",
            Self::Misconfigured => "misconfigured",
            Self::Storage => "storage",
            Self::Unknown => "unknown",
        }
    }

    pub const fn index(self) -> usize {
        self as u8 as usize
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DegradationFallback {
    LexicalOnly,
    AvailableSearchLegs,
    OriginalQuery,
    RrfRecency,
}

impl DegradationFallback {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::LexicalOnly => "lexical_only",
            Self::AvailableSearchLegs => "available_search_legs",
            Self::OriginalQuery => "original_query",
            Self::RrfRecency => "rrf_recency",
        }
    }
}

/// Bounded, serializable degradation (no query/secrets/raw errors).
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SearchDegradation {
    pub leg: SearchLeg,
    pub reason: DegradationReason,
    pub fallback: DegradationFallback,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    pub failed_attempts: u32,
}

impl SearchDegradation {
    pub fn new(
        leg: SearchLeg,
        reason: DegradationReason,
        fallback: DegradationFallback,
        timeout_ms: Option<u64>,
        failed_attempts: u32,
    ) -> Self {
        Self {
            leg,
            reason,
            fallback,
            timeout_ms,
            failed_attempts: failed_attempts.max(1),
        }
    }
}

pub const REASON_COUNT: usize = 7;
pub const LEG_COUNT: usize = 17;

/// Process-local counters. Shared via Arc. Reset on restart (new epoch).
#[derive(Debug)]
pub struct SearchCounters {
    pub epoch_started_at: OffsetDateTime,
    requests_total: AtomicU64,
    degraded_requests_total: AtomicU64,
    /// Full matrix [leg][reason] — Knox pin: always exposed in full enum order.
    leg_reason: [[AtomicU64; REASON_COUNT]; LEG_COUNT],
    last_degradation_unix_ms: AtomicU64,
    search_seq: AtomicU64,
}

impl Default for SearchCounters {
    fn default() -> Self {
        Self::new()
    }
}

impl SearchCounters {
    pub fn new() -> Self {
        Self {
            epoch_started_at: OffsetDateTime::now_utc(),
            requests_total: AtomicU64::new(0),
            degraded_requests_total: AtomicU64::new(0),
            leg_reason: std::array::from_fn(|_| std::array::from_fn(|_| AtomicU64::new(0))),
            last_degradation_unix_ms: AtomicU64::new(0),
            search_seq: AtomicU64::new(0),
        }
    }

    pub fn next_search_seq(&self) -> u64 {
        self.search_seq.fetch_add(1, Ordering::Relaxed) + 1
    }

    pub fn record_request(&self) {
        self.requests_total.fetch_add(1, Ordering::Relaxed);
    }

    /// Record one logical-leg degradation (at most once per leg per request
    /// at the call site). Increments the matrix cell only.
    pub fn record_leg_degradation(&self, d: &SearchDegradation) {
        let li = d.leg.index();
        let ri = d.reason.index();
        self.leg_reason[li][ri].fetch_add(1, Ordering::Relaxed);
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.last_degradation_unix_ms
            .store(now_ms, Ordering::Relaxed);
    }

    /// Call once when response receipt is non-empty.
    pub fn record_degraded_request(&self) {
        self.degraded_requests_total.fetch_add(1, Ordering::Relaxed);
    }

    pub fn snapshot(&self, effective_timeouts: serde_json::Value) -> HealthSearchSnapshot {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last = self.last_degradation_unix_ms.load(Ordering::Relaxed);
        let degraded_recent = last > 0 && now_ms.saturating_sub(last) <= 300_000;
        let mut matrix = Vec::with_capacity(LEG_COUNT * REASON_COUNT);
        for leg in SearchLeg::ALL {
            for reason in DegradationReason::ALL {
                let count = self.leg_reason[leg.index()][reason.index()].load(Ordering::Relaxed);
                matrix.push(LegReasonCount {
                    leg: leg.as_str(),
                    reason: reason.as_str(),
                    count,
                });
            }
        }
        let epoch = self.epoch_started_at;
        let uptime = (OffsetDateTime::now_utc() - epoch).whole_seconds().max(0) as u64;
        HealthSearchSnapshot {
            status: if degraded_recent { "degraded" } else { "ok" },
            counter_epoch_started_at: epoch
                .format(&time::format_description::well_known::Rfc3339)
                .unwrap_or_else(|_| epoch.to_string()),
            uptime_seconds: uptime,
            requests_total: self.requests_total.load(Ordering::Relaxed),
            degraded_requests_total: self.degraded_requests_total.load(Ordering::Relaxed),
            last_degradation_at: if last == 0 {
                None
            } else {
                let secs = (last / 1000) as i64;
                let nsec = ((last % 1000) * 1_000_000) as i64;
                OffsetDateTime::from_unix_timestamp(secs)
                    .ok()
                    .and_then(|t| t.replace_nanosecond((nsec as u32).min(999_999_999)).ok())
                    .and_then(|t| {
                        t.format(&time::format_description::well_known::Rfc3339)
                            .ok()
                    })
            },
            degraded_in_last_300_seconds: degraded_recent,
            leg_degradations_total: matrix,
            effective_timeouts,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct LegReasonCount {
    pub leg: &'static str,
    pub reason: &'static str,
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HealthSearchSnapshot {
    pub status: &'static str,
    pub counter_epoch_started_at: String,
    pub uptime_seconds: u64,
    pub requests_total: u64,
    pub degraded_requests_total: u64,
    pub last_degradation_at: Option<String>,
    pub degraded_in_last_300_seconds: bool,
    /// Full fixed matrix in enum order (Knox pin).
    pub leg_degradations_total: Vec<LegReasonCount>,
    pub effective_timeouts: serde_json::Value,
}

/// Map embedder errors to degradation reason (no raw text).
pub fn reason_from_embedder(err: &kengram_core::EmbedderError) -> DegradationReason {
    use kengram_core::EmbedderError;
    match err {
        EmbedderError::Timeout { .. } => DegradationReason::Timeout,
        EmbedderError::Unreachable(_) => DegradationReason::Unreachable,
        EmbedderError::Backend { .. } => DegradationReason::Backend,
        EmbedderError::MalformedResponse(_) | EmbedderError::DimensionMismatch { .. } => {
            DegradationReason::Malformed
        }
        EmbedderError::EmptyBatch => DegradationReason::Misconfigured,
    }
}

pub fn reason_from_reranker(err: &kengram_embed::RerankerError) -> DegradationReason {
    use kengram_embed::RerankerError;
    match err {
        RerankerError::Timeout { .. } => DegradationReason::Timeout,
        RerankerError::Unreachable(_) => DegradationReason::Unreachable,
        RerankerError::Backend { .. } => DegradationReason::Backend,
        RerankerError::MalformedResponse(_) => DegradationReason::Malformed,
        RerankerError::Misconfigured(_) => DegradationReason::Misconfigured,
    }
}

/// Record degradation at the choke point: receipt + counter + structured WARN.
pub fn record_degradation(
    counters: Option<&Arc<SearchCounters>>,
    receipt: &mut Vec<SearchDegradation>,
    d: SearchDegradation,
    search_seq: u64,
) {
    // At most one receipt entry per leg per request.
    if receipt.iter().any(|e| e.leg == d.leg) {
        return;
    }
    if let Some(c) = counters {
        c.record_leg_degradation(&d);
    }
    tracing::warn!(
        event = "search_leg_degraded",
        leg = d.leg.as_str(),
        reason = d.reason.as_str(),
        fallback = d.fallback.as_str(),
        timeout_ms = d.timeout_ms,
        failed_attempts = d.failed_attempts,
        search_seq,
        "search leg degraded"
    );
    receipt.push(d);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_matrix_has_17_times_7_cells() {
        let c = SearchCounters::new();
        let snap = c.snapshot(serde_json::json!({}));
        assert_eq!(snap.leg_degradations_total.len(), 17 * 7);
        assert_eq!(snap.leg_degradations_total[0].leg, "query_embedding");
        assert_eq!(snap.leg_degradations_total[0].reason, "timeout");
        // last cell
        let last = snap.leg_degradations_total.last().unwrap();
        assert_eq!(last.leg, "rerank");
        assert_eq!(last.reason, "unknown");
    }

    #[test]
    fn record_increments_exact_cell() {
        let c = Arc::new(SearchCounters::new());
        let mut receipt = vec![];
        let seq = c.next_search_seq();
        c.record_request();
        record_degradation(
            Some(&c),
            &mut receipt,
            SearchDegradation::new(
                SearchLeg::TagFacet,
                DegradationReason::Timeout,
                DegradationFallback::AvailableSearchLegs,
                Some(300),
                1,
            ),
            seq,
        );
        c.record_degraded_request();
        assert_eq!(receipt.len(), 1);
        let snap = c.snapshot(serde_json::json!({}));
        assert_eq!(snap.requests_total, 1);
        assert_eq!(snap.degraded_requests_total, 1);
        let cell = snap
            .leg_degradations_total
            .iter()
            .find(|c| c.leg == "tag_facet" && c.reason == "timeout")
            .unwrap();
        assert_eq!(cell.count, 1);
        // neighboring cell zero
        let neighbor = snap
            .leg_degradations_total
            .iter()
            .find(|c| c.leg == "domain_scope" && c.reason == "timeout")
            .unwrap();
        assert_eq!(neighbor.count, 0);
    }

    #[test]
    fn duplicate_leg_receipt_is_idempotent() {
        let c = Arc::new(SearchCounters::new());
        let mut receipt = vec![];
        let seq = c.next_search_seq();
        let d = SearchDegradation::new(
            SearchLeg::ThoughtFts,
            DegradationReason::Timeout,
            DegradationFallback::AvailableSearchLegs,
            Some(300),
            2,
        );
        record_degradation(Some(&c), &mut receipt, d.clone(), seq);
        record_degradation(Some(&c), &mut receipt, d, seq);
        assert_eq!(receipt.len(), 1);
        let snap = c.snapshot(serde_json::json!({}));
        let cell = snap
            .leg_degradations_total
            .iter()
            .find(|c| c.leg == "thought_fts" && c.reason == "timeout")
            .unwrap();
        assert_eq!(cell.count, 1);
    }
}
