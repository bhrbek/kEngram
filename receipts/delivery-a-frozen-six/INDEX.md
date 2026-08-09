# KENGRAM Delivery A — frozen six mutant receipts (SPIKE-CERTIFIED)

- executed_code_head: `dc90f56cb97b5568b1c66c12c080928351461e72`
- certifier: `spike`
- when: `2026-08-09T18:33:38.569162+00:00`
- smith_shape: `324df2a9` exact frozen six
- heavy_input: evaluated 74d0ba7; F1 code kept; F2 logs re-run under spike (not trusted as baseline)

Ceremony: baseline hash → exact one-preimage transform → cargo RED (raw log) →
byte restore + rehash MATCH → cargo GREEN (raw log).

## Mutant M1_query_embedding_recorder_deletion

- file: `crates/kengram-mcp/src/search.rs`
- transform: QueryEmbedding→ThoughtVector in embedder-fail degradation (recorder deletion)
- command: `cargo test -p kengram-mcp --lib search::tests::search_thoughts_degrades_when_embedder_fails -- --exact --test-threads=1 --nocapture`
- capability_marker: `QueryEmbedding`
- baseline_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `9b667ab492426d78159bd6ab5ad10f755f7e78ca68a22007d2d73ceaa5961460`
- RED_log: `receipts/delivery-a-frozen-six/M1_query_embedding_recorder_deletion-RED.log` exit=101
- restore_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M1_query_embedding_recorder_deletion-GREEN.log` exit=0
- **RECEIPT_OK** `M1_query_embedding_recorder_deletion`

## Mutant M2_fts_timeout_healthy_empty

- file: `crates/kengram-mcp/src/search.rs`
- transform: storage_leg_fail_open returns early for ThoughtFts (timeout→silent healthy empty)
- command: `cargo test -p kengram-mcp --lib search::tests::search_thoughts_fts_timeout_causally_armed -- --exact --test-threads=1 --nocapture`
- capability_marker: `ThoughtFts`
- baseline_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `f885db7a4aba36536ea7f73de50b720fd4b38e96ac0c9d793882c63687a1dd00`
- RED_log: `receipts/delivery-a-frozen-six/M2_fts_timeout_healthy_empty-RED.log` exit=101
- restore_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M2_fts_timeout_healthy_empty-GREEN.log` exit=0
- **RECEIPT_OK** `M2_fts_timeout_healthy_empty`

## Mutant M3_rerank_recorder_deletion

- file: `crates/kengram-mcp/src/search.rs`
- transform: Rerank→ThoughtFts in rerank-fail degradation (recorder deletion)
- command: `cargo test -p kengram-mcp --lib search::tests::search_thoughts_rerank_timeout_via_tei_wiremock -- --exact --test-threads=1 --nocapture`
- capability_marker: `Rerank`
- baseline_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `2dabc18e53a2ef60d9f708e76ad1cd1893b0c204ed52dbf02e708ba2143ef03e`
- RED_log: `receipts/delivery-a-frozen-six/M3_rerank_recorder_deletion-RED.log` exit=101
- restore_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M3_rerank_recorder_deletion-GREEN.log` exit=0
- **RECEIPT_OK** `M3_rerank_recorder_deletion`

## Mutant M4_health_mount_removal

- file: `crates/kengram-cli/src/health.rs`
- transform: mount_health no longer registers GET /health
- command: `cargo test -p kengram-cli --bin kengram health::tests::health_route_via_axum_router -- --exact --test-threads=1 --nocapture`
- capability_marker: `health`
- baseline_sha256: `e7ce239d0a7af817695ab303299c2c9e6d17ad2c49d9e008fad3fcf872212b00`
- mutant_sha256: `8138f8518bbbefbff6f7dc5e10e8f29d8ffa707591c1471acd763f2db8d815a6`
- RED_log: `receipts/delivery-a-frozen-six/M4_health_mount_removal-RED.log` exit=101
- restore_sha256: `e7ce239d0a7af817695ab303299c2c9e6d17ad2c49d9e008fad3fcf872212b00` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M4_health_mount_removal-GREEN.log` exit=0
- **RECEIPT_OK** `M4_health_mount_removal`

## Mutant M5_mcp_degradations_serialization

- file: `crates/kengram-mcp/src/server.rs`
- transform: MCP JSON omits degradations field
- command: `cargo test -p kengram-mcp --lib server::tests::search_thoughts_tool_emits_degradations_json -- --exact --test-threads=1 --nocapture`
- capability_marker: `degradation`
- baseline_sha256: `995632cbf615e52b637d08c479392d2fe1b669dc9eb691dc5e7e8944111dca5d`
- mutant_sha256: `2e48dabe9e47bf70f5ea424d7c865f2ee216cacccfa8d6484d63f7556aa9affc`
- RED_log: `receipts/delivery-a-frozen-six/M5_mcp_degradations_serialization-RED.log` exit=101
- restore_sha256: `995632cbf615e52b637d08c479392d2fe1b669dc9eb691dc5e7e8944111dca5d` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M5_mcp_degradations_serialization-GREEN.log` exit=0
- **RECEIPT_OK** `M5_mcp_degradations_serialization`

## Mutant M6_neighbor_counter_index_shift

- file: `crates/kengram-mcp/src/degradation.rs`
- transform: counter cell uses (leg.index()+1)%ALL instead of leg.index() (neighbor shift)
- command: `cargo test -p kengram-mcp --lib degradation::tests::record_increments_exact_cell -- --exact --test-threads=1 --nocapture`
- capability_marker: `index`
- baseline_sha256: `dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49`
- mutant_sha256: `38b5c17401b7ef83c71dc9e4a3e77300a639f9b06822eb16893eaa7e80afbf46`
- RED_log: `receipts/delivery-a-frozen-six/M6_neighbor_counter_index_shift-RED.log` exit=101
- restore_sha256: `dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M6_neighbor_counter_index_shift-GREEN.log` exit=0
- **RECEIPT_OK** `M6_neighbor_counter_index_shift`

## Summary

**ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK** = `True` executed_code_head=`dc90f56cb97b5568b1c66c12c080928351461e72` certifier=`spike`
