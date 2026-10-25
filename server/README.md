# reelsmith-telemetry

A tiny Cloudflare Worker + D1 API that receives the optional render summaries sent by `promo` and the Reelsmith
MCP server (see "Render telemetry" in the root README). Rust, via
[workers-rs](https://github.com/cloudflare/workers-rs).

Endpoints:

- `POST /event` (Bearer `INGEST_TOKEN`): stores one event. `INSERT OR IGNORE` on `event_id` makes client retries
  and offline-queue replays safe.
- `GET /summary` (Bearer `SUMMARY_TOKEN`, a separate token): render counts and seconds grouped by install id and
  resolution.

Setup (your own Cloudflare account; the free tier covers this many times over -
[100k requests/day](https://developers.cloudflare.com/workers/platform/pricing/),
[100k D1 writes/day](https://developers.cloudflare.com/d1/platform/pricing/)):

```sh
npm install -g wrangler worker-build
wrangler login
wrangler d1 create reelsmith-telemetry        # copy the database_id into wrangler.toml
wrangler d1 execute reelsmith-telemetry --remote --file schema.sql
wrangler secret put INGEST_TOKEN              # random string, goes to the rendering machines
wrangler secret put SUMMARY_TOKEN             # different random string, stays with you
wrangler deploy                               # runs worker-build, then publishes
```

Then on each rendering machine:

```sh
export REELSMITH_TELEMETRY_ENDPOINT=https://reelsmith-telemetry.<you>.workers.dev/event
export REELSMITH_TELEMETRY_TOKEN=<INGEST_TOKEN>
promo telemetry          # review exactly what would be sent, to where, and how to disable
promo telemetry accept   # explicit per-machine opt-in; nothing is sent before this
```

Count the renders:

```sh
curl -H "Authorization: Bearer $SUMMARY_TOKEN" https://reelsmith-telemetry.<you>.workers.dev/summary
```

The deploy steps above were not run by the project; they need your account and your approval.
