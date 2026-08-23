ALTER TABLE nodes ADD COLUMN direct_video_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE nodes ADD COLUMN child_media_branch_count INTEGER NOT NULL DEFAULT 0;
ALTER TABLE nodes ADD COLUMN total_video_count INTEGER NOT NULL DEFAULT 0;

CREATE TABLE IF NOT EXISTS metadata_bindings (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    node_id INTEGER NOT NULL,
    provider TEXT NOT NULL CHECK (provider IN ('BANGUMI')),
    provider_subject_id INTEGER NOT NULL,
    provider_title TEXT NOT NULL,
    provider_title_cn TEXT,
    provider_date TEXT,
    provider_image_url TEXT,
    bound_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    UNIQUE(node_id, provider),
    FOREIGN KEY (node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS settings (
    key TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS scan_runs (
    id TEXT NOT NULL,
    root_id INTEGER NOT NULL,
    started_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    finished_at TEXT,
    status TEXT NOT NULL CHECK (status IN ('RUNNING', 'CANCELLING', 'COMPLETED', 'CANCELLED', 'FAILED')),
    folders_scanned INTEGER NOT NULL DEFAULT 0,
    files_scanned INTEGER NOT NULL DEFAULT 0,
    errors INTEGER NOT NULL DEFAULT 0,
    message TEXT,
    PRIMARY KEY (id, root_id),
    FOREIGN KEY (root_id) REFERENCES library_roots(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_nodes_name ON nodes(display_name);
CREATE INDEX IF NOT EXISTS idx_media_files_name ON media_files(file_name);
CREATE INDEX IF NOT EXISTS idx_metadata_bindings_node ON metadata_bindings(node_id);
CREATE INDEX IF NOT EXISTS idx_scan_runs_root ON scan_runs(root_id);

CREATE TRIGGER IF NOT EXISTS trg_nodes_parent_same_root_insert
BEFORE INSERT ON nodes
WHEN NEW.parent_node_id IS NOT NULL
AND NOT EXISTS (
    SELECT 1 FROM nodes parent
    WHERE parent.id = NEW.parent_node_id
      AND parent.library_root_id = NEW.library_root_id
)
BEGIN
    SELECT RAISE(ABORT, 'parent node must belong to the same library root');
END;

CREATE TRIGGER IF NOT EXISTS trg_nodes_parent_same_root_update
BEFORE UPDATE OF parent_node_id, library_root_id ON nodes
WHEN NEW.parent_node_id IS NOT NULL
AND NOT EXISTS (
    SELECT 1 FROM nodes parent
    WHERE parent.id = NEW.parent_node_id
      AND parent.library_root_id = NEW.library_root_id
)
BEGIN
    SELECT RAISE(ABORT, 'parent node must belong to the same library root');
END;
