#!/usr/bin/env bash
# Pack entire repository source code into an AI-friendly markdown bundle using repomix
set -euo pipefail

OUTPUT_FILE="${1:-repomix-repo-full.md}"
TMP_HEADER="$(mktemp /tmp/repo-header-XXXXXX.md)"
TMP_REPOMIX="$(mktemp /tmp/repomix-XXXXXX.md)"

trap 'rm -f "${TMP_HEADER}" "${TMP_REPOMIX}"' EXIT

BRANCH="$(git rev-parse --abbrev-ref HEAD 2>/dev/null || echo 'unknown')"
COMMIT="$(git rev-parse --short HEAD 2>/dev/null || echo 'unknown')"
TIMESTAMP="$(date -u +"%Y-%m-%dT%H:%M:%SZ")"

echo "Generating full repository code bundle for branch '${BRANCH}' (${COMMIT})..."

FILES_TO_PACK="$(git ls-files |
  grep -E '\.(rs|py|sh|lua|toml|ya?ml)$' |
  grep -v -E '^(Cargo\.lock|data/|\.tmp/|target/|mkdocs\.yml|docs/)' |
  sort -u)"

if [[ -z "${FILES_TO_PACK}" ]]; then
  echo "⚠️ No source files found to pack." >&2
  exit 1
fi

TOTAL_FILES="$(echo "${FILES_TO_PACK}" | wc -l)"

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

# Pack source files via repomix
echo "${FILES_TO_PACK}" | npx repomix --stdin --style markdown --no-security-check -o "${TMP_REPOMIX}"

# Combine header and repomix output
cat "${TMP_HEADER}" "${TMP_REPOMIX}" >"${OUTPUT_FILE}"

echo "✓ Full repository bundle successfully generated at: ${OUTPUT_FILE}"
echo "  Total source files: ${TOTAL_FILES}"
echo "  Total lines: $(wc -l <"${OUTPUT_FILE}")"
echo "  Total size : $(du -h "${OUTPUT_FILE}" | cut -f1)"
