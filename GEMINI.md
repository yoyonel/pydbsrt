# pydbsrt Project Rules

## Fact-Checking & Zero Speculation
- **Zero Speculation & Fact-First Rigor (Anti-Hallucination)**: STRICTLY FORBIDDEN from presenting theoretical behaviors, mocks, or untested integrations as proven facts or working production features.
- **Explicit Classification**: Always explicitly distinguish:
  1. **Empirically Proven**: Executed and verified live on actual video/data/external APIs with terminal proof.
  2. **Hermetically Mocked**: Tested in unit/mock isolation without network or external service verification.
  3. **Hypothetical / Untested**: Requiring external API keys, third-party services, or unverified protocols.
- **Fail Fast & Transparent**: Immediately announce when a live API or dependency fails (e.g. HTTP 403, missing credentials, rate limits) instead of assuming nominal behavior.
- **Zero Extrapolation (Terminal Proof Mandatory)**: STRICTLY FORBIDDEN from deducing that an end-to-end integration (e.g. VLC playing a video and displaying subtitles) works simply because an underlying library, mock, or sub-component works. If the exact end-to-end command was NOT executed in the current terminal session with its stdout/stderr verified, it is considered **UNTESTED**. Never describe user-facing outcomes or timings of unexecuted workflows as facts.

## Git & Workflow Rules
- **Human Approval Before Commit/Push**: STRICTLY FORBIDDEN from executing `git commit` or `git push` automatically. Always present summary and proposed commit message, and wait for explicit human validation.
- **Zero Diffs in Chat Responses**: STRICTLY FORBIDDEN from printing code diffs or `git diff` in chat responses (the user uses Git and IDE to inspect diffs). Provide concise summaries, modified file links, and proposed commit message only.
- **Taskfile-First Execution**: All recurring commands, builds, tests, demos, benchmarks must be in `Taskfile.yml` and invoked via `task <name>`.
- **Remote Branches Ceiling (Max 3 Active Dev Branches)**: At most 3 active remote development branches may exist simultaneously on GitHub in addition to `master` and `develop`.
- **Strict Issue ↔ Pull Request ↔ Branch Coupling**:
  - Every dev branch MUST follow `<type>/<issue-id>-<slug>` (e.g. `feat/22-unified-ingestion`).
  - Every dev branch MUST be linked to an open Pull Request targeting `develop`.
  - Every Pull Request MUST be explicitly tied to a tracked GitHub Issue / Ticket (`Closes #...` or `Relates to #...`).
  - Issues and Tickets MUST be updated continuously if the technical scope or implementation plan evolves during development.
- **Exhaustive GitHub Issue & Ticket Governance (Mandatory Metadata & Taxonomy)**:
  - Every created or updated GitHub Issue and Pull Request MUST have its metadata completely and accurately filled:
    1. **Assignee**: Systematically assign the author/owner (`@me`).
    2. **Labels**: Systematically attach relevant labels covering scope (`scope:<area>`), nature (`refactor`, `bug`, `enhancement`), language (`rust`, `python`, `shell`), architecture (`arch:soc`, `tech-debt`) and quality (`quality:zero-regression`).
    3. **Label Creation on Demand**: If existing repository labels do not precisely encompass the issue scope, the assistant/developer MUST proactively create new, consistent GitHub labels (e.g. `scope:portal`, `scope:workers`, `scope:ingestion`, `scope:tooling`, `scope:infra`, `scope:tui`, `python`, `shell`, `tech-debt`).
    4. **Project**: Systematically link the issue to the active GitHub Project roadmap (e.g. `pydbsrt — Roadmap & Backlog v0.2+`).
    5. **Milestone**: Attach to the active milestone when defined.
- **3-Day Inactivity Boundary**: Any active dev branch MUST have commits within the last 3 days. If inactive for > 3 days, it must either be completed/merged or archived (via `archive/<date>/<name>` tag) and deleted from remotes.
- **Local Dev Session & Worktree Governance**:
  - **Session Start Alignment & Daily Briefing**:
    - At the start of every AGY session (first user prompt), the assistant MUST automatically run a brief repository health check (`git status`, active branches, latest commits, pending open PRs/Issues).
    - Assistant MUST systematically deliver a concise **Daily Briefing**:
      1. Summary of recent work done (last commits on active dev branches / `develop`).
      2. Status of active branches vs. 3-day inactivity threshold and 3-branch ceiling.
      3. Actionable proposals for next steps calibrated for the session (1 hour, half-day, or full day roadmap).
    - Explicitly confirm or switch to the target branch/issue before touching any code.
  - **Concurrent Session Isolation**: If an AGY session is already running on the repository, developers/agents MUST spawn an isolated Git worktree for the specific branch/task.
  - **Mandatory Git Worktree Data Seeding**: Whenever branching into or running from an isolated Git worktree, MUST systematically repatriate/symlink all unversioned binary datasets and test fixtures from the parent repository (`data/catalog.dbphash`, `data/snippets/`, `data/__PHASH__/`, `data/reports/batch_*.jsonl`, `data/SRT/couplings.json`) BEFORE starting dev daemons or running benchmarks, preventing zero-data illusions and cold start failures.
  - **Worktree Post-Merge Cleanup**: Worktrees must be cleanly removed (`git worktree remove`) immediately upon PR merge.
## Continuous Documentation, Diátaxis & README Synchronization
- **Mandatory Diátaxis 4-Quadrant Architecture**: All documentation in `docs/` MUST strictly comply with the Diátaxis framework (https://diataxis.fr/):
  1. **🎓 Tutorials (`docs/tutorials/`)**: Learning-oriented step-by-step lessons for beginners (linear, zero alternatives, guaranteed success).
  2. **🛠️ How-To Guides (`docs/how-to/`)**: Problem-oriented recipes for practitioners solving real-world tasks.
  3. **📖 Reference (`docs/reference/`)**: Information-oriented technical descriptions (CLI, REST API, SQL schema, binary formats, Taskfile, config).
  4. **💡 Explanation (`docs/explanation/`)**: Understanding-oriented discussions of architecture, algorithms, and design tradeoffs.
- **Incremental Documentation Coverage**: Any addition, refactoring, or evolution of user-facing features, CLI subcommands, REST endpoints, or database tables MUST simultaneously update the relevant Diátaxis quadrant (`docs/reference/` for specs, `docs/how-to/` for operational tasks) in the exact same PR/commit.
- **Automated Validation Gate**: All documentation MUST pass `task doc:check` (strict build, 0 warnings, zero unrendered LaTeX markers, zero local absolute paths, valid HTML AST).
- **README & Taskfile Synchronization**: The root `README.md` and technical documentation MUST ALWAYS remain 100% synchronized with the actual codebase state, available CLI commands, and Taskfile targets.
- **Pure Taskfile Orchestrator (Zero Inline Logic / Hardcoded Data Lists)**: `Taskfile.yml` is strictly a declarative task orchestrator, NEVER a storage dump for inline shell/Python scripts, PID management blocks, loops, or hardcoded lists of media files. Any operational logic, daemon control, or dataset discovery/seeding MUST be placed in dedicated external scripts under `scripts/` so `Taskfile.yml` remains concise, readable, and maintainable.
- **Zero Local Absolute Paths**: Never write machine-local absolute paths or `file://` URIs in any versioned file.

## Strict TUI Views & Contextual Help Synchronization
- **Mandatory 1:1 Parity between TUI Views & Contextual Help (`HelpTab`)**: Any addition, modification, renaming, or refactoring of columns, badges, enums, status values, operational actions, or keyboard shortcuts across the native Rust TUI (`dbsrt top` in `crates/dbsrt-cli`) MUST simultaneously update the corresponding contextual Help view (`render_help_page` / `HelpTab` / `build_help_*`) in the exact same commit.
- **Exhaustive Domain Glossary**: The Help view MUST provide an unambiguous, technical definition for every single column header and every possible status value (e.g. `CompletedStatus`, `IngestionStatus`), explicitly detailing the operational severity level (Nominal, Cache, Functional Warning, Operator Cancel, Fatal Error).
- **Zero Drift Tolerance**: Unit and integration tests (`tests/test_top.rs`) MUST assert the presence and alignment of all key domain terms between the TUI display buffers and the Help screen contents to mathematically prevent any documentation drift.

## Architectural Reality: Subtitle Retargeting & Ground Truth Couplings
- **NO Magical Resync of Random Subtitles**: `pydbsrt` CANNOT magically resynchronize random, uncoupled subtitles (from OpenSubtitles or elsewhere) against a video using pHash alone. Perceptual hashing (`pHash`) operates strictly between TWO video streams: a reference video (whose timeline and subtitle alignment are certified) and a target video. A subtitle file alone contains zero visual fingerprints.
- **Current Reality of Remote Providers (Axe 5)**:
  - If a subtitle is downloaded from OpenSubtitles and our database does NOT contain certified reference pHashes for that release, the computed visual shift is **0 ms**. The subtitle is injected as-is (pure modern VLSub replacement).
  - Visual recalage by pHash is ONLY active if the target video was successfully recognized against a known reference media whose pHash index exists locally or in PostgreSQL.
- **Relational Certified Couplings (`medias` ↔ `frames` ↔ `subtitles` ↔ `sub_frames`)**:
  - Certified ground-truth couplings are stored both in PostgreSQL 17 (`subtitles` + `sub_frames` linked to `medias` and `frames`) and in the Zero-DB registry `data/SRT/couplings.json` via `dbsrt certify-coupling` / `task coupling:certify`.
  - Only when this ground-truth coupling `(Certified Subtitles <-> Video Frames / pHash)` is established can that subtitle serve as reference to automatically retarget other releases via pHash consensus.

## Strict Rust Architectural Complexity & Function Size Gate
- **Ceiling Thresholds (Hard Limits)**:
  - **Maximum Function Size**: Max **150 LOC** (target $\le$ 100 LOC). Any new or modified function exceeding 150 LOC is rejected by CI/CD.
  - **Maximum Cyclomatic Complexity (CCN)**: Max **20** (target $\le$ 10-15).
  - **Maximum Function Arity**: Max **4 arguments**. If more parameters are needed, mandate a dedicated `Context` / `Criteria` / `Request` struct.
  - **Maximum Nesting Depth**: Max **4 levels**.
- **Ratchet Anti-Regression Mechanism**:
  - Legacy functions currently exceeding these thresholds are grandfathered in `scripts/complexity_baseline.json`.
  - STRICTLY FORBIDDEN for any grandfathered function to increase in LOC, CCN, arity, or nesting depth.
  - Once a grandfathered function is refactored, its baseline entry is ratcheted down (`task complexity -- --ratchet`) or removed, preventing any regression.
- **Automated CI/CD Validation Gate**:
  - Enforced via `scripts/lint_complexity.py` invoked via `task complexity` (and wired into `task lint`, pre-commit hook `rust-complexity-gate`, and `.github/workflows/main.yml`).

## Code Quality & Zero Magic Constants (Strictly Enforced)
- **Zero Dynamic Heap Allocations in Hot-Loop Matching (Arena / Pre-allocated Memory Strict Mandate)**:
  - STRICTLY FORBIDDEN from performing ad-hoc dynamic heap allocations (`Vec::new()`, `Vec::collect()`, `String::clone()`, `Box::new()`, or reallocations) inside the frame-by-frame matching hot-loops.
  - All tracking buffers, candidate indexes, and hypothesis pools MUST be pre-allocated once (via reusable workspace scratchpads, Arenas, or fixed capacity buffers with `clear()`).
  - Search complexity in matching hot-loops MUST be sub-quadratic: strict ban on $\mathcal{O}(H \times C)$ nested full-scans; mandate indexed lookups $\mathcal{O}(H + C)$ or direct addressable maps by `media_id`.
- **Zero Inline Language Nesting (Anti-Pattern Ban)**: STRICTLY FORBIDDEN from embedding multi-line inline scripts of one language inside another (e.g. `python -c "..."` or `python - <<'EOF'` heredocs inside shell scripts, inline shell inside Python via string interpolation, or inline Lua/JS strings). Every language runtime MUST reside in its own dedicated, first-class file (`.py`, `.sh`, `.rs`, `.lua`) under `scripts/` or the source tree, fully typed, formatted, and linted by its native toolchain. Shell scripts are strictly orchestrators of native CLI commands, NEVER container shells for inline Python code.

- **STRICTLY FORBIDDEN from using Magic Constants / Numbers / Unnamed Literals**: Hardcoded numeric literals, timeouts, buffer sizes, frame counts, bit depths, and thresholds are strictly forbidden in business logic, HTTP endpoints, parsers, and data pipelines across Rust, Python, and Lua.
- **Mandatory Named & Documented Constants**: Every operational, algorithmic, or temporal value MUST be declared as a typed, named constant (`pub const` in Rust, `UPPER_SNAKE_CASE` in Python/Lua) placed at the top of the relevant module or in dedicated configuration structs with docstrings explaining units and rationales.
  - Examples: `LOG_BUFFER_CAPACITY`, `DEFAULT_EXTRACT_SAMPLE_SECONDS`, `DEFAULT_RELAY_TIMEOUT_MS`, `FINGERPRINT_PREVIEW_SAMPLE_COUNT`, `SECONDS_PER_DAY`, `DEFAULT_BASELINE_FPS`.
- **Zero Inline Conversions**: Temporal arithmetic (e.g. `% 86400`, `/ 3600`, `* 1000.0`, `/ 60.0`) MUST reference standard named factors (`SECONDS_PER_DAY`, `SECONDS_PER_HOUR`, `MILLIS_PER_SEC`, `SECONDS_PER_MINUTE`).
- **Strict Justification Exception**: Standard radix literals in language-native parsing routines (e.g. `u64::from_str_radix(s, 16)` for hex, base-10/2) and structural 0/1 identity boundaries in loops are tolerated ONLY where defining an external constant degrades readability. All other domain values require explicit named constants.

## Mandatory Normalized Rustdoc Markdown Architecture
- **Standardized Docstring Sections (Strictly Enforced)**: Every new, modified, or refactored Rust struct, enum, function, method, and module MUST systematically include standardized Markdown sections (`# Arguments`, `# Returns`, `# Errors`, `# Panics`, `# Data Flow & Invariants`):
  1. **Top Summary & Rationale**: High-level explanation of the component's purpose, design decisions, and architectural role.
  2. **`# Arguments`**: Technical description of every input argument, its units, domain constraints, and boundaries.
  3. **`# Returns`**: Output semantics, edge conditions (e.g. empty inputs, zero matches), and default value behavior.
  4. **`# Errors`**: Exhaustive conditions causing `Err(E)` returns when the function returns `Result`.
  5. **`# Panics`**: Explicit statement of panic absence or edge cases that trigger panics.
  6. **`# Data Flow & Invariants`**: Step-by-step trace of how data flows through the algorithm, internal state mutations, thread-safety guarantees, caching strategies, and mathematical invariants.
- **Zero Doc Drift**: Any change to function parameters, return types, or algorithmic behaviors MUST simultaneously update its corresponding Rustdoc section in the exact same commit.
- **Safe Doctests**: Code blocks in Rustdoc that depend on internal state, private APIs, or mocks MUST use ````rust,ignore` or ````rust,no_run` so `cargo test --doc` passes with zero failures.

## Communication
- **Caveman Mode**: Always terse, ultra-compressed, 100% technical accuracy, zero filler words.
- **Language**: Always respond in French (Daily Briefing, analyses, propositions, summaries). Technical terms, commit messages, and code comments remain in standard English.

## Video Demonstrations & Media Formats
- **STRICTLY FORBIDDEN from using GIF format**: Never generate, store, or embed `.gif` files for video records or UI/UX demos (destructive 256-color palette, heavy bloat, horrible dithering).
- **Mandatory MP4 / WebM with HTML5 Video**: Always record high-definition H.264/MP4 or WebM (24-bit color, 1080p, lightweight compression). Embed in documentation/reports via HTML5 `<video autoplay loop muted playsinline width="100%"><source src="..." type="video/mp4"></video>`.
- **Visually Verifiable Subtitle Rendering in Demos**: Any screenshot or video recording demonstrating subtitle loading/synchronization MUST capture a timestamp window where the subtitle file has active cues (`start_ms <= playback_time <= end_ms`) and the rendered subtitle text + OSD are visibly overlaid on the video frame. Before recording, inspect the `.srt` cue timestamps first and seek the player (e.g. `--start-time`) to a dense cue window — never record silent/cue-less segments where no subtitle appears on screen, and never alter reference `.srt` test fixtures just to force text at `t=0s`.

## Multi-Platform Deployment & Recette Optimization (o2switch & RPi4)
- **Zero-Unnecessary-Rebuild Rule**: Before triggering slow cross-compilations (`task o2switch:deploy`, `task rpi4:server:deploy`), verify if remote daemon already runs an up-to-date commit and endpoints respond nominally. Never rebuild blindly.
- **Tiger Protect WAF Boundary on o2switch**: Cloud host `2026-pydbsrt.fr` enforces Tiger Protect WAF (anti-bot JavaScript challenges, HTTP 429/503 for non-interactive user agents). Automated E2E test suites on o2switch MUST run via authenticated SSH (`task o2switch:e2e`), never raw unauthenticated external headless Playwright directly hitting the domain.
- **Zero-Friction Dev Authentication**: Standalone daemon instances (Local & RPi4) default to unauthenticated mode (zero API key). Automated scripts (`e2e_portal_playwright.py`) MUST pass `--key ""` and bypass login-form navigation when running in local or appliance mode.
- **Event-Driven E2E Assertions (Zero Fixed Sleeping)**: Strictly ban `page.wait_for_timeout(...)` above 300 ms in Playwright suites. Replace with locator assertions (`page.wait_for_selector`, `expect(loc).to_be_visible()`) and parallel batch execution (`Promise.all([fetch(...), ...])`) for instantaneous execution.

## Long-Term Maintainability, Understandability & Reproducibility (Strictly Enforced)
- **Zero Ephemeral Knowledge**: Systematically design all algorithms, data pipelines, scoring models, benchmarks, and CLI tools with a multi-year horizon: "In 1 week, 1 month, 1 year, can any engineer or AI understand the logic, rerun the exact scripts, and unambiguously re-interpret the results without guessing?"
- **Self-Documenting Scoring & Audit Trails**: Every benchmark, evaluation report, and algorithmic scoring output MUST persist full decomposition (inputs, intermediate window observations, mathematical factors, decision invariants, thresholds, and execution conditions). Condensed or opaque summary metrics (e.g. single aggregate percentage) are strictly forbidden without their complete underlying breakdown.
- **Parametric Extensibility**: Benchmark and scoring algorithms MUST be decoupled from hardcoded window counts or slice positions, accepting arbitrary slice counts ($N=3, 5, 7, \dots$) and custom temporal ratios via explicit configuration.
- **Mandatory Presentation of Difficulties & Deductive Technical Choices**: After completing any non-trivial implementation, benchmark, or algorithm step, the assistant MUST systematically and proactively present to the user: (1) every concrete technical difficulty encountered during execution, and (2) every explicit assumption, fallback, or default heuristic chosen due to incomplete specification/information, allowing the user to arbitrate architectural decisions without hidden biases.
