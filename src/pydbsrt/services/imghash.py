from itertools import islice
from pathlib import Path

# https://pypi.org/project/click-pathlib/
from numpy.ma import floor
from pydbsrt.services.reader_frames import build_reader_frames
from pydbsrt.tools.chunk import chunks
from pydbsrt.tools.ffmpeg_tools.ffmeg_extract_frame import rawframe_to_imghash
from pydbsrt.tools.imghash import imghash_to_bytes
from pydbsrt.tools.rich_colums import TimeElapsedOverRemainingColumn
from rich.console import Console
from rich.progress import (
    BarColumn,
    DownloadColumn,
    Progress,
    TextColumn,
    TransferSpeedColumn,
)

console = Console()


def export_imghash_from_media(media, output_file) -> Path:
    ################################################################
    # PROCESSING                                                   #
    ################################################################
    # Read a video file
    reader, meta = build_reader_frames(media)
    console.print(meta)
    nb_frames_to_read = int(floor(meta["fps"] * meta["duration"]))
    gen_frame_hash = map(rawframe_to_imghash, reader)
    gen_frame_hash = islice(
        gen_frame_hash, nb_frames_to_read + 0 * int(meta["fps"] * 60)
    )
    chunk_nb_seconds = 15
    chunk_size = int(meta["fps"] * chunk_nb_seconds)
    console.print(f"Chunk size (nb frames): {chunk_size}")
    ################################################################
    # EXPORT                                                       #
    ################################################################
    output_file = (
        Path(output_file)
        if output_file
        else Path("/tmp/") / media.with_suffix(".phash").name
    )
    output_file.unlink(missing_ok=True)
    console.print(f"output_file: {str(output_file)}")
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
    task_id = progress.add_task(
        "build&export images hashes", filename=media.name, start=True
    )
    progress.update(task_id, total=nb_frames_to_read)
    with progress:
        for chunk_frames_hashes in chunks(gen_frame_hash, chunk_size):
            with output_file.open("ab") as fo:
                for frame_hash_binary in map(imghash_to_bytes, chunk_frames_hashes):
                    fo.write(frame_hash_binary)
            progress.update(task_id, advance=chunk_size, refresh=True)
    return output_file
