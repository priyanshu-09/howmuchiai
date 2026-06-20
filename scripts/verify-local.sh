#!/usr/bin/env bash
# Local verification before merging PR-13. Run from repo root:
#   ./scripts/verify-local.sh
#   ./scripts/verify-local.sh --full-scan
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT/scanner"

echo "==> Unit tests"
cargo test

echo "==> Browser data audit tests"
cargo test --lib browser_data_audit -- --nocapture 2>&1 | tail -8

echo "==> Clippy"
cargo clippy -- -D warnings

if [[ -d "$ROOT/web/node_modules" ]]; then
  echo "==> Dashboard patch tests"
  (cd "$ROOT/web" && npm test)
else
  echo "==> Skipping web tests (run: cd web && npm install)"
fi

echo "==> Priority web domain tiered estimates (live LS probe on your machine)"
for t in chatgpt_web_estimates priority_web_domains; do
  cargo test --lib "$t" -- --nocapture 2>&1 | tail -4
done

if [[ "${VERIFY_LIVE_AUDIT:-0}" == "1" ]]; then
  echo "==> Live browser exhaustion audit (VERIFY_LIVE_AUDIT=1)"
  cargo test --lib live_browser_audit_exhaustion -- --ignored --nocapture
else
  echo "==> Skipping live browser audit (set VERIFY_LIVE_AUDIT=1 to run)"
fi

if [[ "${VERIFY_CURSOR:-0}" == "1" ]]; then
  echo "==> Cursor live DB cross-check (VERIFY_CURSOR=1; debug build ~10–15 min)"
  cargo test --lib verify_real_cursor_db_crosscheck -- --ignored --nocapture
else
  echo "==> Skipping Cursor live DB cross-check (set VERIFY_CURSOR=1 to run; ~10–15 min)"
fi

if [[ "${1:-}" == "--full-scan" ]]; then
  echo "==> Full release scan (5–10 min; run ONE instance only)"
  cd "$ROOT"
  cargo build --release
  OUT="${TMPDIR:-/tmp}/howmuchiai-verify-$$.json"
  ./target/release/howmuchiai --no-open --format json 2>/dev/null >"$OUT"
  python3 - "$OUT" <<'PY'
import json, sys
path = sys.argv[1]
with open(path) as f:
    d = json.load(f)
print(f"schema_version: {d['schema_version']}")
print(f"totals.estimated_tokens: {d['totals'].get('estimated_tokens', 0):,}")
c = d['sources'].get('cursor', {})
print(f"Cursor: hours={c.get('hours')}, estimated={c.get('tokens_estimated')}, total={(c.get('tokens') or {}).get('total', 0):,}")
priority = ["ChatGPT", "NotebookLM", "Gemini", "Perplexity"]
other = ["Grok", "Lovable", "AI Studio"]
for src in d['sources'].values():
    for name, dom in (src.get('metadata') or {}).get('domains', {}).items():
        if (name not in priority and name not in other) or (dom.get('hours') or 0) <= 0:
            continue
        ev = dom.get('tokens_estimate_evidence') or {}
        if dom.get('tokens_estimated'):
            tier = ev.get('winner_tier', '?')
            accurate = ev.get('accurate_count_possible')
            print(f"  {name}: {dom['hours']:.1f}h ESTIMATED {(dom.get('tokens') or {}).get('total', 0):,} tier={tier} accurate={accurate}")
            if name in priority:
                assert 'accurate_count_possible' in ev, f"{name}: missing evidence.accurate_count_possible"
                if ev.get('winner_tier') in (2, 2.0, 3, 3.0):
                    assert ev.get('accurate_count_possible') is False, f"{name}: tier 2/3 must not be accurate"
        elif dom.get('tokens_unavailable'):
            print(f"  {name}: {dom['hours']:.1f}h UNAVAILABLE")
        else:
            print(f"  {name}: {dom['hours']:.1f}h MISSING (bug)")
            raise SystemExit(1)
print(f"Full JSON: {path}")
PY
else
  echo ""
  echo "Quick checks done. For end-to-end JSON:"
  echo "  ./scripts/verify-local.sh --full-scan"
  echo "  VERIFY_LIVE_AUDIT=1 ./scripts/verify-local.sh  # live exhaustion audit"
fi
