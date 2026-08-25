ALTER TABLE metadata_bindings
ADD COLUMN provider_subject_type INTEGER NOT NULL DEFAULT 2
CHECK (provider_subject_type IN (2, 6));
