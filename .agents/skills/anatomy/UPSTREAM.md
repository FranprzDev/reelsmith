# Upstream provenance

Vendored from https://github.com/wheresryan22/anatomy at commit
`1482e8469b0a9e0d8cc18ea2740a01423d29ae80` (2026-10-18).
Author: Ryan (@wheresryan22). License: MIT, preserved in `LICENSE` alongside this file.

The upstream skill and its kit, references, scripts, tests and example sources are
unchanged, with one exception: the generated example pages (`examples/*/*.html`) and
PDFs (`examples/*/*.pdf`) are not vendored. Rebuild any of them with
`node examples/<name>/build.mjs` (add `--light` for the light theme). Three built
pages are kept because the regression suite in `scripts/` runs against them:
`examples/turning-dial/turning-dial-verify-light.html` and
`examples/dial-indicator/dial-indicator{,-light}.html`. Reelsmith-specific usage is
in `docs/ANATOMY.md`; no npm installer is run when using the MCP server.

To update, review the upstream diff first, replace this directory's upstream files,
preserve this notice and LICENSE, and run every `scripts/*.test.mjs` test as in
`docs/ANATOMY.md` ("Verify the vendored skill"). The capture, drive and print scripts
launch a local Chrome/Chromium and execute page JavaScript, so only run them on pages
you have built and reviewed. Do not run untrusted model scripts.
