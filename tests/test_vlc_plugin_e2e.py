"""
tests/test_vlc_plugin_e2e.py
End-to-end and integration test harness for VLC Lua plugins (vlsub_dbsrt and dbsrt_intf).
Validates bytecode syntax, hermetic Lua mocks, headless cvlc execution, and GUI search/download.
"""

from __future__ import annotations

import os
import shutil
import subprocess
import time
from pathlib import Path

import pytest


def test_lua_unit_tests():
    """Tier 1: Run hermetic Lua unit test suite with mocked LibVLC environment."""
    lua_bin = shutil.which("lua")
    if not lua_bin:
        pytest.skip("lua executable not found on system")

    cmd = [lua_bin, "tests/lua/test_vlsub_dbsrt.lua"]
    res = subprocess.run(cmd, capture_output=True, text=True, check=False)
    assert res.returncode == 0, f"Lua unit tests failed:\n{res.stdout}\n{res.stderr}"
    assert "All" in res.stdout and "PASSED (100%)" in res.stdout


def test_lua_bytecode_syntax():
    """Tier 1: Validate bytecode compilation with luac for all plugins."""
    luac_bin = shutil.which("luac")
    if not luac_bin:
        pytest.skip("luac executable not found on system")

    for script in [
        "vlc-plugin/vlsub_dbsrt.lua",
        "vlc-plugin/dbsrt_intf.lua",
    ]:
        path = Path(script)
        assert path.exists(), f"Plugin script {script} missing"
        res = subprocess.run([luac_bin, "-p", str(path)], capture_output=True, text=True, check=False)
        assert res.returncode == 0, f"Bytecode check failed for {script}: {res.stderr}"


def test_cvlc_headless_integration():
    """Tier 2: Test headless cvlc integration with event-driven autokill."""
    cvlc_bin = shutil.which("cvlc")
    if not cvlc_bin:
        pytest.skip("cvlc executable not found on system")

    # Ensure plugins are installed
    install_res = subprocess.run(["bash", "scripts/vlc_ctl.sh", "install"], capture_output=True, text=True, check=False)
    assert install_res.returncode == 0, f"Plugin installation failed: {install_res.stderr}"

    # Run cvlc headless auto-sync
    res = subprocess.run(["bash", "scripts/vlc_ctl.sh", "test-cvlc"], capture_output=True, text=True, check=False)
    assert res.returncode == 0, f"cvlc test failed:\n{res.stdout}\n{res.stderr}"
    assert "SUCCESS! Media matched" in res.stdout


def test_vlc_gui_phash_search_e2e():
    """Tier 3: Test GUI VLSub dialog under Xvfb with real video and Search by pHash."""
    vlc_bin = shutil.which("vlc")
    xdotool_bin = shutil.which("xdotool")
    xvfb_bin = shutil.which("Xvfb")

    if not (vlc_bin and xdotool_bin and xvfb_bin):
        pytest.skip("vlc, xdotool or Xvfb missing for GUI E2E testing")

    video_path = Path("tests/data/big_buck_bunny_trailer_480p.mp4")
    assert video_path.exists(), "Test video fixture missing"

    display_num = 97
    env = os.environ.copy()
    env["DISPLAY"] = f":{display_num}"

    xvfb = subprocess.Popen(["Xvfb", f":{display_num}", "-screen", "0", "1920x1080x24", "-ac"])
    time.sleep(1.0)
    marco = subprocess.Popen(
        ["marco", f"--display=:{display_num}"], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL
    )
    time.sleep(0.5)

    log_path = Path("/tmp/pytest_vlc_gui.log")
    if log_path.exists():
        log_path.unlink()

    vlc_proc = None
    try:
        with open(log_path, "w") as log_f:
            vlc_proc = subprocess.Popen(
                ["vlc", "-vv", "--no-audio", "--no-sub-autodetect-file", str(video_path)],
                env=env,
                stdout=log_f,
                stderr=log_f,
            )
        time.sleep(2.5)

        # Trigger View menu -> VLSub (dbsrt Enhanced)
        subprocess.run(["xdotool", "key", "alt+i", "Up", "Up", "Return"], env=env, check=False)
        time.sleep(1.5)

        win_out = subprocess.run(["xdotool", "search", "--name", "VLSub"], env=env, capture_output=True, text=True)
        win_ids = win_out.stdout.strip().splitlines()
        assert len(win_ids) > 0, "VLSub dialog window was not opened"
        win_id = win_ids[-1]

        # Activate window and click 'Search by pHash' (x=430, y=87)
        subprocess.run(["xdotool", "windowactivate", "--sync", win_id], env=env, check=False)
        time.sleep(0.2)
        subprocess.run(["xdotool", "mousemove", "--window", win_id, "430", "87", "click", "1"], env=env, check=False)
        time.sleep(2.0)

        # Select first candidate (x=200, y=140) and click Download selection (x=290, y=365)
        subprocess.run(["xdotool", "mousemove", "--window", win_id, "200", "140", "click", "1"], env=env, check=False)
        time.sleep(0.5)
        subprocess.run(["xdotool", "mousemove", "--window", win_id, "290", "365", "click", "1"], env=env, check=False)
        time.sleep(2.0)

    finally:
        if vlc_proc:
            vlc_proc.terminate()
            vlc_proc.wait(timeout=3)
        marco.terminate()
        xvfb.terminate()
        # Restore fixture if modified by download test
        subprocess.run(["git", "checkout", "HEAD", "--", "tests/data/big_buck_bunny_trailer_480p.fr.srt"], check=False)

    # Assert VLC logs contain zero Lua runtime crashes
    with open(log_path, errors="ignore") as f:
        logs = f.read()

    assert "attempt to index" not in logs, f"Lua nil index error detected in VLC log:\n{logs}"
    assert "lua generic warning: Error while running script" not in logs, f"Lua script crash detected:\n{logs}"
