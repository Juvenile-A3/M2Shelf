CREATE TABLE IF NOT EXISTS resource_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id INTEGER NOT NULL,
    absolute_path TEXT NOT NULL UNIQUE COLLATE NOCASE,
    file_name TEXT NOT NULL,
    extension TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    modified_at TEXT NOT NULL,
    resource_type TEXT NOT NULL DEFAULT 'OTHER'
        CHECK (resource_type IN (
            'DOCUMENT', 'IMAGE', 'AUDIO', 'SUBTITLE',
            'ARCHIVE', 'FONT', 'PLAYLIST', 'OTHER'
        )),
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_resource_files_node ON resource_files(node_id);
CREATE INDEX IF NOT EXISTS idx_resource_files_name ON resource_files(file_name);

ALTER TABLE metadata_bindings ADD COLUMN cover_download_error TEXT;
