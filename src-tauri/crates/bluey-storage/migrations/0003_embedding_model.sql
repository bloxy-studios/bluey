-- 0003: remember which embedding model produced a document's vectors.
--
-- Vectors from different models (or different MRL sizes of gemini-embedding-2)
-- live in different spaces and must never be compared. Each document records the
-- `providerId/model` tag and the dimensionality its chunks were embedded with;
-- when the embedding assignment or `ai.embeddingDimensions` changes, documents
-- whose tag differs are re-embedded (`DocumentsManager::reembed_stale`) and
-- retrieval ignores chunks whose vector length differs from the query's.
ALTER TABLE documents ADD COLUMN embedding_model TEXT;          -- e.g. "gemini/gemini-embedding-2"
ALTER TABLE documents ADD COLUMN embedding_dimensions INTEGER;  -- e.g. 768
