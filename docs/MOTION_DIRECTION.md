# Motion direction: briefs that move

A gallery of Claude-made motion videos, [Prompt Motion](https://promptmotion.org/), shows what the strongest
prompts have in common: they ask for continuous action, not a sequence of slides. Each prompt belongs to its
creator and links to the original post. This guide turns that pattern into briefs Reelsmith can render today.
No new tool is needed.

## Principles

1. **One continuous idea.** Every beat starts from where the previous one ended: the same object, color or
   position carries across the cut. Avoid beats that each introduce an unrelated card.
2. **Something moves in every beat.** Use `zoom = [start, end]` push-ins on clips, and an overlay that draws or
   scales on, never a static title that just appears.
3. **Cut on action.** Put the cut where the motion peaks and add `flash = true` only on section changes, not
   on every cut.
4. **Draw, don't pop.** Diagrams and icons enter with `draw_on` (`draw_diagram`, `animate_svg`), so the
   viewer watches the idea being built.
5. **Texture with restraint.** Use `stylize_clip` (ASCII or dither) for one transition beat or one section,
   not the whole video.
6. **Every frame is a function of time.** The HyperFrames engine (`engine = "hyperframes"`) suits scenes
   where each frame is computed from the timeline: HTML/CSS/JS motion rendered frame by frame and
   assembled with FFmpeg, the same approach as the code-driven prompts in the gallery. Keep `fframes` for
   clip-based promos.

## Writing the brief

State, per beat: what is on screen at the start, what moves, and what carries into the next beat. Example:

```toml
[[beat]]
text = "ONE REQUEST"
clips = [{ clip = "terminal", in = 0.0, zoom = [1.0, 1.25] }]
overlay = "diagram_draw_on"

[[beat]]
text = "ONE VIDEO"
clip = "render@2.4"
flash = true
```

## Render flow

1. `promo check` and `promo plan` first.
2. Render the mandatory **360p preview** and review the contact sheet for continuity: does each beat move,
   and does the end of one beat match the start of the next?
3. Stay on 360p previews while iterating. Render 720p or 1080p only when the final is requested.

## Where the ideas came from

- Code-driven frame capture with FFmpeg assembly: <https://promptmotion.org/xelandre-363f00>
- The gallery and its prompt library: <https://promptmotion.org/> and <https://promptmotion.org/prompts>
