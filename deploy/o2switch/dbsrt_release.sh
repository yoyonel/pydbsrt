#!/usr/bin/env bash
# Remote-side releases of the dbsrt binaries on o2switch: activate | rollback | list | running | prune.
# Run by scripts/o2switch_ctl.sh over SSH (deployed to ~/dbsrt/bin/).
# Layout: releases/<id>/{dbsrt-server,dbsrt,SHA256SUMS}, current -> releases/<id>, previous -> the
# release active before, bin/<binary> -> ../current/<binary>. One rename of `current` switches both
# binaries at once; a running daemon keeps its inode until restarted.
# usage: dbsrt_release.sh <action> [base_dir] [id|keep|socket]   (relative paths resolve from $HOME)
set -euo pipefail

ACTION="${1:-list}"
cd "${HOME}"
abs() { [[ "$1" == /* ]] && printf '%s' "$1" || printf '%s/%s' "${HOME}" "$1"; }
BASE="$(abs "${2:-dbsrt}")"
ARG="${3:-}"
BINARIES=(dbsrt-server dbsrt)
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

fail() {
  echo "❌ $*"
  exit 1
}

# Release id of a state link (current/previous), empty when absent
link_id() { [[ -L "${BASE}/$1" ]] && basename "$(readlink "${BASE}/$1")" || true; }

# Atomic: rename(2) of a fresh link over the old one, never a window without the link
switch_link() {
  ln -sfn "$2" "$1.new"
  mv -T "$1.new" "$1"
}

# Same id as computed by o2switch_ctl.sh deploy: hash of the SHA256SUMS listing
release_id() { sha256sum "$1/SHA256SUMS" | cut -c1-16; }

# An id names a directory under releases/: no path separator, no dot-only name
check_release() {
  local id="$1"
  [[ "${id}" =~ ^[A-Za-z0-9_-][A-Za-z0-9._-]*$ ]] || fail "invalid release id '${id}'."
  [[ -d "${BASE}/releases/${id}" ]] || fail "release ${id} not found."
  for b in "${BINARIES[@]}"; do
    grep -qE "^[0-9a-f]{64}  ${b}\$" "${BASE}/releases/${id}/SHA256SUMS" 2>/dev/null ||
      fail "release ${id}: ${b} missing from SHA256SUMS."
  done
  (cd "${BASE}/releases/${id}" && sha256sum --quiet -c SHA256SUMS >/dev/null 2>&1) ||
    fail "release ${id}: checksum mismatch."
}

# First activation on a host deployed by copy: keeps the running binaries as a release to roll back to
import_legacy() {
  [[ ! -L "${BASE}/current" && -f "${BASE}/bin/dbsrt-server" && ! -L "${BASE}/bin/dbsrt-server" ]] || return 0
  local tmp="${BASE}/releases/.import.$$" id
  mkdir -p "${tmp}"
  for b in "${BINARIES[@]}"; do
    cp -p "${BASE}/bin/${b}" "${tmp}/${b}"
  done
  (cd "${tmp}" && sha256sum "${BINARIES[@]}" >SHA256SUMS)
  id="$(release_id "${tmp}")"
  if [[ -d "${BASE}/releases/${id}" ]]; then rm -rf "${tmp}"; else mv -T "${tmp}" "${BASE}/releases/${id}"; fi
  switch_link "${BASE}/current" "releases/${id}"
  echo "-> Imported the copied binaries as release ${id}."
}

# Ids match check_release (no whitespace): word splitting is safe
releases_newest_first() {
  [[ -d "${BASE}/releases" ]] || return 0
  find "${BASE}/releases" -mindepth 1 -maxdepth 1 -type d ! -name ".*" -printf "%T@ %f\n" | sort -rn | cut -d" " -f2
}

link_bins() {
  mkdir -p "${BASE}/bin"
  for b in "${BINARIES[@]}"; do
    switch_link "${BASE}/bin/${b}" "../current/${b}"
  done
}

case "${ACTION}" in
  activate)
    check_release "${ARG}"
    import_legacy
    OLD="$(link_id current)"
    if [[ "${OLD}" != "${ARG}" ]]; then
      switch_link "${BASE}/current" "releases/${ARG}"
      [[ -z "${OLD}" ]] || switch_link "${BASE}/previous" "releases/${OLD}"
    fi
    link_bins
    # prune keeps the most recently activated releases
    touch "${BASE}/releases/${ARG}"
    echo "✓ Release ${ARG} active (previous: ${OLD:-none}). Restart the daemon to serve it."
    ;;

  rollback)
    PREV="$(link_id previous)"
    [[ -n "${PREV}" ]] || fail "no previous release to roll back to."
    check_release "${PREV}"
    CUR="$(link_id current)"
    switch_link "${BASE}/current" "releases/${PREV}"
    [[ -z "${CUR}" ]] || switch_link "${BASE}/previous" "releases/${CUR}"
    link_bins
    touch "${BASE}/releases/${PREV}"
    echo "✓ Rolled back to release ${PREV} (previous: ${CUR:-none}). Restart the daemon to serve it."
    ;;

  list)
    CUR="$(link_id current)"
    PREV="$(link_id previous)"
    for id in $(releases_newest_first); do
      MARK=" "
      [[ "${id}" == "${PREV}" ]] && MARK="-"
      [[ "${id}" == "${CUR}" ]] && MARK="*"
      echo "${MARK} ${id} $(cat "${BASE}/releases/${id}/BUILD" 2>/dev/null || true)"
    done
    ;;

  running)
    # Release each daemon executes: its exe resolves through bin/ and current/ to the release file
    REAL_RELEASES="$(cd "${BASE}" && pwd -P)/releases/"
    for pid in $(bash "${HERE}/dbsrt_server_ctl.sh" pids "${BASE}" ${ARG:+"${ARG}"}); do
      EXE="$(readlink "/proc/${pid}/exe" || true)"
      if [[ "${EXE}" == "${REAL_RELEASES}"* ]]; then
        REL="${EXE#"${REAL_RELEASES}"}"
        echo "${REL%%/*}"
      else
        echo "unknown"
      fi
    done
    ;;

  prune)
    [[ "${ARG}" =~ ^[0-9]+$ && "${ARG}" -ge 2 ]] || fail "prune keeps at least 2 releases, got '${ARG}'."
    CUR="$(link_id current)"
    PREV="$(link_id previous)"
    KEPT=0
    for id in $(releases_newest_first); do
      if [[ "${id}" == "${CUR}" || "${id}" == "${PREV}" || "${KEPT}" -lt $((ARG - 2)) ]]; then
        [[ "${id}" == "${CUR}" || "${id}" == "${PREV}" ]] || KEPT=$((KEPT + 1))
        continue
      fi
      rm -rf "${BASE:?}/releases/${id}"
      echo "-> Pruned release ${id}."
    done
    ;;

  *)
    echo "usage: $0 <activate|rollback|list|running|prune> [base_dir] [id|socket|keep]" >&2
    exit 2
    ;;
esac
