ALTER TABLE virtual_folders ADD COLUMN only_flagged INTEGER NOT NULL DEFAULT 0;
ALTER TABLE virtual_folders ADD COLUMN only_attachments INTEGER NOT NULL DEFAULT 0;
