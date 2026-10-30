# Tesseract low-resolution preview bridge

`scripts/tesseract-preview.py` is an optional local asset bridge. It converts a
Tesseract project into a silent MP4 preview that can be dropped into an existing
Reelsmith clips folder. It is not a Reelsmith engine, editor, MCP tool, or part
of the normal render pipeline.

## Runtime

Install the Tesseract runtime separately from the official release:

- Project: https://github.com/mirage-hq/Tesseract
- Release: https://github.com/mirage-hq/Tesseract/releases/tag/v0.4.0
- Verify the downloaded archive with its published checksum before use.
- Linux `x86_64` with glibc 2.35+ and a Vulkan driver (lavapipe option) is a
  possible target.

Tesseract is proprietary software with its own license conditions; external usage is
limited and no categorical legal permission is granted. This script supports optional
external asset authoring only; it does not bundle the Tesseract
runtime, create a new engine, or add an editor. It does not redistribute
Tesseract, host it, or build a competing engine. See
[TERMS.md](https://github.com/mirage-hq/Tesseract/blob/main/TERMS.md) for the
applicable terms.

## Keep source files

Keep the `.tsrct` project file, fonts, and source assets with your project. The
script reads the project through `tsrct project inspect` and
`tsrct preview`; it does not modify the source.

## Preconditions

- Local low-resolution authoring only: dimensions must be positive even integers
  with short side <= 360 and long side <= 640, and duration must be greater
  than 0 and at most 10 seconds.
- No upscaling or HD intermediate is performed.
- Output must not already exist and must not be the source project.
- FFmpeg must be installed separately.
- The generated MP4 is silent: no audio, loudness proof, or whole-pipeline claim.
- Source must already be low-resolution; this is a silent-only proof.

## Validation

Validated with the official CLI 0.4.0: it renders an editable 6-layer, 4s, 202x360,
15 fps silent source through `scripts/tesseract-preview.py`. Reelsmith and promo were
built unchanged with `cargo build --locked -p reelsmith -p promo`. A real MCP flow ran
`list_clips` -> `create_project` (music: procedural, aspect 9:16) -> `write_brief`
(one 4s beat with source parallax zoom 1,1) -> `render` (resolution: preview,
encoder: x264), succeeding in 5.8s. Output: 360x640, 30 fps, H.264/AAC, 120 frames;
integrated -13.96 LUFS, true peak -2.30 dBTP; no black frames; motion median Y-DIF
0.232. No LLM calls or keys during rendering; telemetry off.

This verifies one opaque MP4 asset path only: not ProRes alpha/audio input, HD, or
custom projects, and not visual parity with the elaborate X demo. The script renders
silent frames; Reelsmith added procedural music.

## Usage

```sh
python3 scripts/tesseract-preview.py \
  --tsrct /path/to/tsrct \
  --project /path/to/project.tsrct \
  --output /path/to/clips/tesseract-preview.mp4 \
  --fps 15
```

`--fps` defaults to `15`.

## Reelsmith handoff

Place the resulting MP4 in an existing Reelsmith clips folder, then use the
normal flow: `list_clips`, `write_brief`, and `render` with a 360p preview
first.
