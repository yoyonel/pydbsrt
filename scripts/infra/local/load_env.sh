#!/usr/bin/env bash
# Sourced helper: loads the optional unversioned local env file (<repo>/.env, then
# ~/.config/pydbsrt/env) without overriding variables already set by the caller.

load_local_env() {
  local repo_root="$1" env_file key
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
