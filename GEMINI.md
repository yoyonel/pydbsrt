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

## Architectural Reality: Subtitle Retargeting & Ground Truth Couplings
- **NO Magical Resync of Random Subtitles**: `pydbsrt` CANNOT magically resynchronize random, uncoupled subtitles (from OpenSubtitles or elsewhere) against a video using pHash alone. Perceptual hashing (`pHash`) operates strictly between TWO video streams: a reference video (whose timeline and subtitle alignment are certified) and a target video. A subtitle file alone contains zero visual fingerprints.
- **Current Reality of Remote Providers (Axe 5)**:
  - If a subtitle is downloaded from OpenSubtitles and our database does NOT contain certified reference pHashes for that release, the computed visual shift is **0 ms**. The subtitle is injected as-is (pure modern VLSub replacement).
  - Visual recalage by pHash is ONLY active if the target video was successfully recognized against a known reference media whose pHash index exists locally or in PostgreSQL.
- **Relational Certified Couplings (`medias` ↔ `frames` ↔ `subtitles` ↔ `sub_frames`)**:
  - Certified ground-truth couplings are stored both in PostgreSQL 17 (`subtitles` + `sub_frames` linked to `medias` and `frames`) and in the Zero-DB registry `data/SRT/couplings.json` via `dbsrt certify-coupling` / `task coupling:certify`.
  - Only when this ground-truth coupling `(Certified Subtitles <-> Video Frames / pHash)` is established can that subtitle serve as reference to automatically retarget other releases via pHash consensus.

## Code Quality & Zero Magic Constants (Strictly Enforced)
- **Zero Inline Language Nesting (Anti-Pattern Ban)**: STRICTLY FORBIDDEN from embedding multi-line inline scripts of one language inside another (e.g. `python -c "..."` or `python - <<'EOF'` heredocs inside shell scripts, inline shell inside Python via string interpolation, or inline Lua/JS strings). Every language runtime MUST reside in its own dedicated, first-class file (`.py`, `.sh`, `.rs`, `.lua`) under `scripts/` or the source tree, fully typed, formatted, and linted by its native toolchain. Shell scripts are strictly orchestrators of native CLI commands, NEVER container shells for inline Python code.
- **STRICTLY FORBIDDEN from using Magic Constants / Numbers / Unnamed Literals**: Hardcoded numeric literals, timeouts, buffer sizes, frame counts, bit depths, and thresholds are strictly forbidden in business logic, HTTP endpoints, parsers, and data pipelines across Rust, Python, and Lua.
- **Mandatory Named & Documented Constants**: Every operational, algorithmic, or temporal value MUST be declared as a typed, named constant (`pub const` in Rust, `UPPER_SNAKE_CASE` in Python/Lua) placed at the top of the relevant module or in dedicated configuration structs with docstrings explaining units and rationales.
  - Examples: `LOG_BUFFER_CAPACITY`, `DEFAULT_EXTRACT_SAMPLE_SECONDS`, `DEFAULT_RELAY_TIMEOUT_MS`, `FINGERPRINT_PREVIEW_SAMPLE_COUNT`, `SECONDS_PER_DAY`, `DEFAULT_BASELINE_FPS`.
- **Zero Inline Conversions**: Temporal arithmetic (e.g. `% 86400`, `/ 3600`, `* 1000.0`, `/ 60.0`) MUST reference standard named factors (`SECONDS_PER_DAY`, `SECONDS_PER_HOUR`, `MILLIS_PER_SEC`, `SECONDS_PER_MINUTE`).
- **Strict Justification Exception**: Standard radix literals in language-native parsing routines (e.g. `u64::from_str_radix(s, 16)` for hex, base-10/2) and structural 0/1 identity boundaries in loops are tolerated ONLY where defining an external constant degrades readability. All other domain values require explicit named constants.

## Communication
- **Caveman Mode**: Always terse, ultra-compressed, 100% technical accuracy, zero filler words.

## Video Demonstrations & Media Formats
- **STRICTLY FORBIDDEN from using GIF format**: Never generate, store, or embed `.gif` files for video records or UI/UX demos (destructive 256-color palette, heavy bloat, horrible dithering).
- **Mandatory MP4 / WebM with HTML5 Video**: Always record high-definition H.264/MP4 or WebM (24-bit color, 1080p, lightweight compression). Embed in documentation/reports via HTML5 `<video autoplay loop muted playsinline width="100%"><source src="..." type="video/mp4"></video>`.
- **Visually Verifiable Subtitle Rendering in Demos**: Any screenshot or video recording demonstrating subtitle loading/synchronization MUST capture a timestamp window where the subtitle file has active cues (`start_ms <= playback_time <= end_ms`) and the rendered subtitle text + OSD are visibly overlaid on the video frame. Before recording, inspect the `.srt` cue timestamps first and seek the player (e.g. `--start-time`) to a dense cue window — never record silent/cue-less segments where no subtitle appears on screen, and never alter reference `.srt` test fixtures just to force text at `t=0s`.
