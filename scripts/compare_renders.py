#!/usr/bin/env python3
"""Compare two renders of the same video (stdlib + the ffmpeg / ffprobe CLIs).

usage: compare_renders.py A.mp4 B.mp4

Prints JSON: for each file its duration, size, fps, decoded frame count, integrated loudness, true peak and loudness
range; then SSIM and PSNR of B against A and the loudness delta. Use it to prove that a change is (or is not)
pixel-identical before merging it: the MCP path vs the CLI path, before vs after a performance change, an encoder
switch. Identical frames give SSIM 1.0 and PSNR inf; plain 8-bit rounding differences land around 0.9999 / 60+ dB.
Both files need the same size and frame rate for SSIM / PSNR to mean anything.
"""
import json
import re
import subprocess
import sys


def run(cmd):
    p = subprocess.run(cmd, capture_output=True, text=True)
    if p.returncode != 0:
        raise SystemExit(f"{cmd[0]} failed on {cmd}: {p.stderr[-800:]}")
    return p


def probe(f):
    out = run(["ffprobe", "-v", "error", "-count_frames", "-show_entries",
               "format=duration:stream=codec_type,width,height,r_frame_rate,nb_read_frames", "-of", "json", f]).stdout
    j = json.loads(out)
    v = next(s for s in j["streams"] if s["codec_type"] == "video")
    return {"duration_s": float(j["format"]["duration"]), "width": v["width"], "height": v["height"],
            "fps": v["r_frame_rate"], "frames": int(v["nb_read_frames"]),
            "has_audio": any(s["codec_type"] == "audio" for s in j["streams"])}


def loudness(f):
    err = run(["ffmpeg", "-nostats", "-hide_banner", "-i", f, "-af", "ebur128=peak=true", "-f", "null", "-"]).stderr
    s = err[err.rfind("Summary:"):]
    return {"integrated_lufs": float(re.search(r"I:\s+(-?[\d.]+) LUFS", s).group(1)),
            "true_peak_dbtp": float(re.search(r"Peak:\s+(-?[\d.]+) dBFS", s).group(1)),
            "lra_lu": float(re.search(r"LRA:\s+(-?[\d.]+) LU", s).group(1))}


def video_quality(a, b):
    err = run(["ffmpeg", "-hide_banner", "-i", a, "-i", b, "-lavfi", "[0:v][1:v]ssim;[0:v][1:v]psnr",
               "-f", "null", "-"]).stderr
    ssim = float(re.search(r"SSIM .*All:([\d.]+)", err).group(1))
    m = re.search(r"PSNR .*average:([\d.]+|inf)", err).group(1)
    return {"ssim": ssim, "psnr_db": "inf" if m == "inf" else float(m)}


def main(argv):
    if len(argv) != 3:
        print(__doc__, file=sys.stderr)
        return 2
    a, b = argv[1], argv[2]
    pa, pb = probe(a), probe(b)
    out = {"a": {"file": a, **pa}, "b": {"file": b, **pb}}
    if pa["has_audio"]:
        out["a"].update(loudness(a))
    if pb["has_audio"]:
        out["b"].update(loudness(b))
    if (pa["width"], pa["height"]) == (pb["width"], pb["height"]):
        out.update(video_quality(a, b))
    else:
        out["note"] = "different sizes: SSIM / PSNR skipped"
    if "integrated_lufs" in out["a"] and "integrated_lufs" in out["b"]:
        out["loudness_delta_lu"] = round(abs(out["a"]["integrated_lufs"] - out["b"]["integrated_lufs"]), 2)
    out["same_frame_count"] = pa["frames"] == pb["frames"]
    print(json.dumps(out, indent=1))
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
