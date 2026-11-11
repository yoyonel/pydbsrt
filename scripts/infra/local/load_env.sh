#!/usr/bin/env bash
# Sourced helper: loads the optional unversioned local env file (<repo>/.env, then
# ~/.config/pydbsrt/env) without overriding variables already set by the caller.

load_local_env() {
  local repo_root="${1:-}" env_file key
  if [[ -z "${repo_root}" ]]; then
    local script_dir
    script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    repo_root="$(cd "${script_dir}/../../.." && pwd)"
    if [[ ! -d "${repo_root}/.git" && -d "${HOME}/dbsrt" ]]; then
      repo_root="${HOME}/dbsrt"
    fi
  fi

  local -A preset=()
  for env_file in "${repo_root}/.env" "${HOME}/.config/pydbsrt/env"; do
    [[ -f "${env_file}" ]] || continue
    while IFS='=' read -r key _; do
      if [[ -n "${!key+x}" ]]; then
        preset["${key}"]="${!key}"
      fi
    done < <(grep -oE '^[A-Za-z_][A-Za-z0-9_]*=' "${env_file}")
    # shellcheck source=/dev/null
    source "${env_file}"
    for key in "${!preset[@]}"; do
      printf -v "${key}" '%s' "${preset[${key}]}"
    done
    return 0
  done
}

# Remote relay API key: DBSRT_REMOTE_API_KEY env, else the pass entry, else empty.
resolve_api_key() {
  if [[ -n "${DBSRT_REMOTE_API_KEY:-}" ]]; then
    echo "${DBSRT_REMOTE_API_KEY}"
    return 0
  fi
  if command -v pass >/dev/null 2>&1; then
    pass show "services/pydbsrt/o2switch_api_key" 2>/dev/null || true
  fi
}

# Standalone execution CLI support when invoked directly rather than sourced
if [[ "${BASH_SOURCE[0]}" == "${0}" ]]; then
  set -euo pipefail

  if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
    echo "Usage: $(basename "$0") [--check | --help]"
    echo ""
    echo "Sourced helper for loading local environment variables (.env, ~/.config/pydbsrt/env)."
    echo ""
    echo "Options:"
    echo "  --help, -h   Show this help message and exit"
    echo "  --check      Inspect and report discovered local environment configuration"
    exit 0
  fi

  echo "=== DBSrt Local Environment Helper Status ==="
  load_local_env ""
  echo "✓ Environment successfully loaded."
  if [[ -n "${O2SWITCH_URL:-}" ]]; then
    echo "  O2SWITCH_URL: ${O2SWITCH_URL}"
  else
    echo "  O2SWITCH_URL: (not set)"
  fi
  if [[ -n "$(resolve_api_key)" ]]; then
    echo "  API Key     : [CONFIGURED]"
  else
    echo "  API Key     : (not configured)"
  fi
fi
