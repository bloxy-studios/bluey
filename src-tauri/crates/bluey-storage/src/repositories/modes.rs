//! Modes (built-in + user defined) and mode ⇄ document attachments.

use std::collections::HashMap;

use bluey_core::error::BlueyError;
use bluey_core::types::mode::{BlueyMode, ModePatch};
use bluey_core::{new_id, now_iso};
use rusqlite::{params, OptionalExtension, Row};

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

/// CRUD, seeding and document attachments for the `modes` table.
pub struct ModeRepository;

impl ModeRepository {
    /// Insert missing built-in modes and refresh the `built_in` flag and
    /// `sort_order` of existing ones — user edits to any other column are
    /// preserved. `sort_order` follows the slice order.
    pub fn seed_built_in(db: &Database, modes: &[BlueyMode]) -> Result<(), BlueyError> {
        let prepared: Vec<(String, String, String, String, i64)> = modes
            .iter()
            .enumerate()
            .map(|(i, m)| {
                Ok((
                    to_enum_str(&m.response_schema)?,
                    to_enum_str(&m.preferred_latency)?,
                    to_json_string(&m.context_requirements)?,
                    opt_to_json(&m.response_style)?.unwrap_or_default(),
                    i as i64,
                ))
            })
            .collect::<Result<_, BlueyError>>()?;
        db.transaction(|conn| {
            for (mode, (schema, latency, requirements, style, sort_order)) in
                modes.iter().zip(prepared)
            {
                let style: Option<String> = if style.is_empty() { None } else { Some(style) };
                let role = mode
                    .preferred_model_role
                    .as_ref()
                    .map(to_enum_str)
                    .transpose()?;
                conn.execute(
                    "INSERT INTO modes (id, name, description, icon, system_instructions,
                        response_schema, preferred_latency, context_requirements, built_in,
                        group_name, response_style, preferred_model_role, sort_order,
                        created_at, updated_at)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, 1, ?9, ?10, ?11, ?12, ?13, ?13)
                     ON CONFLICT(id) DO UPDATE SET
                       built_in = 1,
                       sort_order = excluded.sort_order",
                    params![
                        mode.id,
                        mode.name,
                        mode.description,
                        mode.icon,
                        mode.system_instructions,
                        schema,
                        latency,
                        requirements,
                        mode.group,
                        style,
                        role,
                        sort_order,
                        now_iso()
                    ],
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
                .prepare(&format!("SELECT {MODE_COLS} FROM modes ORDER BY sort_order, name"))
                .sql()?;
            let rows = stmt.query_map([], ModeRow::read).sql()?.collect::<Result<Vec<_>, _>>().sql()?;
            let mut stmt = conn
                .prepare("SELECT mode_id, document_id FROM mode_documents ORDER BY created_at, document_id")
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

    /// Create a custom mode from a draft. `name` is required.
    pub fn create(db: &Database, patch: &ModePatch) -> Result<BlueyMode, BlueyError> {
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
            group: patch.group.clone(),
            response_style: patch.response_style.clone(),
            preferred_model_role: patch.preferred_model_role,
            attached_document_ids: vec![],
            created_at: now.clone(),
            updated_at: now,
        };
        Self::insert_full(db, &mode)?;
        Ok(mode)
    }

    fn insert_full(db: &Database, mode: &BlueyMode) -> Result<(), BlueyError> {
        let schema = to_enum_str(&mode.response_schema)?;
        let latency = to_enum_str(&mode.preferred_latency)?;
        let requirements = to_json_string(&mode.context_requirements)?;
        let style = opt_to_json(&mode.response_style)?;
        let role = mode
            .preferred_model_role
            .as_ref()
            .map(to_enum_str)
            .transpose()?;
        db.with_conn(|conn| {
            let next_order: i64 = conn
                .query_row("SELECT coalesce(max(sort_order), -1) + 1 FROM modes", [], |r| r.get(0))
                .sql()?;
            conn.execute(
                "INSERT INTO modes (id, name, description, icon, system_instructions,
                    response_schema, preferred_latency, context_requirements, built_in,
                    group_name, response_style, preferred_model_role, sort_order, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)",
                params![
                    mode.id,
                    mode.name,
                    mode.description,
                    mode.icon,
                    mode.system_instructions,
                    schema,
                    latency,
                    requirements,
                    mode.built_in,
                    mode.group,
                    style,
                    role,
                    next_order,
                    mode.created_at,
                    mode.updated_at
                ],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Apply a partial update and return the updated mode (built-ins may be
    /// edited; [`ModeRepository::reset_built_in`] restores the original).
    pub fn update(db: &Database, id: &str, patch: &ModePatch) -> Result<BlueyMode, BlueyError> {
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
        if let Some(v) = &patch.group {
            mode.group = Some(v.clone());
        }
        if let Some(v) = &patch.response_style {
            mode.response_style = Some(v.clone());
        }
        if let Some(v) = patch.preferred_model_role {
            mode.preferred_model_role = Some(v);
        }
        mode.updated_at = now_iso();
        Self::write_columns(db, &mode)?;
        Ok(mode)
    }

    fn write_columns(db: &Database, mode: &BlueyMode) -> Result<(), BlueyError> {
        let schema = to_enum_str(&mode.response_schema)?;
        let latency = to_enum_str(&mode.preferred_latency)?;
        let requirements = to_json_string(&mode.context_requirements)?;
        let style = opt_to_json(&mode.response_style)?;
        let role = mode
            .preferred_model_role
            .as_ref()
            .map(to_enum_str)
            .transpose()?;
        let changed = db.with_conn(|conn| {
            conn.execute(
                "UPDATE modes SET name = ?2, description = ?3, icon = ?4, system_instructions = ?5,
                    response_schema = ?6, preferred_latency = ?7, context_requirements = ?8,
                    group_name = ?9, response_style = ?10, preferred_model_role = ?11, updated_at = ?12
                 WHERE id = ?1",
                params![
                    mode.id,
                    mode.name,
                    mode.description,
                    mode.icon,
                    mode.system_instructions,
                    schema,
                    latency,
                    requirements,
                    mode.group,
                    style,
                    role,
                    mode.updated_at
                ],
            )
            .sql()
        })?;
        if changed == 0 {
            return Err(not_found("mode", &mode.id));
        }
        Ok(())
    }

    /// Delete a custom mode. Built-in modes are refused with
    /// `internal.invalid_params`; sessions that referenced the mode fall back
    /// to `general` (schema `ON DELETE SET DEFAULT`).
    pub fn delete(db: &Database, id: &str) -> Result<(), BlueyError> {
        let mode = Self::get(db, id)?;
        if mode.built_in {
            return Err(BlueyError::invalid_params(format!(
                "built-in mode '{id}' cannot be deleted"
            )));
        }
        db.with_conn(|conn| {
            conn.execute("DELETE FROM modes WHERE id = ?1", [id])
                .sql()?;
            Ok(())
        })
    }

    /// Copy a mode (name suffixed with " (Copy)", `built_in = false`,
    /// document attachments copied).
    pub fn duplicate(db: &Database, id: &str) -> Result<BlueyMode, BlueyError> {
        let source = Self::get(db, id)?;
        let now = now_iso();
        let copy = BlueyMode {
            id: new_id("mode"),
            name: format!("{} (Copy)", source.name),
            built_in: false,
            attached_document_ids: source.attached_document_ids.clone(),
            created_at: now.clone(),
            updated_at: now,
            ..source
        };
        Self::insert_full(db, &copy)?;
        for doc_id in &copy.attached_document_ids {
            Self::attach_document(db, &copy.id, doc_id)?;
        }
        Ok(copy)
    }

    /// Restore a built-in mode to its shipped definition (attachments are kept).
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
        Self::write_columns(db, &restored)?;
        Ok(restored)
    }

    /// Attach a document to a mode (idempotent).
    pub fn attach_document(
        db: &Database,
        mode_id: &str,
        document_id: &str,
    ) -> Result<(), BlueyError> {
        db.with_conn(|conn| {
            conn.execute(
                "INSERT OR IGNORE INTO mode_documents (mode_id, document_id, created_at) VALUES (?1, ?2, ?3)",
                params![mode_id, document_id, now_iso()],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Detach a document from a mode (no-op when not attached).
    pub fn detach_document(
        db: &Database,
        mode_id: &str,
        document_id: &str,
    ) -> Result<(), BlueyError> {
        db.with_conn(|conn| {
            conn.execute(
                "DELETE FROM mode_documents WHERE mode_id = ?1 AND document_id = ?2",
                params![mode_id, document_id],
            )
            .sql()?;
            Ok(())
        })
    }

    /// Document ids attached to a mode, oldest attachment first.
    pub fn attached_document_ids(db: &Database, mode_id: &str) -> Result<Vec<String>, BlueyError> {
        db.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT document_id FROM mode_documents
                      WHERE mode_id = ?1 ORDER BY created_at, document_id",
                )
                .sql()?;
            let rows = stmt.query_map([mode_id], |r| r.get(0)).sql()?;
            rows.collect::<Result<Vec<String>, _>>().sql()
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::repositories::DocumentRepository;
    use crate::testutil;
    use bluey_core::types::documents::{DocumentKind, DocumentScope};
    use bluey_core::types::mode::{PreferredLatency, ResponseSchemaId};
    use pretty_assertions::assert_eq;

    #[test]
    fn seed_is_idempotent_and_preserves_user_edits() {
        let db = testutil::db(); // seeds "general"
        let seeded = ModeRepository::get(&db, "general").unwrap();
        assert!(seeded.built_in);

        // User edits instructions; re-seeding must not clobber them.
        let patch = ModePatch {
            system_instructions: Some("Be extremely brief.".into()),
            ..Default::default()
        };
        ModeRepository::update(&db, "general", &patch).unwrap();
        ModeRepository::seed_built_in(
            &db,
            &[
                testutil::mode("general", "General"),
                testutil::mode("sales", "Sales"),
            ],
        )
        .unwrap();
        let general = ModeRepository::get(&db, "general").unwrap();
        assert_eq!(general.system_instructions, "Be extremely brief.");
        assert!(ModeRepository::get(&db, "sales").is_ok());

        let list = ModeRepository::list(&db).unwrap();
        assert_eq!(
            list.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            vec!["general", "sales"]
        );
    }

    #[test]
    fn create_update_duplicate_delete() {
        let db = testutil::db();
        let err = ModeRepository::create(&db, &ModePatch::default()).unwrap_err();
        assert_eq!(err.code, "internal.invalid_params");

        let draft = ModePatch {
            name: Some("Pitch".into()),
            description: Some("Sales pitches".into()),
            response_schema: Some(ResponseSchemaId::Sales),
            preferred_latency: Some(PreferredLatency::Balanced),
            ..Default::default()
        };
        let mode = ModeRepository::create(&db, &draft).unwrap();
        assert!(!mode.built_in);
        assert_eq!(mode.response_schema, ResponseSchemaId::Sales);

        let updated = ModeRepository::update(
            &db,
            &mode.id,
            &ModePatch {
                icon: Some("bolt".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(updated.icon, "bolt");
        assert_eq!(updated.response_schema, ResponseSchemaId::Sales);

        let copy = ModeRepository::duplicate(&db, &mode.id).unwrap();
        assert_eq!(copy.name, "Pitch (Copy)");
        assert_ne!(copy.id, mode.id);

        ModeRepository::delete(&db, &copy.id).unwrap();
        assert!(ModeRepository::get(&db, &copy.id).is_err());
        let err = ModeRepository::delete(&db, "general").unwrap_err();
        assert_eq!(err.code, "internal.invalid_params");
    }

    #[test]
    fn reset_built_in_restores_original() {
        let db = testutil::db();
        ModeRepository::update(
            &db,
            "general",
            &ModePatch {
                name: Some("Weird".into()),
                system_instructions: Some("x".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let restored =
            ModeRepository::reset_built_in(&db, "general", &testutil::mode("general", "General"))
                .unwrap();
        assert_eq!(restored.name, "General");
        assert_eq!(restored.system_instructions, "You are in General mode.");
        let fetched = ModeRepository::get(&db, "general").unwrap();
        assert_eq!(fetched.name, "General");
    }

    #[test]
    fn attachments_round_trip_and_populate_reads() {
        let db = testutil::db();
        let doc = crate::documents::index::add_document(
            &db,
            &testutil::doc_input(
                DocumentKind::Notes,
                DocumentScope::Global,
                None,
                "attach me",
            ),
            |_| unreachable!("inline content"),
        )
        .unwrap();
        ModeRepository::attach_document(&db, "general", &doc.id).unwrap();
        ModeRepository::attach_document(&db, "general", &doc.id).unwrap(); // idempotent
        assert_eq!(
            ModeRepository::attached_document_ids(&db, "general").unwrap(),
            vec![doc.id.clone()]
        );
        assert_eq!(
            ModeRepository::get(&db, "general")
                .unwrap()
                .attached_document_ids,
            vec![doc.id.clone()]
        );
        assert_eq!(
            ModeRepository::list(&db).unwrap()[0].attached_document_ids,
            vec![doc.id.clone()]
        );

        // Deleting the document cascades the attachment away.
        DocumentRepository::delete(&db, &doc.id).unwrap();
        assert!(ModeRepository::attached_document_ids(&db, "general")
            .unwrap()
            .is_empty());

        ModeRepository::detach_document(&db, "general", "whatever").unwrap();
    }
}
