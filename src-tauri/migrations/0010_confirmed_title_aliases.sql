CREATE TABLE IF NOT EXISTS confirmed_title_aliases (
    normalized_alias TEXT NOT NULL CHECK (
        length(normalized_alias) BETWEEN 1 AND 200
    ),
    original_alias TEXT NOT NULL CHECK (
        length(original_alias) BETWEEN 1 AND 200
    ),
    subject_id INTEGER NOT NULL,
    subject_type INTEGER NOT NULL CHECK (subject_type IN (2, 6)),
    source_node_id INTEGER NOT NULL,
    confirmed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (source_node_id, normalized_alias),
    FOREIGN KEY (source_node_id) REFERENCES nodes(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_confirmed_title_aliases_normalized
ON confirmed_title_aliases(normalized_alias);
