import logging
from enum import Enum
from typing import Any, Tuple

from rich.console import Console
from rich.progress import BarColumn, DownloadColumn, Progress, TextColumn, TransferSpeedColumn
from rich.theme import Theme

# https://pypi.org/project/click-pathlib/
from pydbsrt.tools.rich_colums import TimeElapsedOverRemainingColumn

# TODO: put this configuration (theme, console, progress bar) in `tools`
custom_theme = Theme({"info": "dim cyan", "warning": "magenta", "danger": "bold red"})
console = Console(theme=custom_theme)

progress = Progress(
    TextColumn("[bold blue]{task.fields[filename]}", justify="right"),
    BarColumn(bar_width=None),
    "[progress.percentage]{task.percentage:>3.1f}%",
    "•",
    DownloadColumn(),
    "•",
    TransferSpeedColumn(),
    "•",
    TimeElapsedOverRemainingColumn(),
    console=console,
)


class _PrintLevel(Enum):
    DEBUG = logging.DEBUG
    INFO = logging.INFO
    WARNING = logging.WARNING
    ERROR = logging.ERROR


_print_level_format: dict[_PrintLevel, Tuple[str, str]] = {
    _PrintLevel.DEBUG: (":bug: ", "logging.level.debug"),
    _PrintLevel.INFO: (":information: ", "logging.level.info"),
    _PrintLevel.WARNING: (":warning: ", "logging.level.warning"),
    _PrintLevel.ERROR: (":cross_mark: ", "danger"),
}


def _enhanced_print(print_level: _PrintLevel, *objects: Any, **kwargs) -> None:
    print_symbol, print_style = _print_level_format[print_level]
    msg, *_objects = objects
    console.print(*(f"{print_symbol}️{msg}", *_objects), **{**kwargs, **{"style": print_style}})


def print_debug(*objects: Any, **kwargs) -> None:
    _enhanced_print(_PrintLevel.DEBUG, *objects, **kwargs)


def print_info(*objects: Any, **kwargs) -> None:
    _enhanced_print(_PrintLevel.INFO, *objects, **kwargs)


def print_warning(*objects: Any, **kwargs) -> None:
    _enhanced_print(_PrintLevel.WARNING, *objects, **kwargs)


def print_error(*objects: Any, **kwargs) -> None:
    _enhanced_print(_PrintLevel.ERROR, *objects, **kwargs)
