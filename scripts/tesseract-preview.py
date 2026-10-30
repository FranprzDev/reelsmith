#!/usr/bin/env python3
"""Render a low-resolution silent MP4 preview from a Tesseract project.

This is an optional asset bridge: it calls an externally installed ``tsrct``
binary and FFmpeg. It does not add a Reelsmith engine or editor feature.
"""

import argparse
import json
import math
import shutil
import subprocess
import sys
import tempfile
from pathlib import Path


def fail(message):
    print(f"error: {message}", file=sys.stderr)
    raise SystemExit(1)


def run(cmd):
    try:
        return subprocess.run(cmd, check=True, capture_output=True, text=True)
    except FileNotFoundError:
        fail(f"command not found: {cmd[0]}")
    except subprocess.CalledProcessError as exc:
        detail = (exc.stderr or exc.stdout or "").strip()
        if detail:
            fail(f"command failed: {' '.join(cmd)}\n{detail}")
        fail(f"command failed: {' '.join(cmd)}")


def parse_dimension(value, name):
    if isinstance(value, bool):
        fail(f"{name} must be an integer, got {value!r}")
    if isinstance(value, int):
        pass
    elif isinstance(value, float):
        if not math.isfinite(value) or not value.is_integer():
            fail(f"{name} must be an integer, got {value!r}")
        value = int(value)
    else:
        fail(f"{name} must be an integer, got {value!r}")
    if isinstance(value, bool) or not isinstance(value, int):
        fail(f"{name} must be an integer, got {value!r}")
    if value <= 0 or value % 2 != 0:
        fail(f"{name} must be a positive even integer, got {value}")
    return value


def parse_duration(value):
    if isinstance(value, bool) or isinstance(value, str) or not isinstance(value, (int, float)):
        fail(f"duration must be a number, got {value!r}")
    value = float(value)
    if not math.isfinite(value):
        fail(f"duration must be a finite number, got {value!r}")
    if not (value > 0 and value <= 10):
        fail(f"duration must be > 0 and <= 10 seconds, got {value}")
    return value


def inspect_project(tsrct, project):
    result = run([tsrct, "project", "inspect", "-p", str(project)])
    try:
        info = json.loads(result.stdout)
    except json.JSONDecodeError:
        fail(f"inspect did not return JSON: {result.stdout.strip()}")
    if not isinstance(info, dict):
        fail("inspect JSON must be an object")
    dimensions = info.get("dimensions")
    if not isinstance(dimensions, dict):
        fail("inspect JSON did not contain dimensions")
    try:
        width_raw = dimensions["width"]
        height_raw = dimensions["height"]
        duration_raw = info["duration"]
    except KeyError:
        fail("inspect JSON missing required field")
    width = parse_dimension(width_raw, "width")
    height = parse_dimension(height_raw, "height")
    duration = parse_duration(duration_raw)
    short_side, long_side = sorted((width, height))
    if short_side > 360:
        fail(f"short side must be <= 360, got {short_side}")
    if long_side > 640:
        fail(f"long side must be <= 640, got {long_side}")
    return width, height, duration


def main():
    parser = argparse.ArgumentParser(description="Render a low-resolution silent MP4 preview from a Tesseract project.")
    parser.add_argument("--tsrct", required=True)
    parser.add_argument("--project", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--fps", type=int, default=15)
    args = parser.parse_args()

    tsrct = args.tsrct
    project = Path(args.project).resolve()
    output = Path(args.output).resolve()

    if not (1 <= args.fps <= 60):
        fail("--fps must be an integer from 1 to 60")
    if not project.is_file():
        fail(f"project not found: {project}")
    if output.exists() or output.is_symlink():
        fail(f"refusing to overwrite existing output: {output}")
    if str(output) == str(project):
        fail("refusing output equal to source")
    if output.parent and not output.parent.exists():
        fail(f"output directory does not exist: {output.parent}")

    _, _, duration = inspect_project(tsrct, project)

    frame_count = math.ceil(duration * args.fps)

    with tempfile.TemporaryDirectory(prefix="tesseract-preview-") as tmp:
        tmp_dir = Path(tmp)
        for i in range(frame_count):
            frame_path = tmp_dir / f"frame-{i + 1:05d}.png"
            time_seconds = i / args.fps
            run([tsrct, "preview", "-p", str(project), "--time", str(time_seconds), "--output", str(frame_path)])
            if not frame_path.is_file():
                fail(f"preview did not create frame: {frame_path}")

        pattern = str(tmp_dir / "frame-%05d.png")
        temp_mp4 = tmp_dir / "temp.mp4"
        run(["ffmpeg", "-n", "-framerate", str(args.fps), "-i", pattern, "-c:v", "libx264", "-pix_fmt", "yuv420p", str(temp_mp4)])
        if not temp_mp4.is_file():
            fail(f"encode did not create output: {temp_mp4}")

        output_created = False
        try:
            with open(temp_mp4, "rb") as src:
                with open(output, "xb") as dst:
                    output_created = True
                    shutil.copyfileobj(src, dst)
        except FileExistsError:
            fail(f"refusing to overwrite existing output: {output}")
        except OSError as exc:
            if output_created:
                try:
                    output.unlink(missing_ok=True)
                except OSError:
                    pass
            fail(f"failed to publish output: {exc}")

    if not output.is_file():
        fail(f"encode did not create output: {output}")


if __name__ == "__main__":
    main()
