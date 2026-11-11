#!/usr/bin/env bash
# Pack branch diffs and modified files into an AI-friendly markdown bundle using repomix
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/../.." && pwd)"
if [[ ! -d "${REPO_ROOT}/.git" && -d "${HOME}/dbsrt" ]]; then
  REPO_ROOT="${HOME}/dbsrt"
fi

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
  echo "Usage: $(basename "$0") [--check | --dry-run | <target_branch>] [<output_file>] [--help]"
  echo ""
  echo "Packs branch diffs and modified files into an AI-friendly markdown bundle via repomix."
  echo ""
  echo "Arguments:"
  echo "  target_branch       Base branch to compare against (default: origin/develop)"
  echo "  output_file         Output markdown path (default: \${REPO_ROOT}/repomix-branch-review.md)"
  echo ""
  echo "Options:"
  echo "  --check, --dry-run  Inspect modified files and diff stat without generating bundle"
  echo "  --help, -h          Show this help message and exit"
  exit 0
fi

IS_CHECK=0
if [[ "${1:-}" == "--check" || "${1:-}" == "--dry-run" ]]; then
  IS_CHECK=1
  TARGET_BRANCH="origin/develop"
  DEFAULT_OUTPUT="${REPO_ROOT}/repomix-branch-review.md"
  OUTPUT_FILE="${DEFAULT_OUTPUT}"
else
  TARGET_BRANCH="${1:-origin/develop}"
  DEFAULT_OUTPUT="${REPO_ROOT}/repomix-branch-review.md"
  OUTPUT_FILE="${2:-${DEFAULT_OUTPUT}}"
fi

# Ensure TARGET_BRANCH exists or fallback to HEAD~1
if ! git -C "${REPO_ROOT}" rev-parse --verify "${TARGET_BRANCH}" >/dev/null 2>&1; then
  if git -C "${REPO_ROOT}" rev-parse --verify "develop" >/dev/null 2>&1; then
    TARGET_BRANCH="develop"
  elif git -C "${REPO_ROOT}" rev-parse --verify "master" >/dev/null 2>&1; then
    TARGET_BRANCH="master"
  else
    TARGET_BRANCH="HEAD~1"
  fi
fi

# Collect all files to pack: committed differences + working tree changes + untracked files
COMMITTED_FILES="$(git -C "${REPO_ROOT}" diff --name-only "${TARGET_BRANCH}...HEAD" 2>/dev/null || true)"
WORKTREE_FILES="$(git -C "${REPO_ROOT}" diff --name-only "${TARGET_BRANCH}" 2>/dev/null || true)"
UNTRACKED_FILES="$(git -C "${REPO_ROOT}" ls-files --others --exclude-standard 2>/dev/null || true)"

# Filter strictly to source implementations (Rust, Python, Shell, Lua, Taskfile/Cargo configs)
FILES_TO_PACK="$(printf "%s\n%s\n%s" "${COMMITTED_FILES}" "${WORKTREE_FILES}" "${UNTRACKED_FILES}" |
  sed '/^$/d' |
  grep -E '\.(rs|py|sh|lua|toml|ya?ml)$' |
  grep -v -E '^(Cargo\.lock|data/|\.tmp/|target/|mkdocs\.yml)' |
  sort -u)"

if [[ -z "${FILES_TO_PACK}" ]]; then
  echo "⚠️ No modified or untracked source files found compared to ${TARGET_BRANCH}." >&2
  echo "💡 Tip: If this branch was already merged, specify the base commit (e.g. 'task review:pack -- HEAD~1')." >&2
  exit 1
fi

TOTAL_FILES="$(echo "${FILES_TO_PACK}" | wc -l)"

if [[ "${IS_CHECK}" -eq 1 ]]; then
  echo "=== DBSrt Pack Review Inspection ==="
  echo "✓ REPO_ROOT     : ${REPO_ROOT}"
  echo "✓ Target Base   : ${TARGET_BRANCH}"
  echo "✓ Files to pack : ${TOTAL_FILES}"
  echo "✓ Output target : ${OUTPUT_FILE}"
  echo "✓ npx tool      : $(command -v npx >/dev/null 2>&1 && echo "installed" || echo "missing")"
  exit 0
fi

TMP_HEADER="$(mktemp /tmp/review-header-XXXXXX.md)"
TMP_REPOMIX="$(mktemp /tmp/repomix-XXXXXX.md)"

trap 'rm -f "${TMP_HEADER}" "${TMP_REPOMIX}"' EXIT

echo "Generating code review bundle comparing HEAD with ${TARGET_BRANCH}..."

mapfile -t SOURCE_FILES_ARRAY <<<"${FILES_TO_PACK}"

COMMITS_LOG="$(git -C "${REPO_ROOT}" log --oneline "${TARGET_BRANCH}...HEAD" 2>/dev/null || true)"
if [[ -z "${COMMITS_LOG}" ]]; then
  COMMITS_LOG="(Working tree uncommitted changes against ${TARGET_BRANCH})"
fi

DIFF_STAT="$(git -C "${REPO_ROOT}" diff --stat "${TARGET_BRANCH}" -- "${SOURCE_FILES_ARRAY[@]}" 2>/dev/null || true)"
UNIFIED_DIFF="$(git -C "${REPO_ROOT}" diff "${TARGET_BRANCH}" -- "${SOURCE_FILES_ARRAY[@]}" 2>/dev/null || true)"

cat <<EOF >"${TMP_HEADER}"
# Code Review Bundle: ${TARGET_BRANCH}...HEAD

## Review Context & Strategic Objectives
You are conducting an in-depth senior architectural, design, and code review of this pull request branch compared to \`${TARGET_BRANCH}\`.

### Architectural Scope
1. **Separation of Concerns (SoC) & Modular Architecture**:
   - Decoupling monolithic CLI dispatch into domain-specific subcommand modules under \`crates/dbsrt-cli/src/commands/\`.
   - Exposing a reusable, in-process library API (\`crates/dbsrt-cli/src/lib.rs\`) allowing fine-grained unit testing without binary subprocess spawning.
   - Clean layer boundaries between \`dbsrt-cli\` (CLI interface), \`dbsrt-core\` (algorithmic video decoding, pHash computation, providers), \`dbsrt-index\` (BK-Tree, MIH in-memory index, PostgreSQL 17 SP-GiST, Zero-DB mmap), and \`dbsrt-server\` (Axum HTTP/REST daemon sidecar).
2. **Data Flow & Subtitle Retargeting Engine**:
   - Correctness of ground-truth certified couplings \`(Media/pHash Frames <-> Subtitles)\`.
   - Idempotent ingestion pipelines (PostgreSQL COPY / transaction lifecycle / deduplication).
3. **Concurrency, Memory Management & Performance**:
   - Tokio async runtime vs Rayon parallel CPU-bound workloads.
   - Zero-copy mmap and buffer allocation strategies.
   - Temporary file lifecycle, RAII cleanup, and connection pooling.
4. **Resilience & Fault Tolerance**:
   - Fail-fast validation (early argument/file checks before acquiring heavy I/O or network resources).
   - Graceful degradation when PostgreSQL or remote providers are offline.

### Strict Repository Standards
- **Zero Magic Constants**: All numbers, thresholds, timeouts, ports, and dimensions must be named, typed, and documented constants.
- **Pure Taskfile Orchestrator**: Declarative targets only, zero inline logic.
- **Fact-First Rigor & Anti-Hallucination**: Clear distinction between Empirically Proven, Hermetically Mocked, and Hypothetical/Untested.

## Commits in this Branch
\`\`\`text
${COMMITS_LOG}
\`\`\`

## Diff Stat
\`\`\`text
${DIFF_STAT}
\`\`\`

## Unified Git Diff
\`\`\`diff
${UNIFIED_DIFF}
\`\`\`

---

# Modified Files Full Source Code (Packed via Repomix)

EOF

# Pack modified source files via repomix
(
  cd "${REPO_ROOT}"
  echo "${FILES_TO_PACK}" | npx repomix --stdin --style markdown --no-security-check -o "${TMP_REPOMIX}"
)

# Combine header and repomix output
cat "${TMP_HEADER}" "${TMP_REPOMIX}" >"${OUTPUT_FILE}"

echo "✓ Code review bundle successfully generated at: ${OUTPUT_FILE}"
echo "  Total lines: $(wc -l <"${OUTPUT_FILE}")"
echo "  Total size : $(du -h "${OUTPUT_FILE}" | cut -f1)"
