PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS library_roots (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    path TEXT NOT NULL UNIQUE COLLATE NOCASE,
    display_name TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_scan_at TEXT
);

CREATE TABLE IF NOT EXISTS nodes (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    library_root_id INTEGER NOT NULL,
    parent_node_id INTEGER,
    absolute_path TEXT NOT NULL UNIQUE COLLATE NOCASE,
    folder_name TEXT NOT NULL,
    display_name TEXT NOT NULL,
    node_type TEXT NOT NULL DEFAULT 'CONTAINER'
        CHECK (node_type IN ('AUTO_WORK', 'WORK', 'CONTAINER', 'MIXED', 'IGNORED')),
    manual_type_override INTEGER NOT NULL DEFAULT 0
        CHECK (manual_type_override IN (0, 1)),
    cover_source TEXT NOT NULL DEFAULT 'PLACEHOLDER'
        CHECK (cover_source IN ('BANGUMI', 'MANUAL', 'PLACEHOLDER')),
    cover_cache_path TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (library_root_id) REFERENCES library_roots(id) ON DELETE CASCADE,
    FOREIGN KEY (parent_node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS media_files (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id INTEGER NOT NULL,
    absolute_path TEXT NOT NULL UNIQUE COLLATE NOCASE,
    file_name TEXT NOT NULL,
    extension TEXT NOT NULL,
    file_size INTEGER NOT NULL CHECK (file_size >= 0),
    modified_at TEXT NOT NULL,
    duration_ms INTEGER,
    width INTEGER,
    height INTEGER,
    codec TEXT,
    last_seen_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nodes_library_root ON nodes(library_root_id);
CREATE INDEX IF NOT EXISTS idx_nodes_parent ON nodes(parent_node_id);
CREATE INDEX IF NOT EXISTS idx_media_files_node ON media_files(node_id);
