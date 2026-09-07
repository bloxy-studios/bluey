-- Bluey SQLite schema v1.
-- Conventions: ids are text (prefix_uuid), timestamps are RFC 3339 UTC strings,
-- JSON columns hold serde-serialized bluey_core types (camelCase).
-- Never store API keys, auth tokens or raw audio here (keychain / never).

PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS schema_migrations (
  name        TEXT PRIMARY KEY,
  applied_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS users (
  id          TEXT PRIMARY KEY,          -- Clerk user id
  email       TEXT,
  first_name  TEXT,
  last_name   TEXT,
  image_url   TEXT,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS settings (
  key         TEXT PRIMARY KEY,          -- "settings" (whole blob), "panel_state", "active_mode_id"
  value       TEXT NOT NULL,             -- JSON
  updated_at  TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS modes (
  id                   TEXT PRIMARY KEY,
  name                 TEXT NOT NULL,
  description          TEXT NOT NULL DEFAULT '',
  icon                 TEXT NOT NULL DEFAULT 'sparkles',
  system_instructions  TEXT NOT NULL DEFAULT '',
  response_schema      TEXT NOT NULL DEFAULT 'answer',
  preferred_latency    TEXT NOT NULL DEFAULT 'fast',
  context_requirements TEXT NOT NULL DEFAULT '[]',   -- JSON array
  built_in             INTEGER NOT NULL DEFAULT 0,
  group_name           TEXT,
  response_style       TEXT,                          -- JSON ResponseStylePatch
  preferred_model_role TEXT,
  sort_order           INTEGER NOT NULL DEFAULT 0,
  created_at           TEXT NOT NULL,
  updated_at           TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS sessions (
  id          TEXT PRIMARY KEY,
  mode_id     TEXT NOT NULL REFERENCES modes(id) ON DELETE SET DEFAULT DEFAULT 'general',
  title       TEXT,
  status      TEXT NOT NULL CHECK (status IN ('active','paused','completed')),
  started_at  TEXT NOT NULL,
  ended_at    TEXT,
  metadata    TEXT,                                   -- JSON object
  created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_sessions_started_at ON sessions(started_at DESC);
CREATE INDEX IF NOT EXISTS idx_sessions_mode_id ON sessions(mode_id);
CREATE INDEX IF NOT EXISTS idx_sessions_status ON sessions(status);

CREATE TABLE IF NOT EXISTS documents (
  id            TEXT PRIMARY KEY,
  title         TEXT NOT NULL,
  kind          TEXT NOT NULL,
  format        TEXT NOT NULL,
  scope         TEXT NOT NULL CHECK (scope IN ('global','session','mode')),
  scope_id      TEXT,                                 -- session id or mode id
  source_path   TEXT,
  content       TEXT NOT NULL,                        -- normalized full text
  size_bytes    INTEGER NOT NULL DEFAULT 0,
  chunk_count   INTEGER NOT NULL DEFAULT 0,
  index_status  TEXT NOT NULL DEFAULT 'pending',
  has_embeddings INTEGER NOT NULL DEFAULT 0,
  metadata      TEXT,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_documents_scope ON documents(scope, scope_id);
CREATE INDEX IF NOT EXISTS idx_documents_kind ON documents(kind);

-- Mode ⇄ document attachments (a document can also be attached to several modes).
CREATE TABLE IF NOT EXISTS mode_documents (
  mode_id      TEXT NOT NULL REFERENCES modes(id) ON DELETE CASCADE,
  document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  created_at   TEXT NOT NULL,
  PRIMARY KEY (mode_id, document_id)
);

CREATE TABLE IF NOT EXISTS document_chunks (
  id           TEXT PRIMARY KEY,
  document_id  TEXT NOT NULL REFERENCES documents(id) ON DELETE CASCADE,
  chunk_index  INTEGER NOT NULL,
  content      TEXT NOT NULL,
  tokens       INTEGER NOT NULL DEFAULT 0,
  heading      TEXT,
  embedding    BLOB,                                  -- little-endian f32 vector, nullable
  created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_document_chunks_document ON document_chunks(document_id, chunk_index);

CREATE VIRTUAL TABLE IF NOT EXISTS document_chunks_fts USING fts5(
  content,
  heading,
  chunk_id UNINDEXED,
  document_id UNINDEXED,
  tokenize = 'porter unicode61'
);

CREATE TABLE IF NOT EXISTS speakers (
  id           TEXT PRIMARY KEY,
  session_id   TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  label        TEXT NOT NULL,                         -- "You", "Interviewer", "Speaker 2"
  source       TEXT,                                  -- microphone | system
  confidence   REAL,
  created_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_speakers_session ON speakers(session_id);

CREATE TABLE IF NOT EXISTS transcript_segments (
  id                  TEXT PRIMARY KEY,
  session_id          TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  speaker             TEXT,
  speaker_confidence  REAL,
  source              TEXT NOT NULL CHECK (source IN ('microphone','system')),
  text                TEXT NOT NULL,
  start_time_ms       INTEGER NOT NULL,
  end_time_ms         INTEGER NOT NULL,
  confidence          REAL,
  finalized           INTEGER NOT NULL DEFAULT 1,
  language            TEXT,
  created_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_transcript_session_time ON transcript_segments(session_id, start_time_ms);
CREATE INDEX IF NOT EXISTS idx_transcript_created_at ON transcript_segments(created_at);

CREATE VIRTUAL TABLE IF NOT EXISTS transcript_fts USING fts5(
  text,
  segment_id UNINDEXED,
  session_id UNINDEXED,
  tokenize = 'porter unicode61'
);

CREATE TABLE IF NOT EXISTS screen_snapshots (
  id            TEXT PRIMARY KEY,
  session_id    TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  display_id    TEXT,
  width         INTEGER NOT NULL,
  height        INTEGER NOT NULL,
  mime_type     TEXT NOT NULL,
  image_path    TEXT,                                 -- only when privacy.storeScreenshots = true
  hash          TEXT,
  ocr_text      TEXT,                                 -- joined OCR text (may be null)
  ocr_json      TEXT,                                 -- OCRContext JSON
  active_app    TEXT,                                 -- ApplicationContext JSON
  active_window TEXT,                                 -- WindowContext JSON
  captured_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_screen_snapshots_session ON screen_snapshots(session_id, captured_at);

CREATE TABLE IF NOT EXISTS accessibility_snapshots (
  id            TEXT PRIMARY KEY,
  session_id    TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  snapshot_json TEXT NOT NULL,                        -- AccessibilityContext JSON (bounded)
  captured_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ax_snapshots_session ON accessibility_snapshots(session_id, captured_at);

CREATE TABLE IF NOT EXISTS ai_requests (
  id              TEXT PRIMARY KEY,                   -- requestId
  session_id      TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  task            TEXT NOT NULL,
  provider_id     TEXT,
  model           TEXT,
  latency_budget  TEXT,
  context_tokens  INTEGER,
  input_tokens    INTEGER,
  output_tokens   INTEGER,
  ttft_ms         INTEGER,
  total_ms        INTEGER,
  finish_reason   TEXT,
  error_code      TEXT,
  created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ai_requests_session ON ai_requests(session_id, created_at);

CREATE TABLE IF NOT EXISTS ai_responses (
  id             TEXT PRIMARY KEY,
  request_id     TEXT NOT NULL,
  session_id     TEXT REFERENCES sessions(id) ON DELETE CASCADE,
  mode_id        TEXT NOT NULL,
  response_type  TEXT NOT NULL,
  title          TEXT,
  content        TEXT NOT NULL,
  prompt         TEXT,
  response_json  TEXT NOT NULL,                       -- full BlueyResponse JSON
  prepared       INTEGER NOT NULL DEFAULT 0,
  created_at     TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ai_responses_session ON ai_responses(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_ai_responses_request ON ai_responses(request_id);

CREATE VIRTUAL TABLE IF NOT EXISTS responses_fts USING fts5(
  title,
  content,
  prompt,
  response_id UNINDEXED,
  session_id UNINDEXED,
  tokenize = 'porter unicode61'
);

CREATE TABLE IF NOT EXISTS response_feedback (
  response_id  TEXT PRIMARY KEY REFERENCES ai_responses(id) ON DELETE CASCADE,
  rating       TEXT NOT NULL CHECK (rating IN ('up','down')),
  categories   TEXT,                                  -- JSON array
  comment      TEXT,
  created_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS session_events (
  id          TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  type        TEXT NOT NULL,
  title       TEXT NOT NULL,
  detail      TEXT,
  refs        TEXT,                                   -- JSON object of related ids
  confidence  REAL,
  created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_events_session ON session_events(session_id, created_at);
CREATE INDEX IF NOT EXISTS idx_session_events_type ON session_events(type);

CREATE TABLE IF NOT EXISTS session_notes (
  id          TEXT PRIMARY KEY,
  session_id  TEXT NOT NULL REFERENCES sessions(id) ON DELETE CASCADE,
  content     TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  updated_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_session_notes_session ON session_notes(session_id);

CREATE TABLE IF NOT EXISTS session_summaries (
  id            TEXT PRIMARY KEY,
  session_id    TEXT NOT NULL UNIQUE REFERENCES sessions(id) ON DELETE CASCADE,
  mode_id       TEXT NOT NULL,
  summary_json  TEXT NOT NULL,                        -- SessionSummary JSON
  created_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS shortcuts (
  id                   TEXT PRIMARY KEY,              -- ShortcutId
  accelerator          TEXT NOT NULL,
  enabled              INTEGER NOT NULL DEFAULT 1,
  updated_at           TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS model_configs (
  id            TEXT PRIMARY KEY,                     -- provider id
  kind          TEXT NOT NULL,
  name          TEXT NOT NULL,
  base_url      TEXT NOT NULL,
  api_version   TEXT,
  deployments   TEXT,                                 -- JSON map
  enabled       INTEGER NOT NULL DEFAULT 1,
  created_at    TEXT NOT NULL,
  updated_at    TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS ai_cache (
  key         TEXT PRIMARY KEY,                       -- hash of (task, model, prompt)
  value       TEXT NOT NULL,
  created_at  TEXT NOT NULL,
  expires_at  TEXT
);
CREATE INDEX IF NOT EXISTS idx_ai_cache_expires ON ai_cache(expires_at);
