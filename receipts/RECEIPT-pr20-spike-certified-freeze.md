# PR20 spike-certified freeze

- certified_head: 
- code_head_F1: `dc90f56cb97b5568b1c66c12c080928351461e72`
- certifier: spike
- when: `2026-08-09T18:34:06Z`
- smith_shape: 324df2a9 two-item residual

## F1 verification (spike)
- Code: expansion + pairwise record when fails>0 without rankings.is_empty gate
- `search::tests::search_pairwise_fts_timeout_failed_attempts_aggregated` PASS
- `degradation::` 4/4 PASS

## F2 verification (spike regenerated, not heavy-trusted)
- Ceremony: `scripts/delivery-a-six-mutants-ceremony.py`
- Receipt: `receipts/RECEIPT-delivery-a-frozen-six-mutants.md`
- Raw: `receipts/delivery-a-frozen-six/*-RED|GREEN.log`
- M6 RED is **assertion** fail (counter index), not compile typo
- ALL_SIX_FROZEN_MUTANT_RECEIPTS_OK under spike

## Heavy input
- 74d0ba7 treated as input evidence; F1 code retained (dc90f56); F2 logs re-run under spike control

No deploy.
