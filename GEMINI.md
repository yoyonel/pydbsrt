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
- **Taskfile-First Execution**: All recurring commands, builds, tests, demos, benchmarks must be in `Taskfile.yml` and invoked via `task <name>`.
- **Zero Local Absolute Paths**: Never write machine-local absolute paths or `file://` URIs in any versioned file.

## Architectural Reality: Subtitle Retargeting & Ground Truth Couplings
- **NO Magical Resync of Random Subtitles**: `pydbsrt` CANNOT magically resynchronize random, uncoupled subtitles (from OpenSubtitles or elsewhere) against a video using pHash alone. Perceptual hashing (`pHash`) operates strictly between TWO video streams: a reference video (whose timeline and subtitle alignment are certified) and a target video. A subtitle file alone contains zero visual fingerprints.
- **Current Reality of Remote Providers (Axe 5)**:
  - If a subtitle is downloaded from OpenSubtitles and our database does NOT contain certified reference pHashes for that release, the computed visual shift is **0 ms**. The subtitle is injected as-is (pure modern VLSub replacement).
  - Visual recalage by pHash is ONLY active if the target video was successfully recognized against a known reference media whose pHash index exists locally or in PostgreSQL.
- **Future Roadmap: Certified Couplings & Validators**:
  - Long-term goal: build subtitle validation pipelines confirming with 100% certainty (human annotations in phase 1) that a subtitle is precisely timed for a given video/media release.
  - Only when this ground-truth coupling `(Certified Subtitles <-> Video Frames / pHash)` is established can that subtitle serve as reference to automatically retarget other releases via pHash consensus.

## Communication
- **Caveman Mode**: Always terse, ultra-compressed, 100% technical accuracy, zero filler words.

## Video Demonstrations & Media Formats
- **STRICTLY FORBIDDEN from using GIF format**: Never generate, store, or embed `.gif` files for video records or UI/UX demos (destructive 256-color palette, heavy bloat, horrible dithering).
- **Mandatory MP4 / WebM with HTML5 Video**: Always record high-definition H.264/MP4 or WebM (24-bit color, 1080p, lightweight compression). Embed in documentation/reports via HTML5 `<video autoplay loop muted playsinline width="100%"><source src="..." type="video/mp4"></video>`.
- **Meaningful Dialogue Timestamps**: When demoing subtitle sync on real media, never record idle intro/logo sequences. Seek directly to active dialogue timestamps (e.g. `--start-time`) so that spoken subtitles and OSD are clearly visible on screen.
