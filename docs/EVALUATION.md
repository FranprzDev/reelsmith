# Evaluation: reference renders

Reelsmith changes are judged against **reference renders**: committed fixtures that pair a config with
the QA values a good render of it actually produced. A change that drifts a reference without intending
to is a regression, even when every unit test passes.

References live in `evaluations/<name>/`:

- `expected-qa.json` - values recorded from a real reviewed render, with tolerances. Never write numbers
  you did not measure.
- `check.py` - stdlib-only comparison of a fresh `qa.json` against the reference.

## The sample-ad reference

`evaluations/sample-ad/` covers `examples/sample/brief.toml` at `--preview`. Its media is procedural
(the `make_sample_clips` tool draws the same clips from FFmpeg filters), so anyone can reproduce it
without footage:

```sh
# 1. generate the sample media (one JSON-RPC call to the MCP server)
printf '%s\n' \
  '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"local","version":"0"}}}' \
  '{"jsonrpc":"2.0","method":"notifications/initialized"}' \
  '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"make_sample_clips","arguments":{"folder":"<repo>/examples/sample/media"}}}' \
  | (cat; sleep 10) | target/release/reelsmith

# 2. render and QA
target/release/promo build examples/sample/brief.toml --preview

# 3. compare against the reference
python3 evaluations/sample-ad/check.py
```

The comparison covers every QA check by name, duration, output size, integrated loudness, true peak and
the per-shot source-motion medians (within a relative tolerance). It does not replace eyes: the contact
sheet in `build/<name>/qa/contact.png` is still the final visual review.

## Adding a reference

1. Build the config and review the render: QA green **and** the contact sheet looks right.
2. Copy the measured values out of the fresh `build/<name>/qa/qa.json` into the fixture's
   `expected-qa.json`, choosing tolerances that reflect real run-to-run noise (loudness already has its
   own target tolerance; motion medians tolerate 50% - slow clips vary, frozen ones read ~0.001).
3. Record in the fixture how it was produced (config, flags, where the media came from, date).
4. Media never enters this public repo (see `.gitignore`); a reference must be reproducible from
   committed sources plus a documented generation step.
