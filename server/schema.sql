CREATE TABLE IF NOT EXISTS renders (
    event_id TEXT PRIMARY KEY,
    install_id TEXT NOT NULL,
    ts INTEGER NOT NULL,
    resolution TEXT NOT NULL,
    ok INTEGER NOT NULL,
    exit_code INTEGER NOT NULL,
    seconds REAL NOT NULL,
    project TEXT NOT NULL
);
