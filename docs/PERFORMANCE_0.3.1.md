# v0.3.1 performance validation

Measured on Linux CPU with local files and release binaries. Results depend on the workload and host. Kernel improvements below do not add up to the same percentage improvement in a complete video.

## Complete build comparison

A 6-second, 180-frame 360x640 preview with three moving clips, an alpha overlay, captions, procedural audio and QA was built from scratch five times per version. Runs alternated old/new order, using libx264, one job and separate new build directories.

| Version | Median total |
|---|---:|
| Published v0.3.0 | 10.258 s |
| Combined optimization build, through #82 | 9.932 s |

The measured gain is **3.2%**, not the sum of the kernel percentages. All ten MP4 files have the same SHA-256, `ed54ac98d381e7b817b1ba0682e53b8b64d4b1cb200f09d5fc9d0662d355713e`, and passed QA. The final WAV streaming change (#83) is measured separately below.

## Thirty distinct optimizations

Tests, fixes, documentation and rejected experiments are not counted as optimizations. Some independent changes were measured together; their shared result is not evidence of a separate percentage gain for each one.

| # | Computation removed or reduced | PR | Evidence |
|---:|---|---|---|
| 1 | Check media tools once per discovery list | #58 | Together with #2: 24 clips, 1.3884→0.6959 s |
| 2 | Bound and parallelize discovery probes | #58 | Same discovery measurement, metadata/error parity |
| 3 | Hoist Kaiser normalization denominator | #59 | Together with #4-6: sampled audio 0.4862→0.4376 s |
| 4 | Reuse interpolation FIR coefficients | #59 | Same audio batch; WAV byte parity |
| 5 | Reuse decimation FIR coefficients | #59 | Same audio batch; WAV byte parity |
| 6 | Reuse mastering ceiling representation | #59 | Same audio batch; procedural 0.8358→0.7706 s |
| 7 | Cache SVG definitions by dimensions | #60 | Together with #8-9: 10k SVGs 57.09→33.05 ms |
| 8 | Iterate overlays without a temporary vector | #60 | Same SVG batch; 180 decoded frames identical |
| 9 | Build SVG into one document buffer | #60 | Same SVG batch; 180 decoded frames identical |
| 10 | Notify job completion instead of polling | #61 | Response after completion: 450.2 ms→24.5 us; not compute speed |
| 11 | Combine crop/overlay metadata probes | #62 | Overlay preparation 0.3778→0.3336 s; 58 PNGs identical |
| 12 | Batch caption QA captures | #63 | Explicit QA 1.1957→1.0477 s; JPEG/JSON/contact parity |
| 13 | Skip zero-wet convolution | #64 | Together with #14: sampled audio 0.9110→0.8349 s |
| 14 | Skip unused impulse response generation | #64 | 4.420 ms of IR work avoided; WAV byte parity |
| 15 | Hoist boom saturation divisor | #65 | 50-call kernel 215.614→186.227 ms; exact bits |
| 16 | Probe stylize metadata once | #67 | Probe stage 119.86→59.97 ms |
| 17 | Precompute Bayer thresholds | #67 | 30-frame kernel 50.240→42.317 ms; actual stylize 1.324→1.247 s |
| 18 | Stream chunk-key hashing | #68 | 10k synthetic chunks 73.317→23.635 ms; same keys |
| 19 | Reuse identical morph hold frames | #69 | Actual 90-frame morph 1.9784→1.8680 s; all frames identical |
| 20 | Fast RGBA conversion for opaque/zero pixels | #71, fixed by #73 | Kernel 2.399→1.028 ms; glow/pop 180 frames identical after fix |
| 21 | Reuse static non-glow animation holds | #72 | Actual 90-frame animation 1.2717→0.5933 s; all frames identical |
| 22 | Array lookup for ASCII font advances | #74 | Width kernel 86.475→16.480 ms; 180 frames identical |
| 23 | Bounded loudness interleave buffer | #75 | 90s peak RSS 174828→107312 KiB; no speed gain claimed |
| 24 | Cache current caption width | #76 | Repeated-text kernel 16.596→3.239 ms; 180 frames identical |
| 25 | One-pass SVG escaping | #79 | Typical ASCII 13.671→8.665 ms; dense escapes 64.708→31.440 ms |
| 26 | Rename new mux outputs instead of copying | #78 | 128MiB copy 40.474→rename 0.025 ms; existing outputs retain copy semantics |
| 27 | Bound decimation taps before accumulation | #80 | Oversampled 6s channel kernel 20.718→17.350 ms; exact bits/WAV |
| 28 | Fuse ASCII background/glyph writes | #81 | 30-frame kernel 96.499→82.009 ms; actual clip time unchanged |
| 29 | Share font database within one morph | #82 | Two-source option preparation 65.672→32.349 ms; 90 frames identical |
| 30 | Stream WAV channels without interleaved copy | #83 | 90s decode RSS 138452→70988 KiB, 210.621→174.432 ms |

For #83, five alternating old/new process runs on 48kHz stereo gave a 48.7% peak-RSS reduction and 17.2% decode-stage speed gain for 90 seconds. For six seconds: 12428→7976 KiB and 12.894→11.375 ms. Eighteen format/channel/raw-bit tests cover PCM16/24/32, float32, mono/stereo/three-channel input, signed zeros, infinities, NaNs and partial-frame helpers. Decode failures now return an error rather than panicking.

## Compatibility and rejected experiments

- The first #71 implementation changed hidden RGB values on zero-alpha glow pixels. #73 corrected this before the optimization release; final glow/pop decoded frames match the previous implementation. The fix is not counted as another optimization.
- Mux rename is restricted to absent destinations. Existing files, hard links and symlinks keep the previous copy behavior, including inode and permission semantics.
- The width cache is deliberately small; different-text calls can have cache overhead. It is a repeated-caption optimization.
- Lower PNG compression (#66), parallel uncached crops (#70), caption temporary-string removal (#77), a new interpolation loop, monochrome luminance hoisting and in-place limiter experiments were rejected for no measurable gain, regression or a size trade-off. They are not included in the thirty.
- Audio/render overlap (#49) remains a draft with no end-to-end gain demonstrated on the test host.
- GPU speed and client-machine scaling were not measured here.

Use the linked PR numbers in this repository for implementation, tests and measurement details. No paid service or new runtime dependency is required for these changes.
