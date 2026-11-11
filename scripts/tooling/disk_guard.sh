#!/usr/bin/env bash
# Disk space safety harness & automated cleanup for pydbsrt.
# Monitors target/ directory and free partition space.
set -euo pipefail

MAX_TARGET_MB="${MAX_TARGET_MB:-16384}" # 16 GB threshold
MIN_FREE_GB="${MIN_FREE_GB:-10}"        # 10 GB minimum free disk space

get_target_mb() {
  if [[ -d "target" ]]; then
    du -sm "target" 2>/dev/null | awk '{print $1}'
  else
    echo "0"
  fi
}

get_free_gb() {
  df -BG . 2>/dev/null | awk 'NR==2 {gsub("G","",$4); print $4}'
}

clean_all() {
  echo "🧹 Running unified disk cleanup..."
  if command -v cargo >/dev/null 2>&1 && [[ -d "target" ]]; then
    echo "  → Cleaning Rust build artifacts (cargo clean)..."
    cargo clean
  fi

  echo "  → Cleaning Python bytecode & caches (__pycache__, .pytest_cache, .ruff_cache)..."
  find . -type d -name "__pycache__" -prune -exec rm -rf {} + 2>/dev/null || true
  rm -rf .pytest_cache .mypy_cache .ruff_cache .coverage htmlcov/ site/

  echo "✅ Disk cleanup complete."
  echo "   Current free space: $(get_free_gb) GB"
}

check_guard() {
  local target_mb
  local free_gb
  target_mb="$(get_target_mb)"
  free_gb="$(get_free_gb)"

  echo "🛡️  Disk Space Safety Guard:"
  echo "   • Free partition space : ${free_gb} GB (threshold: > ${MIN_FREE_GB} GB)"
  echo "   • Rust target/ size    : ${target_mb} MB (threshold: < ${MAX_TARGET_MB} MB)"

  local alert=0

  if ((free_gb < MIN_FREE_GB)); then
    echo "⚠️  WARNING: Critical low free disk space (${free_gb} GB < ${MIN_FREE_GB} GB)!"
    alert=1
  fi

  if ((target_mb > MAX_TARGET_MB)); then
    echo "⚠️  WARNING: target/ size (${target_mb} MB) exceeds safety threshold (${MAX_TARGET_MB} MB)!"
    echo "   Auto-triggering 'cargo clean' to prevent disk exhaustion..."
    cargo clean
    target_mb="$(get_target_mb)"
    echo "   Reclaimed space. New target/ size: ${target_mb} MB."
    alert=0
  fi

  if ((alert == 0)); then
    echo "✅ Disk usage is healthy."
  fi
}

case "${1:-check}" in
  check)
    check_guard
    ;;
  clean)
    clean_all
    ;;
  --help | -h)
    echo "Usage: $0 [check|clean]"
    exit 0
    ;;
  *)
    echo "Unknown command: $1. Use 'check' or 'clean'." >&2
    exit 1
    ;;
esac
