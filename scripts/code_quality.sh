#!/usr/bin/env bash
# Unified code quality runner for lint and format across .py, .rs, .sh, .lua, .yml, .json, .html.
set -euo pipefail

ACTION="${1:-lint}"

run_lint() {
  echo "=== [1/6] Python (Ruff Check & Format Check) ==="
  uv run ruff check src/ tests/ scripts/
  uv run ruff format --check src/ tests/ scripts/

  echo "=== [2/6] Rust (cargo fmt --check & clippy) ==="
  cargo fmt --all -- --check
  cargo clippy --workspace --all-targets -- -D warnings

  echo "=== [3/6] Shell Scripts (shellcheck & shfmt) ==="
  shellcheck scripts/*.sh data/*.sh
  shfmt -i 2 -ci -d scripts/*.sh data/*.sh

  echo "=== [4/6] Lua VLC Plugins (luac bytecode syntax check) ==="
  for lua_file in vlc-plugin/*.lua; do
    luac -p "$lua_file"
  done
  echo "✓ All Lua plugins compiled cleanly (vlc-plugin/*.lua)"

  echo "=== [5/6] YAML Configs (yamllint) ==="
  yamllint -c .yamllint.yml Taskfile.yml mkdocs.yml bandit.yaml .pre-commit-config.yaml .yamllint.yml docker/*.yml .github/workflows/*.yml
  echo "✓ All YAML files passed yamllint"

  echo "=== [6/6] HTML / Embedded JS / JSON Assets ==="
  uv run python scripts/validate_web_assets.py
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

case "$ACTION" in
  lint) run_lint ;;
  fmt) run_fmt ;;
  *)
    echo "Usage: $0 {lint|fmt}" >&2
    exit 1
    ;;
esac
