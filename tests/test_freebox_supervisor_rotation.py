from scripts import rpi4_o2switch_supervisor as sup
from scripts.rpi4_o2switch_supervisor import NEXT_BATCH_TYPE, decide_next_batch


def test_batch_rotation_cycle():
    assert NEXT_BATCH_TYPE["tvshow"] == "movies"
    assert NEXT_BATCH_TYPE["movies"] == "freebox"
    assert NEXT_BATCH_TYPE["freebox"] == "tvshow"


def test_finished_batch_transitions_to_next():
    assert decide_next_batch("finished", "tvshow", 120.0) == "movies"
    assert decide_next_batch("finished", "movies", 120.0) == "freebox"
    assert decide_next_batch("finished", "freebox", 120.0) == "tvshow"


def test_finished_batch_respects_short_cooldown():
    assert decide_next_batch("finished", "freebox", 10.0) is None
    assert decide_next_batch("finished", "freebox", 65.0) == "tvshow"


def test_crashed_batch_uses_relaunch_backoff():
    # Crashed stays on same type
    assert decide_next_batch("crashed", "freebox", 50.0) is None
    assert decide_next_batch("crashed", "freebox", 1900.0) == "freebox"


def test_failed_freebox_launch_falls_back_to_next_library(monkeypatch):
    launched = []

    def fake_restart(_host, _base, _workers, batch_type="tvshow"):
        launched.append(batch_type)
        return batch_type != "freebox"

    monkeypatch.setattr(sup, "restart_remote_batch", fake_restart)
    monkeypatch.setattr(sup, "get_remote_desired_workers", lambda *_: 5)
    status = {"state": "finished", "pid": None, "type": "movies"}
    sup._relaunch_step("o2switch", "dbsrt", "finished", status, sup._LoopState())
    assert launched == ["freebox", "tvshow"]
