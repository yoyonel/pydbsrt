"""
"""
import logging

import tqdm


class TqdmLoggingHandler(logging.Handler):
    def __init__(self, level=logging.NOTSET):
        super().__init__(level)

    def emit(self, record):
        try:
            msg = self.format(record)
            tqdm.tqdm.write(msg)
            self.flush()
        except (KeyboardInterrupt, SystemExit):
            raise
        except:  # noqa: E722
            self.handleError(record)


def config_logger(
    logger: logging.Logger,
    formatter: str = "%(asctime)s - %(name)s - %(levelname)s - %(message)s",
    level_name: str = "INFO",
):
    logger.setLevel(logging._nameToLevel[level_name])
    tqdm_logging_handler = TqdmLoggingHandler()
    tqdm_logging_handler.setFormatter(logging.Formatter(formatter))
    logger.addHandler(tqdm_logging_handler)
