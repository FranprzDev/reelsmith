# Logo intro and end card from an SVG

`logo.svg` is an original mark made for this example (a reel ring, spokes and a play spark), so it can be shipped and
reused freely. `animate_svg` turns it into two alpha motion graphics, and the sample ad uses them as an intro logo over
the first beat and as the end card.

## Through an MCP host

> Use reelsmith to make sample clips in `<media>`, then animate `examples/logo-intro/logo.svg` twice into `<media>`:
> `animate_intro.json` (pop with a back overshoot, hold, fade out) and `animate_end.json` (glow with a slow pulse).
> Create a project in `<this folder>/ad` with `<media>` as clips and motion folder (9:16, procedural music, captions
> off), write the brief from `write_brief.json` and render a preview.

The JSON files hold the exact tool arguments (replace `<repo>` and `<media>` with absolute paths, add `project_dir` to
`write_brief.json`).

## How the overlay is referenced

`animate_svg` returns `asset_name` (the output file's stem). With the `.mov` in the project's motion folder:

* **Intro logo**: the first beat's `overlay.asset = "logo_intro"`, `delay_s = 0` so it starts with the ad. Position it
  with `x` / `y` / `w` on the 1080-wide design canvas; the crop is measured from the alpha channel, so the canvas padding
  around the artwork does not count towards `w`.
* **End card**: `end_card.asset = "logo_end"`. End card assets are drawn uncropped (the whole canvas), so give `w`
  and centre it: for a square 1080 px animation at `w = 800` on a 9:16 ad, `x = 140`, `y = (1920 - 800) / 2 = 560`.

An overlay slot longer than its asset holds the last frame, so an entrance without an exit simply stays on screen.

In a hand-written `brief.toml` the same thing is:

```toml
[[beat]]
text = "WHAT IF | YOUR SERVER | HAD A TRAILER?"
dur = 2.6
clips = [{ clip = "nebula", in = 0.5 }]
overlay = { asset = "logo_intro", delay = 0.0, crop = "auto", y = 300.0, w = 640.0 }

[end_card]
clip = { clip = "aurora", in = 2.0 }
asset = "logo_end"
dur = 3.5
x = 140.0
y = 560.0
w = 800.0
```
