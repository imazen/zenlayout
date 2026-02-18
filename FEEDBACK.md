# Feedback Log

## 2026-02-18
- User requested extraction of RIAPI parsing reference from imageflow for zenlayout implementation.
- User requested implementation of RIAPI query string parsing plan (Phase 1). Implemented full module: parse.rs, instructions.rs, color.rs, convert.rs, mod.rs. 71 parity tests passing. User emphasized "continue to get parity correct" and "ensure we gracefully compose with parsers for the remaining stuff" — extras BTreeMap preserves all non-layout keys without warnings.
