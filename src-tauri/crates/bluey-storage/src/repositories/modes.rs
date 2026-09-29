//! Modes (built-in + user defined) and their files (mode-scoped documents).

use std::collections::HashMap;

use bluey_core::error::BlueyError;
use bluey_core::types::mode::{BlueyMode, ModePatch, ModelRole};
use bluey_core::{new_id, now_iso};
use rusqlite::{params, Connection, OptionalExtension, Row};

use super::{
    from_enum_str, from_json_str, not_found, opt_from_json, opt_to_json, to_enum_str,
    to_json_string,
};
use crate::db::Database;
use crate::error::SqlExt;

struct ModeRow {
    id: String,
    name: String,
    description: String,
    icon: String,
    system_instructions: String,
    response_schema: String,
    preferred_latency: String,
    context_requirements: String,
    built_in: bool,
    group_name: Option<String>,
    response_style: Option<String>,
    preferred_model_role: Option<String>,
    created_at: String,
    updated_at: String,
}

impl ModeRow {
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            id: row.get(0)?,
            name: row.get(1)?,
            description: row.get(2)?,
            icon: row.get(3)?,
            system_instructions: row.get(4)?,
            response_schema: row.get(5)?,
            preferred_latency: row.get(6)?,
            context_requirements: row.get(7)?,
            built_in: row.get(8)?,
            group_name: row.get(9)?,
            response_style: row.get(10)?,
            preferred_model_role: row.get(11)?,
            created_at: row.get(12)?,
            updated_at: row.get(13)?,
        })
    }

    fn into_mode(self, attached_document_ids: Vec<String>) -> Result<BlueyMode, BlueyError> {
        Ok(BlueyMode {
            id: self.id,
            name: self.name,
            description: self.description,
            icon: self.icon,
            system_instructions: self.system_instructions,
            response_schema: from_enum_str(&self.response_schema)?,
            preferred_latency: from_enum_str(&self.preferred_latency)?,
            context_requirements: from_json_str(&self.context_requirements)?,
            built_in: self.built_in,
            group: self.group_name,
            response_style: opt_from_json(self.response_style)?,
            preferred_model_role: self
                .preferred_model_role
                .as_deref()
                .map(from_enum_str)
                .transpose()?,
            attached_document_ids,
            created_at: self.created_at,
            updated_at: self.updated_at,
        })
    }
}

const MODE_COLS: &str = "id, name, description, icon, system_instructions, response_schema,
    preferred_latency, context_requirements, built_in, group_name, response_style,
    preferred_model_role, created_at, updated_at";

/// The user-editable columns of a mode, as stored.
const EDITABLE_COLS: &str = "name, description, icon, system_instructions, response_schema,
    preferred_latency, context_requirements, group_name, response_style, preferred_model_role";

/// `seed_hash` of built-ins seeded before the column existed and never edited
/// since (set by migration 0005): refreshed on the next seed.
const LEGACY_UNEDITED: &str = "legacy-unedited";

/// A mode's editable columns in their stored (string) form — what gets
/// written, and what the seed fingerprint covers.
struct EditableColumns {
    name: String,
    description: String,
    icon: String,
    system_instructions: String,
    response_schema: String,
    preferred_latency: String,
    context_requirements: String,
    group_name: Option<String>,
    response_style: Option<String>,
    preferred_model_role: Option<String>,
}

impl EditableColumns {
    fn of(mode: &BlueyMode) -> Result<Self, BlueyError> {
        Ok(Self {
            name: mode.name.clone(),
            description: mode.description.clone(),
            icon: mode.icon.clone(),
            system_instructions: mode.system_instructions.clone(),
            response_schema: to_enum_str(&mode.response_schema)?,
            preferred_latency: to_enum_str(&mode.preferred_latency)?,
            context_requirements: to_json_string(&mode.context_requirements)?,
            group_name: mode.group.clone(),
            response_style: opt_to_json(&mode.response_style)?,
            preferred_model_role: mode
                .preferred_model_role
                .as_ref()
                .map(to_enum_str)
                .transpose()?,
        })
    }

    /// Reads a row selected with [`EDITABLE_COLS`].
    fn read(row: &Row<'_>) -> rusqlite::Result<Self> {
        Ok(Self {
            name: row.get(0)?,
            description: row.get(1)?,
            icon: row.get(2)?,
            system_instructions: row.get(3)?,
            response_schema: row.get(4)?,
            preferred_latency: row.get(5)?,
            context_requirements: row.get(6)?,
            group_name: row.get(7)?,
            response_style: row.get(8)?,
            preferred_model_role: row.get(9)?,
        })
    }

    /// Stable change-detection fingerprint (64-bit FNV-1a over the fields;
    /// not a security hash): equal columns always give an equal fingerprint.
    fn fingerprint(&self) -> String {
        let fields = [
            Some(self.name.as_str()),
            Some(self.description.as_str()),
            Some(self.icon.as_str()),
            Some(self.system_instructions.as_str()),
            Some(self.response_schema.as_str()),
            Some(self.preferred_latency.as_str()),
            Some(self.context_requirements.as_str()),
            self.group_name.as_deref(),
            self.response_style.as_deref(),
            self.preferred_model_role.as_deref(),
        ];
        let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
        for field in fields {
            // 0x1e marks a missing value, 0x1f ends each field.
            let bytes = field.map_or(&[0x1e_u8][..], str::as_bytes);
            for &byte in bytes.iter().chain(&[0x1f]) {
                hash ^= u64::from(byte);
                hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
            }
        }
        format!("fnv1a64:{hash:016x}")
    }
}

/// CRUD, seeding and document attachments for the `modes` table.
pub struct ModeRepository;

impl ModeRepository {
    /// Insert missing built-in modes and refresh the `built_in` flag and
    /// `sort_order` of existing ones (`sort_order` follows the slice order).
    /// A built-in the user never edited — its columns still match the
    /// `seed_hash` fingerprint recorded when it was seeded — is refreshed to
    /// the shipped definition; an edited one keeps the user's text (Reset to
    /// default restores the shipped definition).
    pub fn seed_built_in(db: &Database, modes: &[BlueyMode]) -> Result<(), BlueyError> {
        let shipped = modes
            .iter()
            .map(EditableColumns::of)
            .collect::<Result<Vec<_>, _>>()?;
        let now = now_iso();
        db.transaction(|conn| {
            for (sort_order, (mode, cols)) in modes.iter().zip(&shipped).enumerate() {
                let seed_hash = cols.fingerprint();
                let existing = conn
                    .query_row(
                        &format!("SELECT {EDITABLE_COLS}, seed_hash FROM modes WHERE id = ?1"),
                        [&mode.id],
                        |r| Ok((EditableColumns::read(r)?, r.get::<_, Option<String>>(10)?)),
                    )
                    .optional()
                    .sql()?;
                match existing {
                    None => {
                        let row = BlueyMode {
                            built_in: true,
                            created_at: now.clone(),
                            updated_at: now.clone(),
                            ..mode.clone()
                        };
                        insert_row(conn, &row, Some(&seed_hash))?;
                    }
                    Some((current, stored)) => {
                        let unedited = stored
                            .as_deref()
                            .is_some_and(|s| s == LEGACY_UNEDITED || s == current.fingerprint());
                        if unedited && stored.as_deref() != Some(seed_hash.as_str()) {
                            write_editable(conn, &mode.id, cols, &now)?;
                            set_seed_hash(conn, &mode.id, &seed_hash)?;
                        }
                    }
                }
                conn.execute(
                    "UPDATE modes SET built_in = 1, sort_order = ?2 WHERE id = ?1",
                    params![mode.id, sort_order as i64],
                )
                .sql()?;
            }
            Ok(())
        })
    }

    /// All modes ordered by `sort_order`, then name, with attachments populated.
    pub fn list(db: &Database) -> Result<Vec<BlueyMode>, BlueyError> {
        let (rows, attachments) = db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(&format!(
                    "SELECT {MODE_COLS} FROM modes ORDER BY sort_order, name"
                ))
                .sql()?;
            let rows = stmt
                .query_map([], ModeRow::read)
                .sql()?
                .collect::<Result<Vec<_>, _>>()
                .sql()?;
            let mut stmt = conn
                .prepare(
                    "SELECT scope_id, id FROM documents
                      WHERE scope = 'mode' AND scope_id IS NOT NULL
                      ORDER BY created_at, id",
                )
                .sql()?;
            let pairs = stmt
                .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
                .sql()?
                .collect::<Result<Vec<_>, _>>()
                .sql()?;
            Ok((rows, pairs))
        })?;
        let mut by_mode: HashMap<String, Vec<String>> = HashMap::new();
        for (mode_id, doc_id) in attachments {
            by_mode.entry(mode_id).or_default().push(doc_id);
        }
        rows.into_iter()
            .map(|row| {
                let attached = by_mode.remove(&row.id).unwrap_or_default();
                row.into_mode(attached)
            })
            .collect()
    }

    /// Fetch one mode (`storage.not_found` when missing).
    pub fn get(db: &Database, id: &str) -> Result<BlueyMode, BlueyError> {
        let row = db.with_conn(|conn| {
            conn.query_row(
                &format!("SELECT {MODE_COLS} FROM modes WHERE id = ?1"),
                [id],
                ModeRow::read,
            )
            .optional()
            .sql()
        })?;
        let row = row.ok_or_else(|| not_found("mode", id))?;
        let attached = Self::attached_document_ids(db, id)?;
        row.into_mode(attached)
    }

    /// Create a custom mode from a draft. `name` is required; see
    /// [`validate_patch`] for the limits.
    pub fn create(db: &Database, patch: &ModePatch) -> Result<BlueyMode, BlueyError> {
        validate_patch(patch)?;
        let name = patch
            .name
            .as_deref()
            .map(str::trim)
            .filter(|n| !n.is_empty())
            .ok_or_else(|| BlueyError::invalid_params("mode name is required"))?;
        let now = now_iso();
        let mode = BlueyMode {
            id: new_id("mode"),
            name: name.to_string(),
            description: patch.description.clone().unwrap_or_default(),
            icon: patch.icon.clone().unwrap_or_else(|| "sparkles".into()),
            system_instructions: patch.system_instructions.clone().unwrap_or_default(),
            response_schema: patch
                .response_schema
                .unwrap_or(bluey_core::types::mode::ResponseSchemaId::Answer),
            preferred_latency: patch
                .preferred_latency
                .unwrap_or(bluey_core::types::mode::PreferredLatency::Fast),
            context_requirements: patch.context_requirements.clone().unwrap_or_default(),
            built_in: false,
            group: patch.group.as_ref().and_then(|g| clean_group(g.as_deref())),
            response_style: patch.response_style.clone(),
            preferred_model_role: patch.preferred_model_role.flatten(),
            attached_document_ids: vec![],
            created_at: now.clone(),
            updated_at: now,
        };
        db.with_conn(|conn| insert_row(conn, &mode, None))?;
        Ok(mode)
    }

    /// Apply a partial update and return the updated mode (built-ins may be
    /// edited; [`ModeRepository::reset_built_in`] restores the original).
    /// A `null` group or model role clears it.
    pub fn update(db: &Database, id: &str, patch: &ModePatch) -> Result<BlueyMode, BlueyError> {
        validate_patch(patch)?;
        let mut mode = Self::get(db, id)?;
        if let Some(name) = &patch.name {
            let name = name.trim();
            if name.is_empty() {
                return Err(BlueyError::invalid_params("mode name cannot be empty"));
            }
            mode.name = name.to_string();
        }
        if let Some(v) = &patch.description {
            mode.description = v.clone();
        }
        if let Some(v) = &patch.icon {
            mode.icon = v.clone();
        }
        if let Some(v) = &patch.system_instructions {
            mode.system_instructions = v.clone();
        }
        if let Some(v) = patch.response_schema {
            mode.response_schema = v;
        }
        if let Some(v) = patch.preferred_latency {
            mode.preferred_latency = v;
        }
        if let Some(v) = &patch.context_requirements {
            mode.context_requirements = v.clone();
        }
        if let Some(group) = &patch.group {
            mode.group = clean_group(group.as_deref());
        }
        if let Some(v) = &patch.response_style {
            mode.response_style = Some(v.clone());
        }
        if let Some(role) = patch.preferred_model_role {
            mode.preferred_model_role = role;
        }
        mode.updated_at = now_iso();
        Self::write_columns(db, &mode)?;
        Ok(mode)
    }

    fn write_columns(db: &Database, mode: &BlueyMode) -> Result<(), BlueyError> {
        let cols = EditableColumns::of(mode)?;
        db.with_conn(|conn| write_editable(conn, &mode.id, &cols, &mode.updated_at))
    }

    /// Delete a custom mode together with its files (mode-scoped documents;
    /// their chunks and FTS rows cascade) in one transaction. Built-in modes
    /// are refused with `internal.invalid_params`; sessions that referenced
    /// the mode fall back to `general` (schema `ON DELETE SET DEFAULT`).
    pub fn delete(db: &Database, id: &str) -> Result<(), BlueyError> {
        let mode = Self::get(db, id)?;
        if mode.built_in {
            return Err(BlueyError::invalid_params(format!(
                "built-in mode '{id}' cannot be deleted"
            )));
        }
        db.transaction(|conn| {
            conn.execute(
                "DELETE FROM documents WHERE scope = 'mode' AND scope_id = ?1",
                [id],
            )
            .sql()?;
            conn.execute("DELETE FROM modes WHERE id = ?1", [id])
                .sql()?;
            Ok(())
        })
    }

    /// Copy a mode (name suffixed with " (Copy)", `built_in = false`) together
    /// with its files: each mode-scoped document is copied to the new mode
    /// with its chunks (FTS rows via trigger) and embeddings, in one
    /// transaction.
    pub fn duplicate(db: &Database, id: &str) -> Result<BlueyMode, BlueyError> {
        let source = Self::get(db, id)?;
        let now = now_iso();
        let mut copy = BlueyMode {
            id: new_id("mode"),
            name: format!("{} (Copy)", source.name),
            built_in: false,
            attached_document_ids: vec![],
            created_at: now.clone(),
            updated_at: now,
            ..source
        };
        copy.attached_document_ids = db.transaction(|conn| {
            insert_row(conn, &copy, None)?;
            copy_mode_documents(conn, id, &copy.id)
        })?;
        Ok(copy)
    }

    /// Restore a built-in mode to its shipped definition (attachments are kept)
    /// and mark it unedited.
    pub fn reset_built_in(
        db: &Database,
        id: &str,
        original: &BlueyMode,
    ) -> Result<BlueyMode, BlueyError> {
        let current = Self::get(db, id)?;
        if !current.built_in {
            return Err(BlueyError::invalid_params(format!(
                "mode '{id}' is not built-in"
            )));
        }
        if original.id != id {
            return Err(BlueyError::invalid_params(
                "original definition does not match the mode id",
            ));
        }
        let restored = BlueyMode {
            built_in: true,
            attached_document_ids: current.attached_document_ids.clone(),
            created_at: current.created_at.clone(),
            updated_at: now_iso(),
            ..original.clone()
        };
        // The row matches the shipped definition again, so later releases may
        // refresh it (see `seed_built_in`).
        let cols = EditableColumns::of(&restored)?;
        db.transaction(|conn| {
            write_editable(conn, id, &cols, &restored.updated_at)?;
            set_seed_hash(conn, id, &cols.fingerprint())
        })?;
        Ok(restored)
    }

    /// Ids of a mode's files — the documents scoped to it
    /// (`scope = 'mode'`, `scope_id = mode_id`), oldest first. The document
    /// scope is the single source of truth for mode attachments (the legacy
    /// `mode_documents` join table is no longer read or written).
    pub fn attached_document_ids(db: &Database, mode_id: &str) -> Result<Vec<String>, BlueyError> {
        db.with_conn(|conn| mode_document_ids(conn, mode_id))
    }
}

/// Longest accepted mode text, in characters (the mode editor enforces the
/// same limits through `validateModeDraft`).
const NAME_MAX_CHARS: usize = 60;
const DESCRIPTION_MAX_CHARS: usize = 300;
const INSTRUCTIONS_MAX_CHARS: usize = 4000;

/// Model roles a mode may prefer: the ones that generate answers.
const MODE_MODEL_ROLES: [ModelRole; 5] = [
    ModelRole::Default,
    ModelRole::Fast,
    ModelRole::Reasoning,
    ModelRole::Vision,
    ModelRole::Research,
];

/// Reject a draft/patch that breaks the mode limits (`internal.invalid_params`).
fn validate_patch(patch: &ModePatch) -> Result<(), BlueyError> {
    let check_len = |field: &str, value: Option<&str>, max: usize| match value {
        Some(v) if v.chars().count() > max => Err(BlueyError::invalid_params(format!(
            "mode {field} must be {max} characters or fewer"
        ))),
        _ => Ok(()),
    };
    check_len("name", patch.name.as_deref().map(str::trim), NAME_MAX_CHARS)?;
    check_len(
        "description",
        patch.description.as_deref(),
        DESCRIPTION_MAX_CHARS,
    )?;
    check_len(
        "instructions",
        patch.system_instructions.as_deref(),
        INSTRUCTIONS_MAX_CHARS,
    )?;
    if let Some(icon) = &patch.icon {
        if !is_kebab_case(icon) {
            return Err(BlueyError::invalid_params(
                "mode icon must be a kebab-case icon name",
            ));
        }
    }
    if let Some(Some(role)) = patch.preferred_model_role {
        if !MODE_MODEL_ROLES.contains(&role) {
            return Err(BlueyError::invalid_params(format!(
                "a mode cannot prefer the {} model",
                to_enum_str(&role)?
            )));
        }
    }
    Ok(())
}

/// `lucide` icon names: lowercase ASCII words joined by single hyphens.
fn is_kebab_case(name: &str) -> bool {
    !name.is_empty()
        && name.split('-').all(|word| {
            !word.is_empty()
                && word
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
        })
}

/// A blank sidebar group means "no group".
fn clean_group(group: Option<&str>) -> Option<String> {
    group
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .map(str::to_string)
}

/// Overwrite a mode's editable columns (`storage.not_found` when missing).
fn write_editable(
    conn: &Connection,
    id: &str,
    cols: &EditableColumns,
    updated_at: &str,
) -> Result<(), BlueyError> {
    let changed = conn
        .execute(
            "UPDATE modes SET name = ?2, description = ?3, icon = ?4, system_instructions = ?5,
                response_schema = ?6, preferred_latency = ?7, context_requirements = ?8,
                group_name = ?9, response_style = ?10, preferred_model_role = ?11, updated_at = ?12
             WHERE id = ?1",
            params![
                id,
                cols.name,
                cols.description,
                cols.icon,
                cols.system_instructions,
                cols.response_schema,
                cols.preferred_latency,
                cols.context_requirements,
                cols.group_name,
                cols.response_style,
                cols.preferred_model_role,
                updated_at
            ],
        )
        .sql()?;
    if changed == 0 {
        return Err(not_found("mode", id));
    }
    Ok(())
}

/// Record the fingerprint of the shipped definition a built-in now matches.
fn set_seed_hash(conn: &Connection, id: &str, seed_hash: &str) -> Result<(), BlueyError> {
    conn.execute(
        "UPDATE modes SET seed_hash = ?2 WHERE id = ?1",
        params![id, seed_hash],
    )
    .sql()?;
    Ok(())
}

/// Insert a full mode row, appended after the existing modes. `seed_hash`
/// is the shipped-definition fingerprint for built-ins, `None` otherwise.
fn insert_row(
    conn: &Connection,
    mode: &BlueyMode,
    seed_hash: Option<&str>,
) -> Result<(), BlueyError> {
    let cols = EditableColumns::of(mode)?;
    let next_order: i64 = conn
        .query_row(
            "SELECT coalesce(max(sort_order), -1) + 1 FROM modes",
            [],
            |r| r.get(0),
        )
        .sql()?;
    conn.execute(
        "INSERT INTO modes (id, name, description, icon, system_instructions,
            response_schema, preferred_latency, context_requirements, built_in,
            group_name, response_style, preferred_model_role, sort_order, created_at, updated_at,
            seed_hash)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)",
        params![
            mode.id,
            cols.name,
            cols.description,
            cols.icon,
            cols.system_instructions,
            cols.response_schema,
            cols.preferred_latency,
            cols.context_requirements,
            mode.built_in,
            cols.group_name,
            cols.response_style,
            cols.preferred_model_role,
            next_order,
            mode.created_at,
            mode.updated_at,
            seed_hash
        ],
    )
    .sql()?;
    Ok(())
}

fn mode_document_ids(conn: &Connection, mode_id: &str) -> Result<Vec<String>, BlueyError> {
    let mut stmt = conn
        .prepare(
            "SELECT id FROM documents
              WHERE scope = 'mode' AND scope_id = ?1 ORDER BY created_at, id",
        )
        .sql()?;
    let rows = stmt.query_map([mode_id], |r| r.get(0)).sql()?;
    rows.collect::<Result<Vec<String>, _>>().sql()
}

/// Copy every document scoped to mode `from` (row, chunks with their
/// embeddings; FTS rows follow via trigger) to mode `to` under new ids.
/// Returns the new document ids in the source order.
fn copy_mode_documents(conn: &Connection, from: &str, to: &str) -> Result<Vec<String>, BlueyError> {
    let now = now_iso();
    let mut copied = Vec::new();
    for source_doc in mode_document_ids(conn, from)? {
        let doc_id = new_id("doc");
        conn.execute(
            "INSERT INTO documents (id, title, kind, format, scope, scope_id, source_path,
                content, size_bytes, chunk_count, index_status, has_embeddings, metadata,
                embedding_model, embedding_dimensions, created_at, updated_at)
             SELECT ?1, title, kind, format, scope, ?2, source_path,
                content, size_bytes, chunk_count, index_status, has_embeddings, metadata,
                embedding_model, embedding_dimensions, ?3, ?3
               FROM documents WHERE id = ?4",
            params![doc_id, to, now, source_doc],
        )
        .sql()?;
        let chunk_ids: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT id FROM document_chunks WHERE document_id = ?1 ORDER BY chunk_index",
                )
                .sql()?;
            let rows = stmt.query_map([&source_doc], |r| r.get(0)).sql()?;
            rows.collect::<Result<_, _>>().sql()?
        };
        for chunk_id in chunk_ids {
            conn.execute(
                "INSERT INTO document_chunks (id, document_id, chunk_index, content, tokens,
                    heading, embedding, created_at)
                 SELECT ?1, ?2, chunk_index, content, tokens, heading, embedding, ?3
                   FROM document_chunks WHERE id = ?4",
                params![new_id("chk"), doc_id, now, chunk_id],
            )
            .sql()?;
        }
        copied.push(doc_id);
    }
    Ok(copied)
}

#[cfg(test)]
#[path = "modes_tests.rs"]
mod tests;
