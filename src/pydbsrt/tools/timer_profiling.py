import logging

from contexttimer import Timer

logger = logging.getLogger(__name__)


def _Timer(step_name):
    def _log_timer(output):
        logger.info(output.replace("took 0.0000 seconds", "took less than 0.1 ms"))

    return Timer(output=_log_timer,
                 fmt="took {:.4f} seconds",
                 prefix="{} =>".format(step_name))
