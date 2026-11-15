#!/usr/bin/env python3
"""Automated quality, structure, and security validator for workspace Agent Skills."""

from __future__ import annotations

import argparse
import json
import re
import shutil
import subprocess
import sys
from pathlib import Path

import yaml

EXIT_OK: int = 0
EXIT_FAILED: int = 1
MAX_SKILL_MD_LINES: int = 250
MAX_DESCRIPTION_CHARS: int = 1024
MIN_TRIGGER_CASES_PER_CLASS: int = 5
CHECK_CMD_TIMEOUT_SECS: float = 15.0

FORBIDDEN_LOCAL_PREFIX: str = "/" + "home/"
FORBIDDEN_FILE_URI: str = "file:" + "//"
FORBIDDEN_RFC1918_RE = re.compile("192" + r"\.168\.\d+\.\d+")
MARKDOWN_LINK_RE = re.compile(r"\[[^\]]+\]\(([^)]+)\)")


def _extract_frontmatter(skill_md: Path) -> tuple[dict[str, object], list[str]]:
    """Extracts and validates YAML frontmatter from SKILL.md."""
    text = skill_md.read_text(encoding="utf-8")
    lines = text.splitlines()
    errors: list[str] = []
    if len(lines) > MAX_SKILL_MD_LINES:
        errors.append(f"{skill_md}: {len(lines)} lines exceeds ceiling of {MAX_SKILL_MD_LINES}")
    if not text.startswith("---\n"):
        return {}, [*errors, f"{skill_md}: missing opening YAML frontmatter delimiter '---'"]
    parts = text.split("---", 2)
    if len(parts) < 3:
        return {}, [*errors, f"{skill_md}: unclosed YAML frontmatter"]
    meta = yaml.safe_load(parts[1]) or {}
    if not isinstance(meta, dict):
        return {}, [*errors, f"{skill_md}: frontmatter must be a YAML mapping"]
    return meta, errors


def _check_markdown_links(skill_md: Path) -> list[str]:
    """Verifies all relative file links in SKILL.md exist on disk."""
    errors: list[str] = []
    content = skill_md.read_text(encoding="utf-8")
    for raw_target in MARKDOWN_LINK_RE.findall(content):
        if raw_target.startswith(("http://", "https://", "mailto:", "#")):
            continue
        clean_target = raw_target.split("#", 1)[0]
        resolved = (skill_md.parent / clean_target).resolve()
        if not resolved.exists():
            errors.append(f"{skill_md}: broken relative link '{raw_target}'")
    return errors


def _check_zero_local_paths(skill_dir: Path) -> list[str]:
    """Ensures no machine-local paths or file:// URIs leak into skill files."""
    errors: list[str] = []
    for path in sorted(skill_dir.rglob("*")):
        if not path.is_file() or "__pycache__" in path.parts:
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        if FORBIDDEN_LOCAL_PREFIX in text:
            errors.append(f"{path}: contains forbidden machine-local path prefix")
        if FORBIDDEN_FILE_URI in text:
            errors.append(f"{path}: contains forbidden file URI scheme")
        if FORBIDDEN_RFC1918_RE.search(text):
            errors.append(f"{path}: contains hardcoded RFC1918 IP literal")
    return errors


def _check_trigger_evals(skill_dir: Path, expected_name: str) -> list[str]:
    """Validates evals/trigger_evals.json structure and minimum positive/negative coverage."""
    eval_file = skill_dir / "evals" / "trigger_evals.json"
    if not eval_file.is_file():
        return [f"{skill_dir}: missing required evals/trigger_evals.json"]
    try:
        data = json.loads(eval_file.read_text(encoding="utf-8"))
    except json.JSONDecodeError as exc:
        return [f"{eval_file}: invalid JSON: {exc}"]

    errors: list[str] = []
    if data.get("skill_name") != expected_name:
        errors.append(f"{eval_file}: skill_name '{data.get('skill_name')}' != '{expected_name}'")
    pos = data.get("positive_triggers", [])
    neg = data.get("negative_triggers", [])
    if len(pos) < MIN_TRIGGER_CASES_PER_CLASS:
        errors.append(f"{eval_file}: need >= {MIN_TRIGGER_CASES_PER_CLASS} positive_triggers, got {len(pos)}")
    if len(neg) < MIN_TRIGGER_CASES_PER_CLASS:
        errors.append(f"{eval_file}: need >= {MIN_TRIGGER_CASES_PER_CLASS} negative_triggers, got {len(neg)}")
    return errors


def _check_executable_scripts(skill_dir: Path, repo_root: Path) -> list[str]:
    """Runs --check on every Python helper script inside skill/scripts/."""
    scripts_dir = skill_dir / "scripts"
    if not scripts_dir.is_dir():
        return []
    errors: list[str] = []
    for script in sorted(scripts_dir.glob("*.py")):
        proc = subprocess.run(
            [sys.executable, str(script), "--check"],
            cwd=repo_root,
            capture_output=True,
            text=True,
            timeout=CHECK_CMD_TIMEOUT_SECS,
            check=False,
        )
        if proc.returncode != 0:
            errors.append(f"{script} --check failed (rc={proc.returncode}): {proc.stderr.strip()}")
    return errors


def _run_agentskills_cli(skill_dir: Path) -> list[str]:
    """Runs official agentskills validate via uvx when available."""
    if not shutil.which("uvx"):
        return []
    proc = subprocess.run(
        ["uvx", "--from", "skills-ref", "agentskills", "validate", str(skill_dir)],
        capture_output=True,
        text=True,
        timeout=CHECK_CMD_TIMEOUT_SECS,
        check=False,
    )
    if proc.returncode != 0:
        detail = proc.stderr.strip() or proc.stdout.strip()
        return [f"agentskills validate failed for {skill_dir}: {detail}"]
    return []


def validate_single_skill(skill_dir: Path, repo_root: Path) -> list[str]:
    """Validates a single skill directory across spec, links, security, and scripts."""
    skill_md = skill_dir / "SKILL.md"
    if not skill_md.is_file():
        return [f"{skill_dir}: missing SKILL.md"]

    meta, errors = _extract_frontmatter(skill_md)
    name = str(meta.get("name", ""))
    desc = str(meta.get("description", ""))
    if name != skill_dir.name:
        errors.append(f"{skill_md}: frontmatter name '{name}' must match folder '{skill_dir.name}'")
    if not desc or len(desc) > MAX_DESCRIPTION_CHARS:
        errors.append(f"{skill_md}: description length ({len(desc)}) must be 1..{MAX_DESCRIPTION_CHARS}")

    errors.extend(_check_markdown_links(skill_md))
    errors.extend(_check_zero_local_paths(skill_dir))
    errors.extend(_check_trigger_evals(skill_dir, skill_dir.name))
    errors.extend(_check_executable_scripts(skill_dir, repo_root))
    errors.extend(_run_agentskills_cli(skill_dir))
    return errors


def discover_skills(skills_root: Path) -> list[Path]:
    """Lists all skill directories under .agents/skills/."""
    if not skills_root.is_dir():
        return []
    return sorted(p for p in skills_root.iterdir() if p.is_dir() and (p / "SKILL.md").is_file())


def main(argv: list[str] | None = None) -> int:
    """CLI entry point for listing or validating workspace Agent Skills."""
    parser = argparse.ArgumentParser(description="Validate workspace Agent Skills (.agents/skills/*).")
    parser.add_argument("--list", action="store_true", help="List discovered workspace skills and exit.")
    parser.add_argument("--repo", default="", help="Repository root override.")
    args = parser.parse_args(argv)

    repo_root = Path(args.repo).resolve() if args.repo else Path(__file__).resolve().parents[2]
    skills_root = repo_root / ".agents" / "skills"
    skill_dirs = discover_skills(skills_root)

    if not skill_dirs:
        print(f"No workspace skills found in {skills_root}", file=sys.stderr)
        return EXIT_FAILED

    if args.list:
        rows = []
        for sdir in skill_dirs:
            meta, _ = _extract_frontmatter(sdir / "SKILL.md")
            line_cnt = len((sdir / "SKILL.md").read_text(encoding="utf-8").splitlines())
            rows.append({"name": sdir.name, "lines": line_cnt, "description": meta.get("description", "")})
        print(json.dumps({"skills": rows}, indent=2, ensure_ascii=False))
        return EXIT_OK

    all_errors: list[str] = []
    for sdir in skill_dirs:
        errs = validate_single_skill(sdir, repo_root)
        if errs:
            all_errors.extend(errs)
        else:
            print(f"✓ Skill valid: {sdir.relative_to(repo_root)}")

    if all_errors:
        for err in all_errors:
            print(f"✗ {err}", file=sys.stderr)
        return EXIT_FAILED
    return EXIT_OK


if __name__ == "__main__":
    sys.exit(main())
