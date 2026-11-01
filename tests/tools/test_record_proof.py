"""Tests for scripts/record_proof.py (masked terminal proofs published in the public repo)."""

from __future__ import annotations

import importlib.util
import json
import re
import sys
from pathlib import Path

import pytest

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "scripts"))
_spec = importlib.util.spec_from_file_location("record_proof", ROOT / "scripts" / "record_proof.py")
assert _spec and _spec.loader
record_proof = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(record_proof)

VALUES = ["nas.example.lan", "secretuser"]


@pytest.mark.parametrize(
    "raw,masked",
    [
        ('"phash_dir": "/home/alice/dbsrt/run"', '"phash_dir": "/home/<user>/dbsrt/run"'),
        ("dir /home/alice", "dir /home/<user>"),
        ("/home/alice\x1b[0m done", "/home/<user>\x1b[0m done"),
        ("GET https://api.example.org/v1/x?k=1 ok", "GET <url> ok"),
        ("ftp://u:p@nas/share", "<url>"),
        ("mail admin@example.org now", "mail <email> now"),
        ("peer 192.168.1.20:22", "peer <ip>:22"),
        ("Authorization: Bearer abc.def", "Authorization: Bearer <redacted>"),
        ("X-API-Key: k3y", "X-API-Key: <redacted>"),
        ('{"token": "t0k"}', '{"token": "<redacted>"}'),
        ("password=hunter2 next", "password=<redacted> next"),
        ("smb nas.example.lan as secretuser", "smb <masked> as <masked>"),
        ("ffmpeg version 8.1.2 dbsrt/bin/ffmpeg", "ffmpeg version 8.1.2 dbsrt/bin/ffmpeg"),
        ("total_frames: 205211607", "total_frames: 205211607"),
    ],
)
def test_mask_hides_host_identifying_values(raw: str, masked: str) -> None:
    assert record_proof.mask(raw, VALUES) == masked


def test_mask_is_idempotent_and_leaves_nothing_to_report() -> None:
    raw = "/home/alice https://h.example/x a@b.org 10.0.0.1 Bearer zz nas.example.lan"
    once = record_proof.mask(raw, VALUES)
    assert record_proof.mask(once, VALUES) == once
    assert record_proof.leaks(once, VALUES) == []


def test_longer_value_is_masked_before_its_prefix() -> None:
    assert record_proof.mask("nas.example.lan.backup", ["nas.example", "nas.example.lan.backup"]) == "<masked>"


@pytest.mark.parametrize(
    "text,category",
    [
        ("/home/alice/x", "home"),
        ("see http://h.example", "url"),
        ("a@b.org", "email"),
        ("10.0.0.1", "ipv4"),
        ("password: x", "secret"),
        ("on secretuser box", "value"),
    ],
)
def test_leaks_reports_category_never_the_value(text: str, category: str) -> None:
    found = record_proof.leaks(text, VALUES)
    assert found == [category]
    assert "alice" not in " ".join(found) and "secretuser" not in " ".join(found)


def test_svg_namespaces_are_not_leaks() -> None:
    svg = '<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink">'
    assert record_proof.leaks(svg, VALUES) == []


@pytest.mark.parametrize(
    "svg_text,found",
    [
        ("<text>/home/&lt;user&gt;/dbsrt &lt;url&gt; &lt;masked&gt;</text>", []),
        ("<text>/home/alice/dbsrt</text>", ["home"]),
        ("<text>nas.example&#46;lan</text>", ["value"]),
    ],
)
def test_xml_escaped_svg_text_is_checked_unescaped(svg_text: str, found: list[str]) -> None:
    assert record_proof.leaks(svg_text, VALUES) == found


def test_env_values_come_from_env_files(tmp_path: Path) -> None:
    env = tmp_path / ".env"
    env.write_text('# comment\nNAS_HOST="nas.example.lan"\nNAS_USER=secretuser\nEMPTY=\n', encoding="utf-8")
    assert record_proof.env_values([env, tmp_path / "missing"]) == ["nas.example.lan", "secretuser"]


def test_record_writes_masked_cast(tmp_path: Path) -> None:
    command = [sys.executable, "-c", "import sys; print('/home/alice/x'); print('nas.example.lan', file=sys.stderr)"]
    cast = record_proof.record("demo", command, tmp_path, VALUES)
    text = cast.read_text(encoding="utf-8")
    header = json.loads(text.splitlines()[0])
    assert cast == tmp_path / "demo.cast"
    assert header["timestamp"] > 1729900000
    assert "/home/<user>/x" in text and "<masked>" in text
    assert "alice" not in text and "nas.example.lan" not in text


def test_cast_shows_real_facts_only(tmp_path: Path) -> None:
    long_line = "x" * 130
    cast = record_proof.record("demo", [sys.executable, "-c", f"print('{long_line}')"], tmp_path, VALUES)
    lines = cast.read_text(encoding="utf-8").splitlines()
    header = json.loads(lines[0])
    typed = "".join(json.loads(line)[2] for line in lines[1:] if len(json.loads(line)[2]) == 1)
    assert "develop" not in cast.read_text(encoding="utf-8")  # no made-up branch in the prompt
    assert re.search(r"# recorded \d{4}-\d{2}-\d{2} \d{2}:\d{2} UTC", typed)
    assert header["width"] > len(long_line)


def test_record_masks_the_command_line_too(tmp_path: Path) -> None:
    command = [sys.executable, "-c", "print('ok')", "nas.example.lan"]
    text = record_proof.record("demo", command, tmp_path, VALUES).read_text(encoding="utf-8")
    assert "nas.example.lan" not in text


def test_failed_command_writes_no_proof(tmp_path: Path) -> None:
    with pytest.raises(SystemExit, match="rc=3"):
        record_proof.record("demo", [sys.executable, "-c", "raise SystemExit(3)"], tmp_path, VALUES)
    assert list(tmp_path.iterdir()) == []


@pytest.mark.parametrize("name", ["../x", "a b", "", "x/y"])
def test_unsafe_names_are_refused(tmp_path: Path, name: str) -> None:
    with pytest.raises(SystemExit, match="name"):
        record_proof.record(name, [sys.executable, "-c", "print(1)"], tmp_path, VALUES)


def test_ensure_clean_deletes_a_leaking_file(tmp_path: Path) -> None:
    leaking = tmp_path / "x.svg"
    leaking.write_text("<text>/home/alice</text>", encoding="utf-8")
    with pytest.raises(SystemExit, match="home"):
        record_proof.ensure_clean(leaking, VALUES)
    assert not leaking.exists()
