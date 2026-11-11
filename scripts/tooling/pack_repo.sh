#!/usr/bin/env bash
# Pack entire repository source code into an AI-friendly markdown bundle using repomix
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

# Print usage if requested
if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "Usage: $(basename "$0") [--check | --dry-run | <output_file>] [--help]"
  echo ""
  echo "Packs full repository source code into an AI-friendly markdown bundle via repomix."
  echo ""
  echo "Options:"
  echo "  --check, --dry-run  List files that would be packed without running repomix"
  echo "  --help, -h          Show this help message and exit"
  exit 0
fi

BRANCH="$(git -C "${REPO_ROOT}" rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'unknown')"
COMMIT="$(git -C "${REPO_ROOT}" rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

FILES_TO_PACK="$(git -C "${REPO_ROOT}" ls-files |
  grep -E '\.(rs|py|sh|lua|toml|ya?ml)$' |
  grep -v -E '^(Cargo\.lock|data/|\.tmp/|target/|mkdocs\.yml|docs/)' |
  sort -u)"

if [[ -z "${FILES_TO_PACK}" ]]; then
  echo "⚠️ No source files found to pack." >&2
  exit 1
fi

TOTAL_FILES="$(echo "${FILES_TO_PACK}" | wc -l)"

if [[ "${1:-}" == "--check" || "${1:-}" == "--dry-run" ]]; then
  echo "=== DBSrt Pack Repo Inspection ==="
  echo "✓ REPO_ROOT     : ${REPO_ROOT}"
  echo "✓ Branch        : ${BRANCH}"
  echo "✓ Commit        : ${COMMIT}"
  echo "✓ Files to pack : ${TOTAL_FILES}"
  echo "✓ npx tool      : $(command -v npx >/dev/null 2>&1 && echo "installed" || echo "missing")"
  exit 0
fi

DEFAULT_OUTPUT="${REPO_ROOT}/repomix-repo-full.md"
OUTPUT_FILE="${1:-${DEFAULT_OUTPUT}}"
TMP_HEADER="$(mktemp /tmp/repo-header-XXXXXX.md)"
TMP_REPOMIX="$(mktemp /tmp/repomix-XXXXXX.md)"

trap 'rm -f "${TMP_HEADER}" "${TMP_REPOMIX}"' EXIT

echo "Generating full repository code bundle for branch '${BRANCH}' (${COMMIT})..."

cat <<EOF >"${TMP_HEADER}"
# Full Repository Source Code Bundle: pydbsrt

- **Branch**: \`${BRANCH}\`
- **Commit**: \`${COMMIT}\`
- **Generated At**: \`${TIMESTAMP}\`
- **Total Source Files**: ${TOTAL_FILES}

## Included Source Categories
- Rust Crates: \`crates/dbsrt-core\`, \`crates/dbsrt-index\`, \`crates/dbsrt-cli\`, \`crates/dbsrt-server\`
- Python Services & CLI: \`src/pydbsrt/\`, \`tests/\`
- VLC Lua Plugins: \`vlc-plugin/\`
- Operational Scripts & Tooling: \`scripts/\`
- Build Configurations: \`Cargo.toml\`, \`Taskfile.yml\`, \`taskfiles/*.yml\`

---

EOF

# Pack source files via repomix from repository root
(
  cd "${REPO_ROOT}"
  echo "${FILES_TO_PACK}" | npx repomix --stdin --style markdown --no-security-check -o "${TMP_REPOMIX}"
)

# Combine header and repomix output
cat "${TMP_HEADER}" "${TMP_REPOMIX}" >"${OUTPUT_FILE}"

echo "✓ Full repository bundle successfully generated at: ${OUTPUT_FILE}"
echo "  Total source files: ${TOTAL_FILES}"
echo "  Total lines: $(wc -l <"${OUTPUT_FILE}")"
echo "  Total size : $(du -h "${OUTPUT_FILE}" | cut -f1)"
