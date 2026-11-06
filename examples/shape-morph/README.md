# Shape morph

`morph_svg` turns one shape into another as an alpha motion graphic. This example morphs a red circle
into a yellow star over 1.2 s, holds the star for a second, then morphs back:

```
morph_svg(from_svg = ".../examples/shape-morph/circle.svg",
          to_svg   = ".../examples/shape-morph/star.svg",
          duration_s = 1.2, hold_s = 1.0, morph_back_s = 1.2,
          output = "<project>/motion/circle-star.mov")
```

Both SVGs are fitted to the same square canvas first, so the morph reads as pure shape transformation.
Equal path counts morph pairwise (as here: one path into one path); one shape can also split into many
paths, or many merge into one. Morphing works on filled shapes: outline strokes first (stroke to path).
Save the `.mov` in the project's motion folder and use its `asset_name` as a beat `overlay.asset` or
`end_card.asset`, like any `animate_svg` result.
