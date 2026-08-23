CREATE TABLE IF NOT EXISTS favorite_folders (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    normalized_name TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE TABLE IF NOT EXISTS node_favorite_folders (
    folder_id INTEGER NOT NULL,
    node_id INTEGER NOT NULL,
    added_at TEXT NOT NULL
        DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    PRIMARY KEY (folder_id, node_id),
    FOREIGN KEY (folder_id) REFERENCES favorite_folders(id) ON DELETE CASCADE,
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_node_favorite_folders_node
    ON node_favorite_folders(node_id);

CREATE INDEX IF NOT EXISTS idx_node_favorite_folders_folder_added
    ON node_favorite_folders(folder_id, added_at DESC, node_id DESC);
