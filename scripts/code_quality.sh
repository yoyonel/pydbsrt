#!/usr/bin/env bash
# Unified code quality runner for lint and format across .py, .rs, .sh, .lua, .yml, .json, .html.
set -euo pipefail

ACTION="${1:-lint}"

run_lint_fast() {
  echo "=== [1/5] Python (Ruff Check & Format Check) ==="
  uv run ruff check src/ tests/ scripts/
  uv run ruff format --check src/ tests/ scripts/

  echo "=== [2/5] Shell Scripts (shellcheck & shfmt) ==="
  shellcheck scripts/*.sh data/*.sh
  shfmt -i 2 -ci -d scripts/*.sh data/*.sh

  echo "=== [3/5] Lua VLC Plugins (luac bytecode check & hermetic unit tests) ==="
  for lua_file in vlc-plugin/*.lua; do
    luac -p "$lua_file"
  done
  lua tests/lua/test_vlsub_dbsrt.lua
  echo "✓ All Lua plugins compiled cleanly and passed unit tests (vlc-plugin/*.lua)"

  echo "=== [4/5] YAML Configs (yamllint) ==="
  yamllint -c .yamllint.yml Taskfile.yml mkdocs.yml bandit.yaml .pre-commit-config.yaml .yamllint.yml docker/*.yml .github/workflows/*.yml
  echo "✓ All YAML files passed yamllint"

  echo "=== [5/5] HTML / Embedded JS / JSON Assets ==="
  uv run python scripts/validate_web_assets.py
}

run_lint() {
  run_lint_fast
  echo "=== [6/7] Rust (cargo fmt --check & clippy) ==="
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings

  echo "=== [7/7] Disk Space Safety Guard ==="
  bash scripts/disk_guard.sh check
}

run_fmt() {
  echo "=== [1/3] Formatting Python (Ruff) ==="
  uv run ruff format src/ tests/ scripts/
  uv run ruff check --fix src/ tests/ scripts/

  echo "=== [2/3] Formatting Rust (cargo fmt) ==="
  cargo fmt --all

  echo "=== [3/3] Formatting Shell Scripts (shfmt) ==="
  shfmt -i 2 -ci -w scripts/*.sh data/*.sh
}

run_coverage() {
  echo "=== [1/3] Rust Coverage (cargo llvm-cov --workspace) ==="
  cargo llvm-cov --workspace --json --output-path coverage-rust.json

  echo "=== [2/3] Python Coverage (pytest --cov=src) ==="
  uv run pytest --cov=src --cov-report=json:coverage-python.json tests/tools/
  if (echo >"/dev/tcp/${PSQL_IP:-127.0.0.1}/${PSQL_PORT:-5433}") >/dev/null 2>&1; then
    PSQL_PORT="${PSQL_PORT:-5433}" uv run pytest --cov=src --cov-report=json:coverage-python-db.json tests/ --ignore=tests/tools/
    bash scripts/seed_couplings.sh >/dev/null 2>&1 || true
  fi

  echo "=== [3/3] Unified Coverage Report (Rust & Python) ==="
  uv run python scripts/generate_coverage_report.py \
    --rust-json coverage-rust.json \
    --python-json coverage-python.json \
    --python-db-json coverage-python-db.json \
    --output-md coverage-summary.md
  rm -f coverage-rust.json coverage-python.json coverage-python-db.json coverage-summary.md .coverage
}

case "$ACTION" in
  lint) run_lint ;;
  lint-fast) run_lint_fast ;;
  fmt) run_fmt ;;
  coverage) run_coverage ;;
  *)
    echo "Usage: $0 {lint|lint-fast|fmt|coverage}" >&2
    exit 1
    ;;
esac
