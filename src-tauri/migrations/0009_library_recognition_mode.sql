ALTER TABLE library_roots ADD COLUMN recognition_mode TEXT NOT NULL DEFAULT 'FOLDER'
    CHECK (recognition_mode IN ('FOLDER', 'VIDEO_FILE'));
