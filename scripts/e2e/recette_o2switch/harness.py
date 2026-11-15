# pyright: reportMissingImports=false
"""Core harness, coverage trackers, video encoder, and context for o2switch E2E recette."""

from __future__ import annotations

import json
import os
import re
import shutil
import subprocess
import time
from dataclasses import asdict, dataclass, field
from pathlib import Path
from typing import TYPE_CHECKING, Any
from urllib.parse import urlparse

if TYPE_CHECKING:
    from playwright.sync_api import Browser, BrowserContext, CDPSession, Page, Response

VIEWPORT_WIDTH: int = 1920
VIEWPORT_HEIGHT: int = 1080
DEFAULT_TIMEOUT_MS: int = 30000
PROBE_TIMEOUT_MS: int = 60000
FFPROBE_TIMEOUT_MS: int = 45000
UI_SETTLE_MS: int = 250
MILLIS_PER_SEC: float = 1000.0
PERCENT_SCALE: float = 100.0

HTTP_OK: int = 200
HTTP_NO_CONTENT: int = 204
HTTP_BAD_REQUEST: int = 400
HTTP_UNAUTHORIZED: int = 401
HTTP_FORBIDDEN: int = 403
HTTP_NOT_FOUND: int = 404
HTTP_NOT_ACCEPTABLE: int = 406

EXPECTED_FREEBOX_CONTAINERS: int = 595
MIN_SYNOLOGY_VIDEO_ITEMS: int = 500
MIN_SYNOLOGY_TVSHOW_ITEMS: int = 50
MIN_PG_ASSETS_TOTAL: int = 2300
MIN_CATALOG_PHASH_COUNT: int = 4974
MIN_ALIGNED_3_3_COUNT: int = 3800
MIN_CERTIFIED_COUPLINGS: int = 2400
MIN_TRAJECTORY_FILES: int = 5

DESKTOP_USER_AGENT: str = (
    "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36"
)


def find_repo_root() -> Path:
    """Locate repository root dynamically without magic parent indices."""
    current = Path(__file__).resolve().parent
    for candidate in [current, *current.parents]:
        if (candidate / ".git").exists() or (candidate / "Taskfile.yml").is_file():
            return candidate
    return Path.cwd()


@dataclass
class StepRecord:
    """Self-documenting audit record for a single E2E verification step."""

    step_id: str
    portal: str
    feature: str
    status: str
    elapsed_ms: float
    assertions: list[str] = field(default_factory=list)
    metrics: dict[str, Any] = field(default_factory=dict)
    screenshot: str | None = None


@dataclass
class EndpointSpec:
    """Declared HTTP endpoint contract tracked during the E2E run."""

    method: str
    pattern: str
    regex: re.Pattern[str]
    portal: str
    description: str
    hits: int = 0
    statuses: list[int] = field(default_factory=list)


DECLARED_ENDPOINTS: tuple[tuple[str, str, str, str], ...] = (
    # Portal A: Security Gateway & Static Routes
    ("OPTIONS", "/v1/health", "Gateway", "CORS preflight 204 handler"),
    ("GET", "/sources", "Sources", "Sources Médias & Observatoire SPA"),
    ("GET", "/sources.html", "Sources", "Sources SPA direct HTML alias"),
    ("GET", "/monitoring", "Monitoring", "Monitoring & Web Top dashboard"),
    ("POST", "/monitoring", "Gateway", "HTML form API key login POST"),
    ("GET", "/studio", "Studio", "DBSrt Studio relational backoffice"),
    ("GET", "/top", "Monitoring", "Web TUI Top alias route"),
    ("GET", "/logout", "Gateway", "Session cookie revocation & redirect"),
    ("GET", "/favicon.ico", "Gateway", "Inline SVG favicon handler"),
    ("GET", "/api/artifact/download", "Gateway", "Zero-spawn binary artifact download"),
    ("HEAD", "/api/artifact/download", "Gateway", "Zero-spawn artifact SHA256 HEAD check"),
    # Portal B: Sources, Assets, pHashes & Trajectories Python/PG APIs
    ("GET", "/api/sources", "Sources", "Host & storage channels metadata"),
    ("GET", "/api/probe", "Sources", "Live channel accessibility probe"),
    ("GET", "/api/files", "Sources", "Paginated container & directory listing"),
    ("GET", "/api/file/details", "Sources", "Live remote ffprobe stream inspection"),
    ("GET", "/api/assets", "Sources", "PostgreSQL media assets audit list"),
    ("GET", "/api/asset/details", "Sources", "Single PostgreSQL asset deep inspection"),
    ("GET", "/api/phashes", "Sources", "Tripartite RAW/Zero-DB/PG reconciliation"),
    ("GET", "/api/phash/details", "Sources", "Microscopic pHash popcount & Δ(t) curve"),
    ("GET", "/api/trajectory/files", "Sources", "Available JSONL trajectory audit reports"),
    ("GET", "/api/trajectories", "Sources", "Paginated spatiotemporal trajectory KPIs & rows"),
    ("GET", "/api/trajectory/details", "Sources", "Per-cut trajectory consensus breakdown"),
    # Portal C & D: Rust dbsrt-server Daemon Routes
    ("GET", "/health", "Daemon", "Root daemon healthcheck"),
    ("GET", "/v1/health", "Daemon", "API v1 healthcheck"),
    ("GET", "/v1/status", "Daemon", "Daemon index & sanitized status summary"),
    ("GET", "/v1/metrics", "Daemon", "Daemon request & latency counters"),
    ("GET", "/v1/logs", "Daemon", "Daemon ring-buffer logs"),
    ("GET", "/v1/monitoring/telemetry", "Monitoring", "Batch, audit & supervisor telemetry"),
    ("GET", "/v1/monitoring/top", "Monitoring", "SWR top process & catalog snapshot"),
    ("GET", "/v1/monitoring/events", "Monitoring", "Persistent Server-Sent Events stream"),
    ("POST", "/v1/monitoring/audit/stop", "Monitoring", "Idempotent catalog audit stop order"),
    ("GET", "/v1/couplings", "Studio", "Certified ground-truth couplings registry"),
    ("GET", "/v1/couplings/sub_frames", "Studio", "Paginated subtitle cues & pHash anchors"),
    ("GET", "/v1/snippets", "Studio", "Available video snippets datalist"),
    ("GET", "/v1/subtitles/{hash}", "Studio", "CAS raw SRT content by digest"),
    ("POST", "/v1/search", "Studio", "Multi-tier pHash/MovieHash/Name search"),
    ("GET", "/v1/search/latest", "Studio", "Latest search 6-stage pipeline telemetry"),
    ("POST", "/v1/recognize", "Studio", "Direct RAM MIH visual recognition"),
    ("POST", "/v1/certify", "Studio", "Ground-truth coupling certification handler"),
    ("POST", "/v1/download", "Studio", "Subtitle retrieval handler"),
    ("POST", "/v1/retarget", "Studio", "Visual pHash subtitle retargeting handler"),
    ("POST", "/v1/autosync", "Studio", "End-to-end AutoSync pipeline handler"),
    ("GET", "/v1/catalog/status", "Daemon", "In-memory .dbphash catalog index status"),
    ("POST", "/v1/catalog/reload", "Daemon", "Atomic hot-reload of .dbphash & couplings"),
)


class EndpointCoverageTracker:
    """Tracks empirical HTTP hits and status codes across all declared routes."""

    def __init__(self) -> None:
        self.specs: list[EndpointSpec] = []
        for method, pattern, portal, desc in DECLARED_ENDPOINTS:
            escaped = re.escape(pattern).replace(r"\{hash\}", r"[0-9a-fA-F]+")
            regex = re.compile(f"^{escaped}$")
            self.specs.append(
                EndpointSpec(
                    method=method.upper(),
                    pattern=pattern,
                    regex=regex,
                    portal=portal,
                    description=desc,
                )
            )

    @staticmethod
    def normalize_path(url: str) -> str:
        """Strip /dbsrt gateway prefix to match canonical route patterns."""
        path = urlparse(url).path
        idx = path.find("/dbsrt")
        norm = path[idx + len("/dbsrt") :] if idx != -1 else path
        if not norm or norm == "/":
            return "/sources"
        return norm

    def record_manual(self, method: str, url: str, status: int) -> None:
        """Record an explicit HTTP check (e.g. OPTIONS or HEAD via Python/fetch)."""
        norm = self.normalize_path(url)
        meth = method.upper()
        for spec in self.specs:
            if spec.method == meth and spec.regex.match(norm):
                spec.hits += 1
                if status not in spec.statuses:
                    spec.statuses.append(status)
                break

    def on_request(self, request: Any) -> None:
        """Playwright request listener."""
        norm = self.normalize_path(request.url)
        meth = request.method.upper()
        for spec in self.specs:
            if spec.method == meth and spec.regex.match(norm):
                spec.hits += 1
                break

    def on_response(self, response: Response) -> None:
        """Playwright response listener."""
        norm = self.normalize_path(response.url)
        meth = response.request.method.upper()
        for spec in self.specs:
            if spec.method == meth and spec.regex.match(norm):
                if response.status not in spec.statuses:
                    spec.statuses.append(response.status)
                break

    def summary(self) -> dict[str, Any]:
        """Produce structured JSON coverage breakdown."""
        total = len(self.specs)
        covered = sum(1 for s in self.specs if s.hits > 0)
        pct = round((covered / total) * PERCENT_SCALE, 2) if total else 0.0
        return {
            "total_declared": total,
            "covered_count": covered,
            "coverage_pct": pct,
            "uncovered": [f"{s.method} {s.pattern}" for s in self.specs if s.hits == 0],
            "endpoints": [
                {
                    "method": s.method,
                    "pattern": s.pattern,
                    "portal": s.portal,
                    "description": s.description,
                    "hits": s.hits,
                    "statuses": sorted(s.statuses),
                    "covered": s.hits > 0,
                }
                for s in self.specs
            ],
        }


class ConsoleGuard:
    """Intercepts browser console errors, Alpine expression failures, and uncaught exceptions."""

    def __init__(self) -> None:
        self.errors: list[str] = []
        self.warnings: list[str] = []
        self.allow_expected_http_errors: bool = False

    def attach(self, page: Page) -> None:
        """Register console and pageerror listeners on a Playwright page."""
        page.on("console", self._on_console)
        page.on("pageerror", self._on_page_error)

    def _on_console(self, msg: Any) -> None:
        text = msg.text or ""
        mtype = msg.type
        is_alpine_bug = (
            "Alpine Expression Error" in text
            or "ReferenceError" in text
            or "TypeError:" in text
            or "SyntaxError:" in text
        )
        if is_alpine_bug:
            self.errors.append(f"[JS/ALPINE] {text}")
            return
        if mtype == "error":
            if self.allow_expected_http_errors and "Failed to load resource" in text:
                self.warnings.append(f"[EXPECTED_HTTP] {text}")
                return
            if "Failed to load resource: the server responded with a status of 401" in text:
                self.warnings.append(f"[AUTH_401] {text}")
                return
            self.errors.append(f"[CONSOLE_ERROR] {text}")
        elif mtype == "warning":
            self.warnings.append(f"[WARN] {text}")

    def _on_page_error(self, exc: Any) -> None:
        self.errors.append(f"[PAGE_ERROR] {exc}")


class V8CoverageSession:
    """Collects Chromium CDP V8 JavaScript coverage across multiple page sessions."""

    def __init__(self) -> None:
        self.raw_scripts: list[dict[str, Any]] = []
        self._active_cdp: CDPSession | None = None

    def start_for_page(self, page: Page) -> None:
        """Start precise V8 coverage on the given page."""
        cdp = page.context.new_cdp_session(page)
        cdp.send("Profiler.enable")
        cdp.send("Profiler.startPreciseCoverage", {"callCount": True, "detailed": True})
        self._active_cdp = cdp

    def harvest_current_page(self) -> None:
        """Harvest V8 coverage before closing the current page/context."""
        if not self._active_cdp:
            return
        cov = self._active_cdp.send("Profiler.takePreciseCoverage")
        self._active_cdp.send("Profiler.stopPreciseCoverage")
        self._active_cdp.send("Profiler.disable")
        self.raw_scripts.extend(cov.get("result", []))
        self._active_cdp = None

    @staticmethod
    def _measure_function_ranges(ranges: list[dict[str, Any]]) -> tuple[int, int, int]:
        """Returns (fn_used_inc, total_span_bytes, used_span_bytes) for one JS function."""
        fn_inc = 1 if any(r.get("count", 0) > 0 for r in ranges) else 0
        t_bytes = 0
        u_bytes = 0
        for rng in ranges:
            span = max(0, rng.get("endOffset", 0) - rng.get("startOffset", 0))
            t_bytes += span
            if rng.get("count", 0) > 0:
                u_bytes += span
        return fn_inc, t_bytes, u_bytes

    def summary(self) -> dict[str, Any]:
        """Aggregate V8 function and byte-span coverage across first-party portal scripts."""
        total_bytes = 0
        used_bytes = 0
        fn_total = 0
        fn_used = 0
        first_party_urls: set[str] = set()
        for entry in self.raw_scripts:
            url = entry.get("url", "")
            if not url or "chrome-extension" in url or "cdn." in url or "unpkg." in url:
                continue
            first_party_urls.add(url.split("?")[0])
            for fn in entry.get("functions", []):
                fn_total += 1
                inc, tb, ub = self._measure_function_ranges(fn.get("ranges", []))
                fn_used += inc
                total_bytes += tb
                used_bytes += ub
        return {
            "first_party_urls": sorted(first_party_urls),
            "profiled_script_Snapshots": len(self.raw_scripts),
            "total_functions": fn_total,
            "covered_functions": fn_used,
            "function_coverage_pct": round((fn_used / fn_total) * PERCENT_SCALE, 2) if fn_total else 0.0,
            "total_byte_spans": total_bytes,
            "covered_byte_spans": used_bytes,
            "byte_coverage_pct": round((used_bytes / total_bytes) * PERCENT_SCALE, 2) if total_bytes else 0.0,
        }


def convert_webm_to_mp4(webm_source: Path, mp4_dest: Path) -> dict[str, Any]:
    """Transcode Playwright WebM recording into H.264 yuv420p MP4 (strictly zero GIF)."""
    ffmpeg_bin = shutil.which("ffmpeg") or "/usr/bin/ffmpeg"
    mp4_dest.parent.mkdir(parents=True, exist_ok=True)
    cmd = [
        ffmpeg_bin,
        "-y",
        "-i",
        str(webm_source),
        "-c:v",
        "libx264",
        "-preset",
        "fast",
        "-crf",
        "23",
        "-pix_fmt",
        "yuv420p",
        "-movflags",
        "+faststart",
        str(mp4_dest),
    ]
    t0 = time.monotonic()
    proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
    elapsed_ms = round((time.monotonic() - t0) * MILLIS_PER_SEC, 1)
    if proc.returncode != 0 or not mp4_dest.is_file():
        raise RuntimeError(f"FFmpeg MP4 conversion failed for {webm_source}: {proc.stderr[-400:]}")
    return {
        "filename": mp4_dest.name,
        "path": str(mp4_dest),
        "size_bytes": mp4_dest.stat().st_size,
        "encode_ms": elapsed_ms,
    }


@dataclass
class RecetteContext:
    """Shared state and helper facade passed across scenario modules (arity <= 4)."""

    browser: Browser
    base_url: str
    api_key: str
    output_dir: Path
    screenshots_dir: Path
    screencasts_dir: Path
    raw_video_dir: Path
    tracker: EndpointCoverageTracker = field(default_factory=EndpointCoverageTracker)
    console_guard: ConsoleGuard = field(default_factory=ConsoleGuard)
    v8_coverage: V8CoverageSession = field(default_factory=V8CoverageSession)
    steps: list[StepRecord] = field(default_factory=list)
    videos: list[dict[str, Any]] = field(default_factory=list)
    sample_phash_filename: str = ""
    sample_subtitle_hash: str = ""
    context: BrowserContext | None = None
    page: Page | None = None
    _shot_counter: int = 0

    def open_recorded_session(self) -> Page:
        """Open a fresh 1920x1080 recorded browser context and attach all telemetry hooks."""
        self.raw_video_dir.mkdir(parents=True, exist_ok=True)
        ctx = self.browser.new_context(
            viewport={"width": VIEWPORT_WIDTH, "height": VIEWPORT_HEIGHT},
            user_agent=DESKTOP_USER_AGENT,
            record_video_dir=str(self.raw_video_dir),
            record_video_size={"width": VIEWPORT_WIDTH, "height": VIEWPORT_HEIGHT},
            locale="fr-FR",
        )
        ctx.set_default_timeout(DEFAULT_TIMEOUT_MS)
        ctx.on("request", self.tracker.on_request)
        ctx.on("response", self.tracker.on_response)
        page = ctx.new_page()
        self.console_guard.attach(page)
        self.v8_coverage.start_for_page(page)
        self.context = ctx
        self.page = page
        return page

    def close_and_save_video(self, mp4_stem: str) -> None:
        """Harvest V8 coverage, close context to flush WebM, and encode H.264 MP4."""
        if not self.page or not self.context:
            return
        self.v8_coverage.harvest_current_page()
        video_obj = self.page.video
        self.context.close()
        webm_path_str = video_obj.path() if video_obj else ""
        self.page = None
        self.context = None
        if webm_path_str and os.path.exists(webm_path_str):
            webm_src = Path(webm_path_str)
            mp4_dest = self.screencasts_dir / f"{mp4_stem}.mp4"
            info = convert_webm_to_mp4(webm_src, mp4_dest)
            info["path"] = str(mp4_dest.relative_to(self.output_dir))
            self.videos.append(info)

    def shot(self, slug: str) -> str:
        """Capture a numbered 1920x1080 PNG screenshot and return its filename."""
        assert self.page is not None, "Active page required for screenshot"
        self._shot_counter += 1
        filename = f"{self._shot_counter:02d}_{slug}.png"
        dest = self.screenshots_dir / filename
        self.page.screenshot(path=str(dest), full_page=False)
        return filename

    def record_step(
        self,
        step_id: str,
        portal: str,
        feature: str,
        t0: float,
        assertions: list[str],
        metrics: dict[str, Any] | None = None,
        screenshot: str | None = None,
    ) -> None:
        """Append a PASS step record with monotonic latency and full audit decomposition."""
        elapsed_ms = round((time.monotonic() - t0) * MILLIS_PER_SEC, 1)
        rec = StepRecord(
            step_id=step_id,
            portal=portal,
            feature=feature,
            status="PASS",
            elapsed_ms=elapsed_ms,
            assertions=assertions,
            metrics=metrics or {},
            screenshot=screenshot,
        )
        self.steps.append(rec)
        shot_note = f" [{screenshot}]" if screenshot else ""
        print(f"  ✓ [{step_id}] ({portal}) {feature} — {elapsed_ms} ms{shot_note}")

    def export_manifest(self, started_iso: str, finished_iso: str, total_sec: float) -> Path:
        """Persist self-documenting JSON manifest with steps, endpoints, V8 coverage, and videos."""
        manifest = {
            "target_url": self.base_url,
            "started_at": started_iso,
            "finished_at": finished_iso,
            "total_duration_sec": round(total_sec, 2),
            "viewport": f"{VIEWPORT_WIDTH}x{VIEWPORT_HEIGHT}",
            "total_steps": len(self.steps),
            "passed_steps": sum(1 for s in self.steps if s.status == "PASS"),
            "console_errors_count": len(self.console_guard.errors),
            "console_errors": self.console_guard.errors,
            "console_warnings_count": len(self.console_guard.warnings),
            "endpoint_coverage": self.tracker.summary(),
            "v8_js_coverage": self.v8_coverage.summary(),
            "videos": self.videos,
            "steps": [asdict(s) for s in self.steps],
        }
        dest = self.output_dir / "recette_manifest.json"
        dest.write_text(json.dumps(manifest, indent=2, ensure_ascii=False), encoding="utf-8")
        return dest
