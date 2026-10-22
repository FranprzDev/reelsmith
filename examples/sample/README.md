# Sample: a complete ad from procedural media

No footage needed: `make_sample_clips` draws four 6 s clips with FFmpeg's own sources (`nebula` Mandelbrot zoom,
`cells` Game of Life, `aurora` moving gradients, `grid` test pattern) and two alpha motion graphics with a short sound
(`badge` title card, `cta` full-frame end card). The ad: 4 beats + end card, about 11 s, vertical.

## Through an MCP host

> Use reelsmith to make sample clips in `<this folder>/media`, create a project in `<this folder>/ad` with them (9:16,
> procedural music, captions off), write the brief from `examples/sample/write_brief.json` and render a preview.

`write_brief.json` holds the exact `write_brief` arguments (add `project_dir`).

## With the CLI

```sh
# media first (once), e.g. through the MCP tool above; then:
promo build examples/sample/brief.toml --preview   # out/sample_ad_preview_360x640.mp4
promo build examples/sample/brief.toml             # 720p draft
```
