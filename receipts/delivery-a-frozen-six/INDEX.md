# KENGRAM Delivery A — frozen six mutant receipts (diesel ceremony repair)

- executed_code_head: `1b9b5ca9342e97129958bb7cb1df0d4c8e0b3171`
- certifier: `diesel`
- when: `2026-08-09T18:50:38.259730+00:00`
- frozen_spec_section: `10`

Checker: exactly-one-preimage; RED only with capability-after-entry + unique
`KENGRAM_DELIVERY_A_RED` terminal marker; reject compile/setup; exact restore;
GREEN; `RECEIPT_OK` never unconditional.

## Mutant M1_query_embedding_recorder_deletion

- file: `crates/kengram-mcp/src/search.rs`
- transform: DELETE query-embedding recorder call (SearchLeg::QueryEmbedding record_degradation)
- preimage_count: `1`
- command: `cargo test -p kengram-mcp --lib search::tests::search_thoughts_degrades_when_embedder_fails -- --exact --test-threads=1 --nocapture`
- capability_after_entry: `search_thoughts_degrades_when_embedder_fails`
- terminal_marker: `KENGRAM_DELIVERY_A_RED:V1_query_embedding_receipt`
- baseline_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `cb0b7bafc527800737f9d691c727d68d7305881f69cec2e23c003a638bcc7753`
- RED_log: `receipts/delivery-a-frozen-six/M1_query_embedding_recorder_deletion-RED.log` exit=101
- restore_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M1_query_embedding_recorder_deletion-GREEN.log` exit=0
- **RECEIPT_OK** `M1_query_embedding_recorder_deletion` = `True`

## Mutant M2_fts_timeout_healthy_empty

- file: `crates/kengram-mcp/src/search.rs`
- transform: ThoughtFts timeout fail-open becomes healthy empty (no record)
- preimage_count: `1`
- command: `cargo test -p kengram-mcp --lib search::tests::search_thoughts_fts_timeout_causally_armed -- --exact --test-threads=1 --nocapture`
- capability_after_entry: `search_thoughts_fts_timeout_causally_armed`
- terminal_marker: `KENGRAM_DELIVERY_A_RED:V2_thought_fts_timeout_receipt`
- baseline_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `6b86e8f35cb7a3980877864b8b4ad1c4e314e548afa3c59dc033ed7601ec6884`
- RED_log: `receipts/delivery-a-frozen-six/M2_fts_timeout_healthy_empty-RED.log` exit=101
- restore_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M2_fts_timeout_healthy_empty-GREEN.log` exit=0
- **RECEIPT_OK** `M2_fts_timeout_healthy_empty` = `True`

## Mutant M3_rerank_recorder_deletion

- file: `crates/kengram-mcp/src/search.rs`
- transform: DELETE/BYPASS rerank recorder call on timeout branch (return false without record)
- preimage_count: `1`
- command: `cargo test -p kengram-mcp --lib search::tests::search_thoughts_rerank_timeout_via_tei_wiremock -- --exact --test-threads=1 --nocapture`
- capability_after_entry: `search_thoughts_rerank_timeout_via_tei_wiremock`
- terminal_marker: `KENGRAM_DELIVERY_A_RED:V3_rerank_timeout_receipt`
- baseline_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- mutant_sha256: `120b0a8149b2d127d93ed530f7e2cf25877a973e9bd16674f9025cb27c2c94e9`
- RED_log: `receipts/delivery-a-frozen-six/M3_rerank_recorder_deletion-RED.log` exit=101
- restore_sha256: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M3_rerank_recorder_deletion-GREEN.log` exit=0
- **RECEIPT_OK** `M3_rerank_recorder_deletion` = `True`

## Mutant M4_health_mount_removal

- file: `crates/kengram-cli/src/health.rs`
- transform: Remove GET /health route mount
- preimage_count: `1`
- command: `cargo test -p kengram-cli --bin kengram health::tests::health_route_via_axum_router -- --exact --test-threads=1 --nocapture`
- capability_after_entry: `health_route_via_axum_router`
- terminal_marker: `KENGRAM_DELIVERY_A_RED:V4_health_route_mount`
- baseline_sha256: `e7ce239d0a7af817695ab303299c2c9e6d17ad2c49d9e008fad3fcf872212b00`
- mutant_sha256: `58dfa031dba05bc0c1c12a698a14ca92e0b26fdef5321760465aa8f842c842f1`
- RED_log: `receipts/delivery-a-frozen-six/M4_health_mount_removal-RED.log` exit=101
- restore_sha256: `e7ce239d0a7af817695ab303299c2c9e6d17ad2c49d9e008fad3fcf872212b00` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M4_health_mount_removal-GREEN.log` exit=0
- **RECEIPT_OK** `M4_health_mount_removal` = `True`

## Mutant M5_mcp_degradations_serialization

- file: `crates/kengram-mcp/src/server.rs`
- transform: Remove degradations from MCP JSON serialization
- preimage_count: `1`
- command: `cargo test -p kengram-mcp --lib server::tests::search_thoughts_tool_emits_degradations_json -- --exact --test-threads=1 --nocapture`
- capability_after_entry: `search_thoughts_tool_emits_degradations_json`
- terminal_marker: `KENGRAM_DELIVERY_A_RED:V4_mcp_degradations_serialization`
- baseline_sha256: `995632cbf615e52b637d08c479392d2fe1b669dc9eb691dc5e7e8944111dca5d`
- mutant_sha256: `e763325fe801bc4877f0e39d78e5daf4a241a0db0283e9132f12920785ecc2c1`
- RED_log: `receipts/delivery-a-frozen-six/M5_mcp_degradations_serialization-RED.log` exit=101
- restore_sha256: `995632cbf615e52b637d08c479392d2fe1b669dc9eb691dc5e7e8944111dca5d` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M5_mcp_degradations_serialization-GREEN.log` exit=0
- **RECEIPT_OK** `M5_mcp_degradations_serialization` = `True`

## Mutant M6_neighbor_counter_index_shift

- file: `crates/kengram-mcp/src/degradation.rs`
- transform: Per-leg counter index shifted to neighboring enum member
- preimage_count: `1`
- command: `cargo test -p kengram-mcp --lib degradation::tests::record_increments_exact_cell -- --exact --test-threads=1 --nocapture`
- capability_after_entry: `record_increments_exact_cell`
- terminal_marker: `KENGRAM_DELIVERY_A_RED:M6_counter_index`
- baseline_sha256: `dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49`
- mutant_sha256: `c016561647ade42e6a0d51acfc4388c0ed81cb81d351d9c08b0e5ba1c81f148a`
- RED_log: `receipts/delivery-a-frozen-six/M6_neighbor_counter_index_shift-RED.log` exit=101
- restore_sha256: `dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49` MATCH
- GREEN_log: `receipts/delivery-a-frozen-six/M6_neighbor_counter_index_shift-GREEN.log` exit=0
- **RECEIPT_OK** `M6_neighbor_counter_index_shift` = `True`

## Summary

**ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK** = `True` executed_code_head=`1b9b5ca9342e97129958bb7cb1df0d4c8e0b3171`

Final file hashes after ceremony:

- `search.rs`: `27e0a0deb61b83a656f45cc67a139816eb275d9454eb5a71cd4be325e649e0a9`
- `degradation.rs`: `dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49`
- `server.rs`: `995632cbf615e52b637d08c479392d2fe1b669dc9eb691dc5e7e8944111dca5d`
- `health.rs`: `e7ce239d0a7af817695ab303299c2c9e6d17ad2c49d9e008fad3fcf872212b00`
