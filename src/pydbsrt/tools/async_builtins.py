from typing import AsyncIterator, TypeVar

_T = TypeVar("_T")


async def anext(ait: AsyncIterator[_T]) -> _T:
    return await ait.__anext__()
