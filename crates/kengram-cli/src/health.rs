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

pub fn mount_health(router: axum::Router<HealthState>) -> axum::Router<HealthState> {
    router.route("/health", axum::routing::get(health_handler))
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

    /// V4 — real Axum GET /health through production router (not direct fn call alone).
    #[tokio::test]
    async fn health_route_via_axum_router() {
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let counters = Arc::new(SearchCounters::new());
        let mut receipt = vec![];
        let seq = counters.next_search_seq();
        counters.record_request();
        record_degradation(
            Some(&counters),
            &mut receipt,
            SearchDegradation::new(
                SearchLeg::Rerank,
                DegradationReason::Timeout,
                DegradationFallback::RrfRecency,
                Some(1000),
                1,
            ),
            seq,
        );
        counters.record_degraded_request();

        let state = HealthState {
            counters: counters.clone(),
            effective_timeouts: serde_json::json!({"thought_fts_timeout_ms": 300}),
        };
        let app = mount_health(axum::Router::new()).with_state(state);

        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let v: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(v["status"], "degraded");
        assert_eq!(v["search"]["requests_total"], 1);
        assert_eq!(v["search"]["degraded_requests_total"], 1);
        assert!(
            v["search"]["leg_degradations_total"]
                .as_array()
                .unwrap()
                .len()
                == 17 * 7
        );
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(!body.contains("postgres://"));
        assert!(!body.contains("password"));

        // Restart/new counter-object control: epoch resets.
        let fresh = Arc::new(SearchCounters::new());
        let snap = fresh.snapshot(serde_json::json!({}));
        assert_eq!(snap.requests_total, 0);
        assert_eq!(snap.degraded_requests_total, 0);
        // Fresh object is a new epoch; counts reset even if RFC3339 collides in same second.
        assert_eq!(snap.status, "ok");
    }

    #[tokio::test]
    async fn health_route_missing_mount_is_detectable() {
        // Companion control for mutant 4: router without /health returns 404.
        use axum::body::Body;
        use axum::http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = axum::Router::new().route("/mcp", axum::routing::get(|| async { "ok" }));
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
