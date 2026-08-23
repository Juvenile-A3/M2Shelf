CREATE TABLE IF NOT EXISTS watch_history (
    node_id INTEGER PRIMARY KEY,
    last_watched_at TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    watch_count INTEGER NOT NULL DEFAULT 1 CHECK (watch_count > 0),
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_watch_history_last_watched
    ON watch_history(last_watched_at DESC);
