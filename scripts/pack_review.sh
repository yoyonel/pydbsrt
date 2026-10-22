#!/usr/bin/env bash
# Pack branch diffs and modified files into an AI-friendly markdown bundle using repomix
set -euo pipefail

TARGET_BRANCH="${1:-origin/develop}"
OUTPUT_FILE="${2:-repomix-branch-review.md}"
TMP_HEADER="$(mktemp /tmp/review-header-XXXXXX.md)"
TMP_REPOMIX="$(mktemp /tmp/repomix-XXXXXX.md)"

trap 'rm -f "${TMP_HEADER}" "${TMP_REPOMIX}"' EXIT

echo "Generating code review bundle comparing HEAD with ${TARGET_BRANCH}..."

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
$(git log --oneline "${TARGET_BRANCH}...HEAD")
\`\`\`

## Diff Stat
\`\`\`text
$(git diff --stat "${TARGET_BRANCH}...HEAD")
\`\`\`

## Unified Git Diff
\`\`\`diff
$(git diff "${TARGET_BRANCH}...HEAD")
\`\`\`

---

# Modified Files Full Source Code (Packed via Repomix)

EOF

# Pack modified files via repomix
git diff --name-only "${TARGET_BRANCH}...HEAD" | npx repomix --stdin --style markdown --no-security-check -o "${TMP_REPOMIX}"

# Combine header and repomix output
cat "${TMP_HEADER}" "${TMP_REPOMIX}" >"${OUTPUT_FILE}"

echo "✓ Code review bundle successfully generated at: ${OUTPUT_FILE}"
echo "  Total lines: $(wc -l <"${OUTPUT_FILE}")"
echo "  Total size : $(du -h "${OUTPUT_FILE}" | cut -f1)"
