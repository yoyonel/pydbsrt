"""The o2switch gateway rate-limits failed logins by the connection IP, not a client header."""

import json
import shutil
import socket
import subprocess
import time
import urllib.error
import urllib.request
from collections.abc import Iterator
from pathlib import Path

import pytest

REPO_ROOT = Path(__file__).resolve().parents[2]
GATEWAY = REPO_ROOT / "deploy" / "o2switch" / "index.php"
MAX_FAILED_ATTEMPTS = 5

pytestmark = pytest.mark.skipif(shutil.which("php") is None, reason="php CLI not installed")


def free_port() -> int:
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


@pytest.fixture
def gateway_url(tmp_path: Path) -> Iterator[str]:
    # Same layout as o2switch: ~/public_html/dbsrt/index.php reads ~/dbsrt/etc and ~/dbsrt/run
    docroot = tmp_path / "public_html" / "dbsrt"
    docroot.mkdir(parents=True)
    shutil.copy(GATEWAY, docroot / "index.php")
    (tmp_path / "dbsrt" / "etc").mkdir(parents=True)
    (tmp_path / "dbsrt" / "run").mkdir(parents=True)
    (tmp_path / "dbsrt" / "etc" / "api_key.secret").write_text("the-real-key\n")

    port = free_port()
    # Same error settings as o2switch (display_errors = Off); errors go to a log the tests read
    error_settings = ["-d", "display_errors=0", "-d", "log_errors=1", "-d", f"error_log={tmp_path / 'php_errors.log'}"]
    server = subprocess.Popen(
        ["php", *error_settings, "-S", f"127.0.0.1:{port}", "-t", str(docroot), str(docroot / "index.php")],
        stdout=subprocess.DEVNULL,
        stderr=subprocess.DEVNULL,
    )
    try:
        for _ in range(50):
            try:
                socket.create_connection(("127.0.0.1", port), timeout=0.1).close()
                break
            except OSError:
                time.sleep(0.1)
        yield f"http://127.0.0.1:{port}/v1/status"
    finally:
        server.terminate()
        server.wait()


def status_of(req: urllib.request.Request) -> int:
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status
    except urllib.error.HTTPError as e:
        return e.code


def php_errors(tmp_path: Path) -> list[str]:
    log = tmp_path / "php_errors.log"
    return [line for line in log.read_text().splitlines() if "PHP " in line] if log.exists() else []


def status_with_wrong_key(url: str, forwarded_for: str) -> int:
    req = urllib.request.Request(
        url,
        headers={"X-API-Key": "wrong-key", "Accept": "application/json", "X-Forwarded-For": forwarded_for},
    )
    try:
        with urllib.request.urlopen(req, timeout=5) as resp:
            return resp.status
    except urllib.error.HTTPError as e:
        return e.code


def test_forged_forwarded_for_does_not_dodge_the_lockout(gateway_url: str) -> None:
    codes = [status_with_wrong_key(gateway_url, f"203.0.113.{i}") for i in range(MAX_FAILED_ATTEMPTS + 1)]
    assert codes[:MAX_FAILED_ATTEMPTS] == [401] * MAX_FAILED_ATTEMPTS
    assert codes[MAX_FAILED_ATTEMPTS] == 429


def test_forged_forwarded_for_cannot_lock_out_another_client(gateway_url: str, tmp_path: Path) -> None:
    for _ in range(MAX_FAILED_ATTEMPTS):
        status_with_wrong_key(gateway_url, "198.51.100.7")
    locked = json.loads((tmp_path / "dbsrt" / "run" / "auth_rate_limit.json").read_text())
    assert set(locked) == {"127.0.0.1"}


@pytest.mark.parametrize(
    ("query", "headers"),
    [
        ("?token[]=x", {}),
        ("?key[]=x", {}),
        ("", {"Cookie": "dbsrt_session[]=x"}),
    ],
    ids=["token-array", "key-array", "session-cookie-array"],
)
def test_array_shaped_credential_is_refused_without_php_error(
    gateway_url: str, tmp_path: Path, query: str, headers: dict[str, str]
) -> None:
    req = urllib.request.Request(gateway_url + query, headers={"Accept": "application/json", **headers})
    assert status_of(req) == 401
    assert php_errors(tmp_path) == []


def test_array_shaped_login_form_key_counts_as_a_failure_without_php_error(gateway_url: str, tmp_path: Path) -> None:
    req = urllib.request.Request(gateway_url, data=b"api_key[]=x", method="POST")
    status_of(req)
    locked = json.loads((tmp_path / "dbsrt" / "run" / "auth_rate_limit.json").read_text())
    assert set(locked) == {"127.0.0.1"}
    assert php_errors(tmp_path) == []
