# Performance

Where a Reelsmith / `promo` build spends its time, what was changed, and how to run it on a Windows PC with an
NVIDIA RTX 4060.

## Where the time goes

A build runs `check → plate → audio → render → mux → srt → qa`. On a cold 360p preview of a 22.5 s ad (13 shots, 7
alpha overlays, 675 frames), before the changes below:

| stage | time | what it is |
|---|---|---|
| plate | 25.2 s (56%) | ffmpeg decodes each source clip / motion graphic and writes JPG / PNG frames |
| render | 8.5 s | fframes rasterises each frame's SVG in chunk processes (lossless intermediate) |
| mux | 4.7 s | joins the chunks, encodes the delivery H.264, muxes the audio |
| audio | 3.6 s | music, hits, cues, overlay sfx, −14 LUFS mastering |
| qa | 1.9 s | loudness, black frames, contact sheet |
| **total** | **44.6 s** | |

(2-core arm64 sandbox, no GPU, libx264, build folder on local disk. The same build took 62–76 s with the build folder on
a network filesystem: keep `build/` on a local SSD, not in a synced or network folder.)

## What changed (measured)

All timings are from CI release binaries on the same 2-core machine. Every change below produces a byte-identical MP4
for the preview (same md5 as the reference render); the preview QA passes (22.500 s, 360x640, 675 frames, −14.04 LUFS,
0 black frames).

| change | PR | measured on 2 cores | why it scales on the client |
|---|---|---|---|
| plates cut in parallel (`--jobs`, default every core) | #9 | plate 25.2 → 22.1 s | each plate is its own ffmpeg; QuickTime RLE overlays decode on one thread |
| render chunks in parallel (same `--jobs`) | #11 | render 8.5 → 7.4 s | one fframes chunk process uses about one core |
| FFVHuff instead of FFV1 for the lossless chunks | #14 | 720p sample: render 11.0 → 9.6 s, mux 5.6 → 4.6 s | the mux decodes every chunk on one thread; FFVHuff encodes / decodes ~5x faster |
| GPU backend detection, hardware decode, per-tier presets | #13 | preview unchanged (libx264 `veryfast`) | NVENC encode + CUDA decode on the RTX 4060 |

SpookMC preview end to end, cold, 2 cores (CI release binaries):

| build | total | plate | render | mux |
|---|---|---|---|---|
| before (`01a9896`) | 44.6 s | 23.8–25.2 s | 8.4–8.5 s | 4.7–5.8 s |
| #9 + #11 + #14 (`3f791d9`), 2 runs | 45.0 / 43.0 s | 27.4 / 25.5 s | 7.2 / 7.1 s | 3.8 / 3.8 s |
| + #13 (GPU backend, no GPU here) | 40.2 s | 21.6 s | 7.7 s | 3.9 s |

On 2 cores the end-to-end gain is within noise: the plate stage swings by ±3 s between identical runs, and the old
pipeline already kept both cores busy (each ffmpeg / fframes process is multi-threaded). The stages the changes
target moved consistently: render −15% and mux −20 to −35%. All three MP4s are byte-identical to the reference.

### Expected scaling on 8+ cores (estimate, not measured)

Of the ~41 s, about 10 s is serial and does not change with cores: check 0.6 s, audio 3.7 s, mux 3.8 s, qa 2.1 s.
The other ~29 s (plate + render) now runs on the worker pool, one ffmpeg or fframes process per job, and used both
cores fully here. If it scales at 60–80% efficiency, 8 cores bring it to roughly 9–12 s, so a preview of this ad
lands around 20–23 s on a CPU of the same speed, and less on a current desktop CPU, which is faster per core than this
sandbox. Before #9 / #11 the plates and chunks ran one at a time, so extra cores mostly sat idle. With the RTX 4060,
NVENC takes the delivery encode off the CPU in the mux and CUDA decode takes the H.264 clip decode off the CPU in the
plates; the serial floor (audio, qa) stays. Draft and final spend proportionally more time in plate and render, so
they gain more from extra cores than the preview does.

Sample ad (`examples/sample`, 292 frames, 720p draft): 28.9 s → 24.9 s (−14%) with #9, #11 and #14, on 2 cores.

Since PR #27 the chunks are no longer lossless intermediates: each chunk streams its frames into the selected H.264
encoder once and the final mux stream-copies the video (audio encoded once). On a 12.1 s, 363-frame test with one
chunk job and libx264: mux 10.4 s → 0.3 s, a fully cached rebuild 12.8 s → 2.65 s, cold chunks 38.4 s → 37.2 s
(within noise). The output is no longer pixel-identical to the old lossless route (SSIM 0.9931 against it); the joined
video is identical to its encoded chunks. The FFV1 / FFVHuff numbers below are kept for the record.

Lossless intermediate (before #27), 675 frames at 360p, one thread:

| codec | encode | decode | size |
|---|---|---|---|
| FFV1 | 2.66 s | 2.96 s | 60.1 MB |
| FFVHuff (#14, until #27) | 0.57 s | 0.62 s | 87.5 MB |

### Tried and rejected

- **Thin LTO + one codegen unit** (#10, closed): 44.6 → 45.3 s total, render-only 11.2 / 11.3 s vs 11.4 / 11.0 s, within
  noise. The hot code is fframes' rasteriser and FFmpeg (a prebuilt C library), so it only made CI slower.
- **`panic = "abort"`**: the MCP server runs each job on a blocking task and must survive a job that panics.
- **`target-cpu=native`**: release binaries are copied to client machines whose CPU is unknown.
- **Lower PNG compression for overlay frames**: 455 → 372 ms for one overlay at 360p; the decode dominates, too small to
  matter.
- **Ut Video** for the chunks: about as fast as FFVHuff, but its encoder could not be confirmed in the static FFmpeg
  that fframes links.

## GPU backend

`--encoder auto` (the default, also the MCP `render` default) test-opens each encoder with a one-frame FFmpeg encode
and uses the first that opens:

| OS | order |
|---|---|
| Windows | `h264_nvenc` (NVIDIA) → `h264_amf` (AMD) → `h264_qsv` (Intel) → `libx264` |
| Linux | `h264_nvenc` → `h264_vaapi` (AMD / Intel) → `libx264` |
| macOS | `h264_videotoolbox` → `libx264` |

The paired hardware decoder cuts the source clips: CUDA (NVIDIA), D3D11VA (AMD / Intel on Windows), VA-API (Linux),
VideoToolbox (macOS). It is test-opened too; when it does not open the clips are decoded on the CPU and the reason
says so. The MCP tool `system_info` runs the detection and every render result carries `backend` (encoder, GPU,
decoder, each probe with FFmpeg's error, and the reason). `--encoder nvenc|amf|qsv|vaapi|videotoolbox|x264` forces
one and fails with FFmpeg's message when it cannot open.

Presets per quality tier:

| backend | preview (360p) | draft (720p) | final (1080p) |
|---|---|---|---|
| NVENC | `p4` | `p5` | `p6` + `-temporal_aq 1 -rc-lookahead 32` |
| (all NVENC) | `-tune hq -rc vbr -cq 19 -b:v 0 -bf 3 -spatial_aq 1` | same | `-cq 18` |
| AMF | `speed`, CQP 20/22 | `balanced`, 19/21 | `quality`, 18/20 |
| QSV | `veryfast`, ICQ 21 | `medium`, 20 | `slower`, 19 |
| VA-API | CQP 21 | 20 | 19 |
| libx264 (CRF 18) | `veryfast` | `fast` | `medium` |
| VideoToolbox | ~0.15 bit per pixel | same | same |

On a machine without a GPU, the slower libx264 presets cost time: the 720p sample draft's mux went from 5.6 s
(`veryfast`) to 10.2 s (`fast`) on 2 cores, for a 12% bigger file at SSIM 0.993 against the `veryfast` encode. With
the RTX 4060 the delivery encode runs on NVENC and this does not apply.

## Recommended setup for an RTX 4060 (Windows)

1. **FFmpeg**: `winget install Gyan.FFmpeg` (a full build with NVENC, CUDA decode and libx264). Keep the NVIDIA driver
   current; if `system_info` says `h264_nvenc` did not open with "Driver does not support the required nvenc API
   version", update the driver.
2. **Check the backend once**: call `system_info` from the MCP host (or run a preview). Expected: `encoder: nvenc`,
   `gpu: nvidia`, `hw_decode: cuda`.
3. **Leave `encoder` on `auto`** and **`--jobs` at its default** (every logical core). With 16 GB of RAM or less, a
   1080p final with many cores can run out of memory (one fframes process per job): pass `--jobs` equal to the
   physical core count, or half of it, if Windows starts paging.
4. **Keep projects and `build/` on a local SSD**, not in OneDrive / a network share.
5. **Render previews and drafts first**, the final only once: the final is 9x the pixels of the preview.

| tier | use it for | on the RTX 4060 |
|---|---|---|
| `preview` (360p) | cut, timing, captions and audio checks; iterate here | NVENC `p4`, fastest |
| `draft` (720p) | the version the client reviews | NVENC `p5` |
| `final` (1080p) | delivery, once the draft is approved | NVENC `p6` + temporal AQ, lookahead 32 |

## Still to measure on the client's GPU

- NVENC vs libx264 mux time at 720p / 1080p, and the NVENC presets' quality (SSIM against the libx264 encode).
- CUDA decode's effect on the plate stage (the 1080x1920 60 fps H.264 clips) with many plates in parallel.
- `--jobs` scaling on 8+ cores and the memory of a 1080p final with the default `--jobs`.
