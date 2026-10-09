-- 0005: mode lifecycle fixes.

-- Deleting a custom mode used to leave its files behind (`documents.scope_id`
-- has no foreign key). Purge those orphans; chunks and FTS rows cascade.
DELETE FROM documents
 WHERE scope = 'mode'
   AND scope_id IS NOT NULL
   AND scope_id NOT IN (SELECT id FROM modes);

-- Built-in modes remember a fingerprint of the shipped definition they were
-- seeded from, so a later release can refresh the ones the user never edited
-- (see `ModeRepository::seed_built_in`).
ALTER TABLE modes ADD COLUMN seed_hash TEXT;

-- Rows seeded before this column existed: every edit (and Reset to default)
-- bumps `updated_at`, so a built-in whose timestamps still match was never
-- touched. Mark it for refresh; anything else keeps the user's text.
UPDATE modes
   SET seed_hash = 'legacy-unedited'
 WHERE built_in = 1 AND created_at = updated_at;
