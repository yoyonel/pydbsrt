from dataclasses import dataclass

Hash = str | bytes


@dataclass(frozen=True)
class PHashMedia:
    p_hash: int
    frame_offset: int
