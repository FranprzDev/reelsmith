# Promo copy and design: writing beats that convert

Copy and motion guidance for the promo template, derived from the open source `landing-page-design` skill by
[elayadesign/ai-design-skills](https://github.com/elayadesign/ai-design-skills) (MIT). The original targets landing
pages; this document adapts its strategy, copy and motion principles to short vertical video. It is guidance for
writing `brief.toml` content, not a code contract: [`PROMO_VIDEOS.md`](PROMO_VIDEOS.md) stays the config reference.

## Structure: the landing arc, mapped to beats

The conversion structure of a landing page (hero, benefits, how it works, proof, CTA) maps onto a promo timeline:

| Landing section | Promo equivalent | Notes |
|---|---|---|
| Hero | beat 1 (the hook) | The first 1-2 s decide the scroll-past. Lead with the strongest, most specific claim. |
| Benefits | middle beats | One idea per beat. Benefit before feature: say what the viewer gets, not what the product has. |
| How it works | 2-3 beats, max | Three steps, each one beat. If it needs more, the ad is the wrong format. |
| Proof | a beat near the end | A number, a name, a result. Specific beats generic. |
| Final CTA | `[end_card]` | One action, said once, on screen long enough to read and act (the template default holds it ~4.5 s). |

Rules carried over unchanged:

- **One offer, one audience, one primary action.** A promo that pushes two actions converts neither.
- **Specific numbers instead of vague verbs.** "Join 12,400 players" beats "join thousands". Avoid vague value props
  like "streamline" or "optimize".
- **Proof sits next to the claim it supports.** If a beat claims speed, the next beat shows the number.
- **Cut risk, not corners.** If there is a guarantee or a free tier, it earns a beat.

## Copy rules for captions and voice-over

Caption space is tiny, so the landing copy rules apply harder:

- **Clear over clever.** The headline says what it is; the clever version goes in the hook only if it stays clear.
- **No filler copy.** No lorem ipsum, no placeholder brands, no round fake numbers (`99.99%`, `$100.00`). Use organic
  values (`47.2%`, `$99.00`).
- **No AI cliches.** Never "Elevate", "Seamless", "Unleash", "Next Gen", "Game changer", "Delve".
- **Sentence case, active voice.** "We could not save your changes", not "Mistakes were made".
- **Short caption groups.** The template splits groups on `|` and times 0.45-0.8 s per group; groups that need more
  than ~1 s of reading time get cut. Rewrite, do not stretch `dur`.
- **CTA wording names the action**: "Play free", "Watch the demo", not "Submit" or "Learn more" without an object.

## Motion principles

The skill's motion rule (no default transitions; everything moves with real-world spring easing) is already how the
harness behaves; keep it that way when authoring assets:

- The shared easing in `crates/datavid` (`cubic-bezier(0.32, 0.72, 0, 1)`) matches the skill's fluid curve. Use it for
  any new animated element instead of linear or default easings.
- **Reveal, don't appear.** Overlays and text should enter with motion (scale pop, draw-on, fade-up) rather than
  cutting in statically. The caption pop and `draw_on` animations cover this.
- **Cuts carry energy, not effects.** A `flash` beat is the section break; do not stack transitions on top of it.
- **Sound sells motion.** An overlay moving without its `sfx` reads as broken; keep the asset's sound or add a `cue`.

## Pre-flight checklist for a new promo

- [ ] Hook beat makes one specific claim in under 2 s
- [ ] One idea per beat; benefit before feature
- [ ] At least one proof beat with a real number
- [ ] End card asks for exactly one action
- [ ] No placeholder copy, no round fake numbers, no AI cliches
- [ ] Every caption group readable in its slot (`promo check` / `promo plan` before rendering)
- [ ] Preview at 360p watched end to end before the draft render

## Attribution

Adapted from `landing-page-design` in <https://github.com/elayadesign/ai-design-skills>, MIT license, copyright
Elaya. Only the strategy/copy/motion principles were carried over; landing-specific layout and code rules (Tailwind
scales, spacing tokens, SEO) do not apply to video and were intentionally left out.
