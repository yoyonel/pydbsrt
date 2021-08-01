import asyncio
import sys
from functools import wraps
from typing import Any, Awaitable, Callable, Coroutine, Optional, Set

import click


def all_tasks(
    loop: Optional[asyncio.AbstractEventLoop] = None,
) -> Set["asyncio.Task[Any]"]:
    tasks = list(asyncio.all_tasks(loop))
    return {t for t in tasks if not t.done()}


def _cancel_all_tasks(loop: asyncio.AbstractEventLoop) -> None:
    to_cancel = all_tasks(loop)
    if not to_cancel:
        return

    click.secho(f"\U0001F4A3 Cancelling {len(to_cancel)} tasks.", fg="magenta")
    for task in to_cancel:
        task.cancel()

    click.secho(f"\U0001F4A3 Wait for {len(to_cancel)} tasks to finish.", fg="magenta")
    loop.run_until_complete(asyncio.gather(*to_cancel, loop=loop, return_exceptions=True))

    for task in to_cancel:
        # KEEPME: while we find out why this was added.
        # if task.cancelled():
        #     continue
        if task.exception() is not None:
            loop.call_exception_handler(
                {
                    "message": "unhandled exception during asyncio.run() shutdown",
                    "exception": task.exception(),
                    "task": task,
                }
            )


def run_coro(coro: Coroutine):
    loop = asyncio.get_event_loop()
    task = None
    complete = False
    try:
        # Serve requests until Ctrl+C is pressed
        task = loop.create_task(coro)
        loop.run_until_complete(task)
        complete = True
    except KeyboardInterrupt:
        click.secho("\nInterrupted", fg="red", bold=True)
    except asyncio.CancelledError:  # pragma: no cover
        click.secho("\nCancelled", fg="red", bold=True)
    finally:
        if task:
            if not complete:
                click.secho("\U0001F4A3 Cancelling main task.", fg="magenta")
            if task.exception():
                click.secho(f"\U0001F4A3 Error on main task: {task.exception()}", fg="magenta")
            else:
                task.cancel()
                try:
                    loop.run_until_complete(task)
                except asyncio.CancelledError:  # pragma: no cover
                    pass
        _cancel_all_tasks(loop)
        if sys.version_info >= (3, 6):  # don't use PY_36 to pass mypy
            if not complete:
                click.secho("\U0001F4A3 Shutting down async generators.", fg="magenta")
            loop.run_until_complete(loop.shutdown_asyncgens())
        loop.close()


def coroclick(func: Callable[..., Awaitable]):
    @wraps(func)
    def wrap_async(*args, **kwargs):
        run_coro(func(*args, **kwargs))

    return wrap_async
