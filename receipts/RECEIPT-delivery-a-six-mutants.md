# KENGRAM Delivery A — six watched-mutant receipts (ITEM B)

- exact_head: `875809af1ff610faccca899ce69d9d7e0951c9ac`
- tree: `ca9c3558002035669f50a7802fd5418db753d8d6`
- host: `Yeti-Mini.local`
- when: `2026-08-09T18:05:24Z`
- DATABASE_URL_set: `True`

Ceremony: baseline hash → capability-breaking mutation → cargo test RED →
`git checkout -- file` exact restore → cargo test GREEN → final hash equals baseline.

## Mutant V2_pairwise_failed_attempts

- file: `crates/kengram-mcp/src/search.rs`
- package: `kengram-mcp` filter: `search_pairwise_fts_timeout_failed_attempts_aggregated`
- baseline_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47`
- mutant_sha256: `cf8b09154c5d4b9e4b7d74b35aa150cda5395e6f28bb144a2a893b61c25b0220`
- capability_after_entry_RED: exit=101 PASS_RED
- restored_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47` MATCH
- after_restore_GREEN: exit=0 PASS_GREEN
- **RECEIPT_OK** `V2_pairwise_failed_attempts`

## Mutant V2_pairwise_one_logical_receipt

- file: `crates/kengram-mcp/src/search.rs`
- package: `kengram-mcp` filter: `search_pairwise_fts_timeout_failed_attempts_aggregated`
- baseline_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47`
- mutant_sha256: `96365832e64fb5fe8b7131662bf693d7ff3c6d2401a3827816e46c7ffd129b20`
- capability_after_entry_RED: exit=101 PASS_RED
- restored_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47` MATCH
- after_restore_GREEN: exit=0 PASS_GREEN
- **RECEIPT_OK** `V2_pairwise_one_logical_receipt`

## Mutant M6_counter_index

- file: `crates/kengram-mcp/src/degradation.rs`
- package: `kengram-mcp` filter: `record_increments_exact_cell`
- baseline_sha256: `dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49`
- mutant_sha256: `87df5b57331601e3e591f916907dd3840851f80bad16d4ebacd3ced9698be34f`
- capability_after_entry_RED: exit=101 PASS_RED
- restored_sha256: `dcceacec6028e4f9772ad8a843b3dc992221fa2a93bc1d5c0aa6dbab1a626c49` MATCH
- after_restore_GREEN: exit=0 PASS_GREEN
- **RECEIPT_OK** `M6_counter_index`

## Mutant V1_query_embedding_receipt

- file: `crates/kengram-mcp/src/search.rs`
- package: `kengram-mcp` filter: `query_embedding`
- baseline_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47`
- mutant_sha256: `4401fa6f92b85a889a1b5bc68481b744adbcbea07137b30d687fbc5561265504`
- capability_after_entry_RED: exit=101 PASS_RED
- restored_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47` MATCH
- after_restore_GREEN: exit=0 PASS_GREEN
- **RECEIPT_OK** `V1_query_embedding_receipt`

## Mutant F1_unequal_thought_fts_routed

- file: `crates/kengram-mcp/src/search.rs`
- package: `kengram-mcp` filter: `lexical_timeouts_unequal`
- baseline_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47`
- mutant_sha256: `3b6e8eb7c359cda903c7b7ea383e6a73206c7028c018abf36afef97ef43bf53d`
- capability_after_entry_RED: exit=101 PASS_RED
- restored_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47` MATCH
- after_restore_GREEN: exit=0 PASS_GREEN
- **RECEIPT_OK** `F1_unequal_thought_fts_routed`

## Mutant V3_rerank_timeout_receipt

- file: `crates/kengram-mcp/src/search.rs`
- package: `kengram-mcp` filter: `rerank_timeout`
- baseline_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47`
- mutant_sha256: `4445b0375a227f415699e95a1c42a033f3fc150bf4692ea36fd744d68ed003d1`
- capability_after_entry_RED: exit=101 PASS_RED
- restored_sha256: `7e24204d2595f79ed8632617ddc10d089efb5e1a65ec2a0887104b6fd6da1f47` MATCH
- after_restore_GREEN: exit=0 PASS_GREEN
- **RECEIPT_OK** `V3_rerank_timeout_receipt`

## Summary

**ALL_SIX_MUTANT_RECEIPTS_OK** exact_head=`875809af1ff610faccca899ce69d9d7e0951c9ac`
