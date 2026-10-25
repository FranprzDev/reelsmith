use worker::*;

#[derive(serde::Deserialize)]
struct Event {
    event_id: String,
    install_id: String,
    ts: u64,
    resolution: String,
    ok: bool,
    exit_code: u8,
    seconds: f64,
    project: String,
}

fn authorized(req: &Request, env: &Env, secret_name: &str) -> bool {
    let Ok(expected) = env.secret(secret_name) else {
        return false;
    };
    req.headers()
        .get("Authorization")
        .ok()
        .flatten()
        .map(|h| h == format!("Bearer {}", expected))
        .unwrap_or(false)
}

#[event(fetch)]
async fn handle(req: Request, env: Env, _ctx: Context) -> Result<Response> {
    Router::new()
        .post_async("/event", |mut req, ctx| async move {
            if !authorized(&req, &ctx.env, "INGEST_TOKEN") {
                return Response::error("unauthorized", 401);
            }
            let e: Event = match req.json().await {
                Ok(e) => e,
                Err(_) => return Response::error("bad request", 400),
            };
            let db = ctx.env.d1("DB")?;
            db.prepare("INSERT OR IGNORE INTO renders (event_id, install_id, ts, resolution, ok, exit_code, seconds, project) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)")
                .bind(&[
                    e.event_id.into(),
                    e.install_id.into(),
                    (e.ts as f64).into(),
                    e.resolution.into(),
                    e.ok.into(),
                    (e.exit_code as f64).into(),
                    e.seconds.into(),
                    e.project.into(),
                ])?
                .run()
                .await?;
            Response::ok("ok")
        })
        .get_async("/summary", |req, ctx| async move {
            if !authorized(&req, &ctx.env, "SUMMARY_TOKEN") {
                return Response::error("unauthorized", 401);
            }
            let db = ctx.env.d1("DB")?;
            let rows = db
                .prepare("SELECT install_id, resolution, COUNT(*) AS renders, ROUND(SUM(seconds), 1) AS seconds FROM renders GROUP BY install_id, resolution ORDER BY install_id, resolution")
                .all()
                .await?;
            Response::from_json(&rows.results::<serde_json::Value>()?)
        })
        .run(req, env)
        .await
}
