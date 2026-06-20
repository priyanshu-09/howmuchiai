# howmuchiai web dashboard patch

TypeScript modules for the production dashboard at [howmuchiai.xyz](https://howmuchiai.xyz). The live site ships a bundled SPA; integrate these files into the dashboard source repo.

## Changes

- `src/lib/tokenDisplay.ts` — resolves logged vs `≈estimated` vs `—` unavailable; evidence-aware tooltips and `costEstimated` for Rs
- `src/lib/toolBreakdown.ts` — patched `buildToolBreakdown` reading `metadata.domains` token fields (schema v5)
- `src/lib/types.ts` — scanner JSON types including per-domain `tokens_unavailable` and schema v6 `tokens_estimate_evidence`

## Integration

Replace the minified `gw` breakdown function with `buildToolBreakdown` from `toolBreakdown.ts`. Headline totals must keep `totals.tokens` (logged) separate from `totals.estimated_tokens`.

## Test

```bash
cd web && npm install && npm test
```
