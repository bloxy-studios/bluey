-- ADR 0010 §2: the merged fast-path trace (`LatencyTrace` JSON) next to `ttft_ms`.
ALTER TABLE ai_requests ADD COLUMN trace TEXT;
