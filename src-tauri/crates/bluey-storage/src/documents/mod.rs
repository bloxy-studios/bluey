//! Document pipeline: parsing (PDF/DOCX/TXT/MD) → chunking → indexing (SQLite +
//! FTS5) → retrieval (keyword bm25, semantic cosine, or hybrid).

pub mod chunk;
pub mod index;
pub mod parse;
pub mod retrieve;

pub use chunk::{chunk_text, estimate_tokens, ChunkDraft};
pub use index::{add_document, reindex, DEFAULT_CHUNK_OVERLAP_TOKENS, DEFAULT_CHUNK_TOKENS};
pub use parse::{detect_format, parse_document, ParsedDocument, MAX_DOCUMENT_BYTES};
pub use retrieve::{cosine, retrieve};
