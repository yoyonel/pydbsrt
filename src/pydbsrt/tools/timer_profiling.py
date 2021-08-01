import logging

from contexttimer import Timer

logger = logging.getLogger(__name__)


def _Timer(step_name, func_to_log=logger.info):
    def _log_timer(output):
        func_to_log(output.replace("took 0.0000 seconds", "took less than 0.1 ms"))

    return Timer(output=_log_timer, fmt="took {:.4f} seconds", prefix="{} =>".format(step_name))
