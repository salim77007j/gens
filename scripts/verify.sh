#!/usr/bin/env bash
# Gens - local verification gate (Phase 0 §0.3 and §0.8).
# Run before EVERY push:  ./scripts/verify.sh
# CI (`.github/workflows/ci.yml`) mirrors these gates; this is the fast local pre-flight.
set -euo pipefail
cd "$(dirname "$0")/.."

say() { printf '==> %s\n' "$*"; }
die() { printf 'VERIFY: FAIL - %s\n' "$*" >&2; exit 1; }

say "[0/17] governance docs present"
for f in docs/WORKLOG.md docs/DECISIONS.md docs/RISKS.md docs/ROLLBACK_PLAN.md docs/EXPERIMENTS.md; do
  [ -f "$f" ] || die "missing $f"
done

say "[1/17] secret scan"
if git grep -nE 'gh[pousr]_[A-Za-z0-9]{20,}' -- . 2>/dev/null; then
  die "possible GitHub credential in tracked files (see docs/ROLLBACK_PLAN.md section 5)"
fi

if [ ! -f Cargo.toml ]; then
  say "no Cargo workspace yet (Phase 0) - Rust gates armed, not active"
  echo "VERIFY: PASS (phase-0 scope)"
  exit 0
fi

say "[2/17] cargo fmt";    cargo fmt --all --check
say "[3/17] cargo clippy"; cargo clippy --workspace --all-targets -- -D warnings
say "[4/17] cargo build";  cargo build --workspace --all-targets
say "[5/17] cargo test";   cargo test --workspace
say "[6/17] physics smoke (input -> physics -> ECS replay, CPU-only, deterministic)"
cargo run -q -p gens-runtime --example physics_hello -- --steps 600 --out /tmp/gens_verify_physics.json
echo "physics smoke: $(wc -c < /tmp/gens_verify_physics.json) bytes of trajectory evidence"

say "[7/17] animation+AI+UI smoke (BT -> animation -> ECS -> egui layout replay)"
cargo run -q -p gens-runtime --example animate_hello -- --steps 900 --out /tmp/gens_verify_animate.json
echo "animate smoke: $(wc -c < /tmp/gens_verify_animate.json) bytes of trajectory evidence"

say "[8/17] assets+scripting+save smoke (glTF -> Rhai script -> save/load replay, CPU-only)"
cargo run -q -p gens-runtime --example asset_script_hello -- --steps 1200 --out /tmp/gens_verify_asset_script.json
echo "asset+script smoke: $(wc -c < /tmp/gens_verify_asset_script.json) bytes of save/load replay evidence"

say "[9/17] networking smoke (data document -> QUIC session -> world replication, loopback only)"
cargo run -q -p gens-runtime --example net_hello -- --steps 600 --replicate-at 300 --out /tmp/gens_verify_net.json
echo "net smoke: $(wc -c < /tmp/gens_verify_net.json) bytes of replication evidence"

say "[10/17] GPU smoke (offscreen wgpu render; skips honestly without an adapter)"
# Local machines may lack a Vulkan driver: --allow-no-gpu makes the
# absence explicit instead of failing. CI (Linux+macOS) renders for real;
# see ADR-0011 for the per-OS guarantees.
cargo run -q -p gens-runtime --example gpu_hello -- --frames 60 --out /tmp/gens_verify_gpu.png --allow-no-gpu
if [ -f /tmp/gens_verify_gpu.png ]; then
  echo "gpu smoke: $(wc -c < /tmp/gens_verify_gpu.png) bytes rendered locally"
else
  echo "gpu smoke: GPU-SKIP (no adapter here; CI renders on Linux/macOS)"
fi
say "[11/17] editor example build (the example itself needs an adapter; CI renders it)"
cargo build -q -p gens-editor --examples
echo "editor example: built (GPU render proven in CI on Linux/macOS)"

say "[12/17] profiler smoke (observation-only proof + phase timings + memory, CPU-only)"
cargo run -q -p gens-runtime --example profile_hello -- --steps 600 --out /tmp/gens_verify_profile.json
echo "profiler smoke: $(wc -c < /tmp/gens_verify_profile.json) bytes of ADR-0027 proof evidence"

say "[13/17] cinematic smoke (ADR-0030 replay + seek-equivalence proof, CPU-only)"
cargo run -q -p gens-runtime --example cinematic_hello -- --steps 300 --out /tmp/gens_verify_cinematic.json
echo "cinematic smoke: $(wc -c < /tmp/gens_verify_cinematic.json) bytes of replay-proof evidence"

say "[14/17] worldgen smoke (ADR-0031 order/streaming/restart proofs, CPU-only)"
cargo run -q -p gens-runtime --example worldgen_hello -- --steps 300 --out /tmp/gens_verify_worldgen.json
echo "worldgen smoke: $(wc -c < /tmp/gens_verify_worldgen.json) bytes of generation evidence"

say "[15/17] plugin smoke (ADR-0032 out-of-tree downstream proof, CPU-only)"
# The proof crate builds OUTSIDE the workspace; CARGO_TARGET_DIR shares
# the workspace target (R-032). A failure here means the plugin API is
# broken — a gate, not a warning.
(cd plugins/hello-plugin && cargo run -q --bin hello-plugin-demo -- --frames 120 --out /tmp/gens_verify_plugin.json)
echo "plugin smoke: $(wc -c < /tmp/gens_verify_plugin.json) bytes of API evidence"

say "[16/17] build-pipeline smoke (package build -> verify -> headless player run, CPU-only)"
cargo run -q -p gens-build --example package_hello -- --out /tmp/gens_verify_package.json
echo "package smoke: $(wc -c < /tmp/gens_verify_package.json) bytes of export-pipeline evidence"

say "[17/17] binary size gates (debug + release, ADR-0012 + ADR-0029)"
# Debug calibrated in Phase 2 (ADR-0012), re-baselined in Phase 8 (the dev
# profile moved to line-tables-only debuginfo): 170.1 MiB measured, gate =
# 350 MiB (2.1x headroom, unchanged limit).
DEBUG_LIMIT_BYTES=$((350 * 1024 * 1024))
cargo build -q -p gens-runtime --example gpu_hello
DEBUG_SIZE=$(wc -c < target/debug/examples/gpu_hello)
echo "gpu_hello debug binary: $DEBUG_SIZE bytes (limit $DEBUG_LIMIT_BYTES)"
if [ "$DEBUG_SIZE" -gt "$DEBUG_LIMIT_BYTES" ]; then
  die "debug size gate failed: $DEBUG_SIZE > $DEBUG_LIMIT_BYTES (see docs/DECISIONS.md ADR-0012)"
fi
# Release calibrated in Phase 8 (ADR-0029): thin-LTO release binary
# measured 8.3 MiB on Linux x86_64; gate = 20 MiB (2.4x headroom).
# DISK-AWARE SKIP (R-028 discipline: skips are loud and auditable): the
# release dependency tree needs ~2.5 GB; on constrained sandboxes where
# debug + release cannot coexist, the gate prints an explicit skip and CI
# (>= 14 GB runners) enforces it on every push.
FREE_KB=$(df -k / | tail -1 | awk '{print $4}')
if [ "$FREE_KB" -lt $((3 * 1024 * 1024)) ]; then
  echo "RELEASE-GATE-SKIP: only ${FREE_KB}KB free (<3GB); the release tree cannot coexist with debug here."
  echo "RELEASE-GATE-SKIP: CI enforces the 20 MiB release gate on every push (see ci.yml)."
else
  RELEASE_LIMIT_BYTES=$((20 * 1024 * 1024))
  cargo build -q --release -p gens-runtime --example gpu_hello
  RELEASE_SIZE=$(wc -c < target/release/examples/gpu_hello)
  echo "gpu_hello release binary: $RELEASE_SIZE bytes (limit $RELEASE_LIMIT_BYTES)"
  if [ "$RELEASE_SIZE" -gt "$RELEASE_LIMIT_BYTES" ]; then
    die "release size gate failed: $RELEASE_SIZE > $RELEASE_LIMIT_BYTES (see docs/DECISIONS.md ADR-0029)"
  fi
fi

echo "VERIFY: PASS"
