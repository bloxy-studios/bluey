-- Keep the standalone FTS5 tables in sync with their source tables via triggers.
-- Requires PRAGMA recursive_triggers = ON (set per-connection in db.rs) so that
-- cascading deletes and OR REPLACE also fire the delete triggers.

-- ── document_chunks → document_chunks_fts ──────────────────────────────────
CREATE TRIGGER IF NOT EXISTS trg_document_chunks_fts_insert
AFTER INSERT ON document_chunks
BEGIN
  INSERT INTO document_chunks_fts(content, heading, chunk_id, document_id)
  VALUES (new.content, coalesce(new.heading, ''), new.id, new.document_id);
END;

CREATE TRIGGER IF NOT EXISTS trg_document_chunks_fts_delete
AFTER DELETE ON document_chunks
BEGIN
  DELETE FROM document_chunks_fts WHERE chunk_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_document_chunks_fts_update
AFTER UPDATE OF content, heading ON document_chunks
BEGIN
  DELETE FROM document_chunks_fts WHERE chunk_id = old.id;
  INSERT INTO document_chunks_fts(content, heading, chunk_id, document_id)
  VALUES (new.content, coalesce(new.heading, ''), new.id, new.document_id);
END;

-- ── transcript_segments → transcript_fts (finalized segments only) ─────────
CREATE TRIGGER IF NOT EXISTS trg_transcript_fts_insert
AFTER INSERT ON transcript_segments
WHEN new.finalized = 1
BEGIN
  INSERT INTO transcript_fts(text, segment_id, session_id)
  VALUES (new.text, new.id, coalesce(new.session_id, ''));
END;

CREATE TRIGGER IF NOT EXISTS trg_transcript_fts_delete
AFTER DELETE ON transcript_segments
BEGIN
  DELETE FROM transcript_fts WHERE segment_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_transcript_fts_update
AFTER UPDATE ON transcript_segments
BEGIN
  DELETE FROM transcript_fts WHERE segment_id = old.id;
  INSERT INTO transcript_fts(text, segment_id, session_id)
  SELECT new.text, new.id, coalesce(new.session_id, '')
  WHERE new.finalized = 1;
END;

-- ── ai_responses → responses_fts ───────────────────────────────────────────
CREATE TRIGGER IF NOT EXISTS trg_responses_fts_insert
AFTER INSERT ON ai_responses
BEGIN
  INSERT INTO responses_fts(title, content, prompt, response_id, session_id)
  VALUES (coalesce(new.title, ''), new.content, coalesce(new.prompt, ''), new.id, coalesce(new.session_id, ''));
END;

CREATE TRIGGER IF NOT EXISTS trg_responses_fts_delete
AFTER DELETE ON ai_responses
BEGIN
  DELETE FROM responses_fts WHERE response_id = old.id;
END;

CREATE TRIGGER IF NOT EXISTS trg_responses_fts_update
AFTER UPDATE ON ai_responses
BEGIN
  DELETE FROM responses_fts WHERE response_id = old.id;
  INSERT INTO responses_fts(title, content, prompt, response_id, session_id)
  VALUES (coalesce(new.title, ''), new.content, coalesce(new.prompt, ''), new.id, coalesce(new.session_id, ''));
END;
