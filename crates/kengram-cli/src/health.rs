//! GET /health — Delivery A process-local search degradation snapshot.

use axum::Json;
use axum::extract::State;
use kengram_mcp::degradation::SearchCounters;
use serde::Serialize;
use std::sync::Arc;

#[derive(Clone)]
pub struct HealthState {
    pub counters: Arc<SearchCounters>,
    pub effective_timeouts: serde_json::Value,
}

#[derive(Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub counter_epoch_started_at: String,
    pub uptime_seconds: u64,
    pub search: HealthSearchBody,
}

#[derive(Serialize)]
pub struct HealthSearchBody {
    pub requests_total: u64,
    pub degraded_requests_total: u64,
    pub last_degradation_at: Option<String>,
    pub degraded_in_last_300_seconds: bool,
    pub leg_degradations_total: Vec<kengram_mcp::degradation::LegReasonCount>,
    pub effective_timeouts: serde_json::Value,
}

pub async fn health_handler(State(state): State<HealthState>) -> Json<HealthResponse> {
    let snap = state.counters.snapshot(state.effective_timeouts.clone());
    Json(HealthResponse {
        status: snap.status,
        counter_epoch_started_at: snap.counter_epoch_started_at,
        uptime_seconds: snap.uptime_seconds,
        search: HealthSearchBody {
            requests_total: snap.requests_total,
            degraded_requests_total: snap.degraded_requests_total,
            last_degradation_at: snap.last_degradation_at,
            degraded_in_last_300_seconds: snap.degraded_in_last_300_seconds,
            leg_degradations_total: snap.leg_degradations_total,
            effective_timeouts: snap.effective_timeouts,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use kengram_mcp::degradation::{
        DegradationFallback, DegradationReason, SearchDegradation, SearchLeg, record_degradation,
    };

    #[test]
    fn health_snapshot_full_matrix_and_degraded_flag() {
        let counters = Arc::new(SearchCounters::new());
        let mut receipt = vec![];
        let seq = counters.next_search_seq();
        counters.record_request();
        record_degradation(
            Some(&counters),
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
        counters.record_degraded_request();
        let snap = counters.snapshot(serde_json::json!({"tag_facet_timeout_ms": 300}));
        assert_eq!(snap.leg_degradations_total.len(), 17 * 7);
        assert_eq!(snap.requests_total, 1);
        assert_eq!(snap.degraded_requests_total, 1);
        assert!(snap.degraded_in_last_300_seconds);
        assert_eq!(snap.status, "degraded");
        let cell = snap
            .leg_degradations_total
            .iter()
            .find(|c| c.leg == "tag_facet" && c.reason == "timeout")
            .unwrap();
        assert_eq!(cell.count, 1);
        let s = serde_json::to_string(&snap).unwrap();
        assert!(!s.contains("postgres://"));
        assert!(!s.contains("password"));
    }
}
