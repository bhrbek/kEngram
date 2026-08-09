# KENGRAM Delivery A — frozen six mutant receipts (smith terminal set)

- exact_head: `dc90f56cb97b5568b1c66c12c080928351461e72`
- tree: `ccb1e998ae2da225db4a056b90d5163a1c5fa70e`
- host: `Yeti-Mini.local`
- when: `2026-08-09T18:26:39Z`
- DATABASE_URL_set: `True`

Frozen six: query-embedding recorder deletion; FTS timeout→healthy empty;
rerank recorder deletion; `/health` mount removal; MCP degradations serialization
removal; neighboring counter-index shift.

Raw logs: `receipts/delivery-a-frozen-six/<id>-RED.log` and `-GREEN.log`.

## Mutant M1_query_embedding_recorder_deletion

- files: `crates/kengram-mcp/src/search.rs`
- package/filter: `kengram-mcp` / `search_thoughts_degrades_when_embedder_fails`
- command: `cargo test -p kengram-mcp --lib search_thoughts_degrades_when_embedder_fails -- --test-threads=1 --nocapture`
- baseline_sha256: `crates/kengram-mcp/src/search.rs`=`27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `crates/kengram-mcp/src/search.rs`=`bea15814457cc6a8a24d67112df36e1ca4f20666b41d3993cdce850b64b45dc8`
- RED_log: `receipts/delivery-a-frozen-six/M1_query_embedding_recorder_deletion-RED.log` exit=101
- GREEN_log: `receipts/delivery-a-frozen-six/M1_query_embedding_recorder_deletion-GREEN.log` exit=0
- restored_sha256: `crates/kengram-mcp/src/search.rs`=`27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- capability_RED: PASS_RED (failure_signal=yes)
- restore_MATCH: True
- GREEN: PASS_GREEN
- **RECEIPT_OK** `M1_query_embedding_recorder_deletion`

## Mutant M2_fts_timeout_healthy_empty

- files: `crates/kengram-mcp/src/search.rs`
- package/filter: `kengram-mcp` / `search_thoughts_soft_fails_timed_out_fts_leg`
- command: `cargo test -p kengram-mcp --lib search_thoughts_soft_fails_timed_out_fts_leg -- --test-threads=1 --nocapture`
- baseline_sha256: `crates/kengram-mcp/src/search.rs`=`27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `crates/kengram-mcp/src/search.rs`=`149c63cd8e593b7d52440718904b68867b48893cd4f9e838c2d5aabd27a81647`
- RED_log: `receipts/delivery-a-frozen-six/M2_fts_timeout_healthy_empty-RED.log` exit=101
- GREEN_log: `receipts/delivery-a-frozen-six/M2_fts_timeout_healthy_empty-GREEN.log` exit=0
- restored_sha256: `crates/kengram-mcp/src/search.rs`=`27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- capability_RED: PASS_RED (failure_signal=yes)
- restore_MATCH: True
- GREEN: PASS_GREEN
- **RECEIPT_OK** `M2_fts_timeout_healthy_empty`

## Mutant M3_rerank_recorder_deletion

- files: `crates/kengram-mcp/src/search.rs`
- package/filter: `kengram-mcp` / `rerank_timeout`
- command: `cargo test -p kengram-mcp --lib rerank_timeout -- --test-threads=1 --nocapture`
- baseline_sha256: `crates/kengram-mcp/src/search.rs`=`27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `crates/kengram-mcp/src/search.rs`=`712062609f2af035433dab35976c8a16e842c33e79150905006f4fc43f3206ae`
- RED_log: `receipts/delivery-a-frozen-six/M3_rerank_recorder_deletion-RED.log` exit=101
- GREEN_log: `receipts/delivery-a-frozen-six/M3_rerank_recorder_deletion-GREEN.log` exit=0
- restored_sha256: `crates/kengram-mcp/src/search.rs`=`27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- capability_RED: PASS_RED (failure_signal=yes)
- restore_MATCH: True
- GREEN: PASS_GREEN
- **RECEIPT_OK** `M3_rerank_recorder_deletion`

## Mutant M4_health_mount_removal

- files: `crates/kengram-cli/src/health.rs`
- package/filter: `kengram-cli` / `health_route_via_axum_router`
- command: `cargo test -p kengram-cli health_route_via_axum_router -- --test-threads=1 --nocapture`
- baseline_sha256: `crates/kengram-cli/src/health.rs`=`e7ce239d0a7af817695ab303299c2c9e6d17ad2c49d9e008fad3fcf872212b00`
- mutant_sha256: `crates/kengram-cli/src/health.rs`=`8bf7d8ce957208ff6284a0d75bf153c327f276239811ae9d281ef3819dbb71b8`
- RED_log: `receipts/delivery-a-frozen-six/M4_health_mount_removal-RED.log` exit=101
- GREEN_log: `receipts/delivery-a-frozen-six/M4_health_mount_removal-GREEN.log` exit=0
- restored_sha256: `crates/kengram-cli/src/health.rs`=`e7ce239d0a7af817695ab303299c2c9e6d17ad2c49d9e008fad3fcf872212b00`
- capability_RED: PASS_RED (failure_signal=yes)
- restore_MATCH: True
- GREEN: PASS_GREEN
- **RECEIPT_OK** `M4_health_mount_removal`

## Mutant M5_mcp_degradations_serialization

- files: `crates/kengram-mcp/src/server.rs`
- package/filter: `kengram-mcp` / `search_thoughts_tool_emits_degradations_json`
- command: `cargo test -p kengram-mcp --lib search_thoughts_tool_emits_degradations_json -- --test-threads=1 --nocapture`
- baseline_sha256: `crates/kengram-mcp/src/server.rs`=`995632cbf615e52b637d08c479392d2fe1b669dc9eb691dc5e7e8944111dca5d`
- mutant_sha256: `crates/kengram-mcp/src/server.rs`=`d386a9ed56fcb0963b17b2c176dec977e5b5fa8e2062b0f641c6a41d77d2fcd5`
- RED_log: `receipts/delivery-a-frozen-six/M5_mcp_degradations_serialization-RED.log` exit=101
- GREEN_log: `receipts/delivery-a-frozen-six/M5_mcp_degradations_serialization-GREEN.log` exit=0
- restored_sha256: `crates/kengram-mcp/src/server.rs`=`995632cbf615e52b637d08c479392d2fe1b669dc9eb691dc5e7e8944111dca5d`
- capability_RED: PASS_RED (failure_signal=yes)
- restore_MATCH: True
- GREEN: PASS_GREEN
- **RECEIPT_OK** `M5_mcp_degradations_serialization`

## Mutant M6_neighbor_counter_index_shift

- files: `crates/kengram-mcp/src/degradation.rs`
- package/filter: `kengram-mcp` / `record_increments_exact_cell`
- command: `cargo test -p kengram-mcp --lib record_increments_exact_cell -- --test-threads=1 --nocapture`
- baseline_sha256: `crates/kengram-mcp/src/degradation.rs`=`dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49`
- mutant_sha256: `crates/kengram-mcp/src/degradation.rs`=`622ce13a6054e95ff4531d20be8202ed9c72a04dc34c1d4561c9db9991a3fd0a`
- RED_log: `receipts/delivery-a-frozen-six/M6_neighbor_counter_index_shift-RED.log` exit=101
- GREEN_log: `receipts/delivery-a-frozen-six/M6_neighbor_counter_index_shift-GREEN.log` exit=0
- restored_sha256: `crates/kengram-mcp/src/degradation.rs`=`dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49`
- capability_RED: PASS_RED (failure_signal=yes)
- restore_MATCH: True
- GREEN: PASS_GREEN
- **RECEIPT_OK** `M6_neighbor_counter_index_shift`

## Summary

**ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK** exact_head=`dc90f56cb97b5568b1c66c12c080928351461e72`
