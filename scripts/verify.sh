#!/usr/bin/env bash
# Gens - local verification gate (Phase 0 §0.3 and §0.8).
# Run before EVERY push:  ./scripts/verify.sh
# CI (`.github/workflows/ci.yml`) mirrors these gates; this is the fast local pre-flight.
set -euo pipefail
cd "$(dirname "$0")/.."

say() { printf '==> %s\n' "$*"; }
die() { printf 'VERIFY: FAIL - %s\n' "$*" >&2; exit 1; }

say "[0/6] governance docs present"
for f in docs/WORKLOG.md docs/DECISIONS.md docs/RISKS.md docs/ROLLBACK_PLAN.md docs/EXPERIMENTS.md; do
  [ -f "$f" ] || die "missing $f"
done

say "[1/6] secret scan"
if git grep -nE 'gh[pousr]_[A-Za-z0-9]{20,}' -- . 2>/dev/null; then
  die "possible GitHub credential in tracked files (see docs/ROLLBACK_PLAN.md section 5)"
fi

if [ ! -f Cargo.toml ]; then
  say "no Cargo workspace yet (Phase 0) - Rust gates armed, not active"
  echo "VERIFY: PASS (phase-0 scope)"
  exit 0
fi

say "[2/6] cargo fmt";    cargo fmt --all --check
say "[3/6] cargo clippy"; cargo clippy --workspace --all-targets -- -D warnings
say "[4/6] cargo build";  cargo build --workspace --all-targets
say "[5/6] cargo test";   cargo test --workspace
say "[6/6] binary size gate"
# Calibrated with real binaries in Phase 1; threshold recorded in docs/DECISIONS.md.
echo "(placeholder - calibrated in Phase 1)"

echo "VERIFY: PASS"
