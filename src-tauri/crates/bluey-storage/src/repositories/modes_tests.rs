//! Unit tests for [`super::ModeRepository`] (kept apart to keep `modes.rs` small).

use super::*;
use crate::db::MIGRATIONS;
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

/// Add an inline mode-scoped document (indexed into chunks + FTS).
fn add_mode_doc(db: &Database, mode_id: &str, content: &str) -> String {
    crate::documents::index::add_document(
        db,
        &testutil::doc_input(
            DocumentKind::Notes,
            DocumentScope::Mode,
            Some(mode_id),
            content,
        ),
        |_| unreachable!("inline content"),
    )
    .unwrap()
    .id
}

/// Rows in `table` whose `document_id` column is `doc_id`.
fn count_for_doc(db: &Database, table: &str, doc_id: &str) -> i64 {
    db.with_conn(|c| {
        c.query_row(
            &format!("SELECT count(*) FROM {table} WHERE document_id = ?1"),
            [doc_id],
            |r| r.get(0),
        )
        .sql()
    })
    .unwrap()
}

#[test]
fn deleting_a_mode_deletes_its_files_chunks_and_fts_rows() {
    let db = testutil::db();
    let mode = ModeRepository::create(
        &db,
        &ModePatch {
            name: Some("Pitch".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let doc_id = add_mode_doc(&db, &mode.id, "quarterly pricing notes for the pitch");
    let global = crate::documents::index::add_document(
        &db,
        &testutil::doc_input(DocumentKind::Notes, DocumentScope::Global, None, "keep me"),
        |_| unreachable!("inline content"),
    )
    .unwrap();
    assert!(count_for_doc(&db, "document_chunks", &doc_id) > 0);
    assert!(count_for_doc(&db, "document_chunks_fts", &doc_id) > 0);

    ModeRepository::delete(&db, &mode.id).unwrap();

    assert!(DocumentRepository::get(&db, &doc_id).is_err());
    assert_eq!(count_for_doc(&db, "document_chunks", &doc_id), 0);
    assert_eq!(count_for_doc(&db, "document_chunks_fts", &doc_id), 0);
    assert!(DocumentRepository::get(&db, &global.id).is_ok());
}

/// A database file at the 0.1.2 schema (migrations 0001–0004) prepared by
/// `setup`, then reopened through [`Database::open`] so the newer
/// migrations run as they would on upgrade.
fn upgraded_from_0_1_2(setup: impl FnOnce(&rusqlite::Connection)) -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bluey.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA recursive_triggers = ON;
             CREATE TABLE schema_migrations (name TEXT PRIMARY KEY, applied_at TEXT NOT NULL);",
        )
        .unwrap();
        for (name, sql) in MIGRATIONS
            .iter()
            .take_while(|(name, _)| *name <= "0004_ai_request_trace")
        {
            conn.execute_batch(sql).unwrap();
            conn.execute(
                "INSERT INTO schema_migrations (name, applied_at) VALUES (?1, ?2)",
                params![name, now_iso()],
            )
            .unwrap();
        }
        setup(&conn);
    }
    let db = Database::open(&path).unwrap();
    (dir, db)
}

/// Insert a document with one chunk the way 0.1.2 did (FTS via trigger).
fn insert_legacy_doc(conn: &rusqlite::Connection, id: &str, scope: &str, scope_id: &str) {
    let now = now_iso();
    conn.execute(
        "INSERT INTO documents (id, title, kind, format, scope, scope_id, content,
            created_at, updated_at)
         VALUES (?1, 'Notes', 'notes', 'txt', ?2, ?3, 'secret agenda', ?4, ?4)",
        params![id, scope, scope_id, now],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO document_chunks (id, document_id, chunk_index, content, created_at)
         VALUES (?1, ?2, 0, 'secret agenda', ?3)",
        params![format!("chk_{id}"), id, now],
    )
    .unwrap();
}

#[test]
fn upgrade_purges_files_left_behind_by_deleted_modes() {
    let (_dir, db) = upgraded_from_0_1_2(|conn| {
        conn.execute(
            "INSERT INTO modes (id, name, description, icon, system_instructions,
                response_schema, preferred_latency, context_requirements, built_in,
                sort_order, created_at, updated_at)
             VALUES ('mode_kept', 'Kept', '', 'sparkles', '', 'answer', 'fast', '[]', 0, 0,
                '2026-01-01T00:00:00.000Z', '2026-01-01T00:00:00.000Z')",
            [],
        )
        .unwrap();
        insert_legacy_doc(conn, "doc_orphan", "mode", "mode_deleted");
        insert_legacy_doc(conn, "doc_kept", "mode", "mode_kept");
        insert_legacy_doc(conn, "doc_global", "global", "");
    });

    assert!(DocumentRepository::get(&db, "doc_orphan").is_err());
    assert_eq!(count_for_doc(&db, "document_chunks", "doc_orphan"), 0);
    assert_eq!(count_for_doc(&db, "document_chunks_fts", "doc_orphan"), 0);
    assert!(DocumentRepository::get(&db, "doc_kept").is_ok());
    assert!(DocumentRepository::get(&db, "doc_global").is_ok());
    assert_eq!(count_for_doc(&db, "document_chunks_fts", "doc_kept"), 1);
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
fn attachments_are_the_mode_scoped_documents() {
    let db = testutil::db();
    let doc_id = add_mode_doc(&db, "general", "attach me");
    let global = crate::documents::index::add_document(
        &db,
        &testutil::doc_input(DocumentKind::Notes, DocumentScope::Global, None, "not mine"),
        |_| unreachable!("inline content"),
    )
    .unwrap();
    assert_ne!(global.id, doc_id);
    assert_eq!(
        ModeRepository::attached_document_ids(&db, "general").unwrap(),
        vec![doc_id.clone()]
    );
    assert_eq!(
        ModeRepository::get(&db, "general")
            .unwrap()
            .attached_document_ids,
        vec![doc_id.clone()]
    );
    assert_eq!(
        ModeRepository::list(&db).unwrap()[0].attached_document_ids,
        vec![doc_id.clone()]
    );

    DocumentRepository::delete(&db, &doc_id).unwrap();
    assert!(ModeRepository::attached_document_ids(&db, "general")
        .unwrap()
        .is_empty());
}

#[test]
fn duplicate_copies_files_so_the_copy_lists_and_retrieves_them() {
    use bluey_core::types::documents::{RetrievalQuery, RetrievalStrategy, ScopeRef};
    let db = testutil::db();
    let source_doc = add_mode_doc(&db, "general", "the quarterly roadmap mentions zebras");

    let copy = ModeRepository::duplicate(&db, "general").unwrap();

    let listed = DocumentRepository::list(&db, Some(DocumentScope::Mode), Some(&copy.id)).unwrap();
    assert_eq!(listed.len(), 1);
    assert_ne!(listed[0].id, source_doc);
    assert_eq!(copy.attached_document_ids, vec![listed[0].id.clone()]);
    assert_eq!(
        ModeRepository::get(&db, &copy.id)
            .unwrap()
            .attached_document_ids,
        copy.attached_document_ids
    );
    assert_eq!(
        DocumentRepository::chunks(&db, &listed[0].id)
            .unwrap()
            .len(),
        DocumentRepository::chunks(&db, &source_doc).unwrap().len()
    );
    let hits = crate::documents::retrieve::retrieve(
        &db,
        &RetrievalQuery {
            query: "zebras".into(),
            scopes: vec![ScopeRef {
                scope: DocumentScope::Mode,
                scope_id: Some(copy.id.clone()),
            }],
            kinds: None,
            limit: None,
            strategy: Some(RetrievalStrategy::Keyword),
        },
        None,
    )
    .unwrap();
    assert!(!hits.is_empty());
    assert!(hits.iter().all(|h| h.document_id == listed[0].id));

    // The source keeps its own file; deleting the copy leaves it alone.
    ModeRepository::delete(&db, &copy.id).unwrap();
    assert!(DocumentRepository::get(&db, &source_doc).is_ok());
    assert_eq!(
        ModeRepository::attached_document_ids(&db, "general").unwrap(),
        vec![source_doc]
    );
}

/// `general` as a later release ships it.
fn general_v2() -> BlueyMode {
    BlueyMode {
        system_instructions: "Improved general instructions.".into(),
        preferred_latency: PreferredLatency::Balanced,
        ..testutil::mode("general", "General")
    }
}

#[test]
fn seed_refreshes_unedited_built_ins_and_keeps_edited_ones() {
    let db = testutil::db(); // seeds v1 of "general"
    testutil::seed_mode(&db, "sales", "Sales");

    // User edits "sales" only; the next release changes both definitions.
    let edit = ModePatch {
        system_instructions: Some("My own sales playbook.".into()),
        ..Default::default()
    };
    ModeRepository::update(&db, "sales", &edit).unwrap();
    let sales_v2 = BlueyMode {
        description: "New sales copy.".into(),
        ..testutil::mode("sales", "Sales")
    };
    ModeRepository::seed_built_in(&db, &[general_v2(), sales_v2.clone()]).unwrap();

    let general = ModeRepository::get(&db, "general").unwrap();
    assert_eq!(
        general.system_instructions,
        "Improved general instructions."
    );
    assert_eq!(general.preferred_latency, PreferredLatency::Balanced);
    let sales = ModeRepository::get(&db, "sales").unwrap();
    assert_eq!(sales.system_instructions, "My own sales playbook.");
    assert_eq!(sales.description, "Sales mode");

    // Reset takes the shipped definition and makes the row refreshable again.
    ModeRepository::reset_built_in(&db, "sales", &sales_v2).unwrap();
    let sales_v3 = BlueyMode {
        icon: "chart".into(),
        ..sales_v2
    };
    ModeRepository::seed_built_in(&db, &[general_v2(), sales_v3]).unwrap();
    let sales = ModeRepository::get(&db, "sales").unwrap();
    assert_eq!(sales.icon, "chart");
    assert_eq!(sales.description, "New sales copy.");
}

#[test]
fn upgrade_refreshes_built_ins_a_0_1_2_user_never_edited() {
    let (_dir, db) = upgraded_from_0_1_2(|conn| {
        // 0.1.2 seeded with created_at = updated_at; an edit bumped updated_at.
        for (id, updated_at) in [
            ("general", "2026-01-01T00:00:00.000Z"),
            ("sales", "2026-02-01T00:00:00.000Z"),
        ] {
            conn.execute(
                "INSERT INTO modes (id, name, description, icon, system_instructions,
                    response_schema, preferred_latency, context_requirements, built_in,
                    sort_order, created_at, updated_at)
                 VALUES (?1, ?1, 'old', 'sparkles', 'Old text.', 'answer', 'fast', '[]', 1, 0,
                    '2026-01-01T00:00:00.000Z', ?2)",
                params![id, updated_at],
            )
            .unwrap();
        }
    });

    ModeRepository::seed_built_in(&db, &[general_v2(), testutil::mode("sales", "Sales")]).unwrap();

    let general = ModeRepository::get(&db, "general").unwrap();
    assert_eq!(
        general.system_instructions,
        "Improved general instructions."
    );
    assert_eq!(general.name, "General");
    let sales = ModeRepository::get(&db, "sales").unwrap();
    assert_eq!(sales.system_instructions, "Old text.");
    assert_eq!(sales.description, "old");

    // Seeding the same release again changes nothing.
    ModeRepository::seed_built_in(&db, &[general_v2(), testutil::mode("sales", "Sales")]).unwrap();
    assert_eq!(
        ModeRepository::get(&db, "general").unwrap().updated_at,
        general.updated_at
    );
}

fn invalid(db: &Database, id: &str, patch: ModePatch) -> String {
    let err = ModeRepository::update(db, id, &patch).unwrap_err();
    assert_eq!(err.code, "internal.invalid_params");
    err.message
}

#[test]
fn create_and_update_enforce_the_mode_limits() {
    let db = testutil::db();
    let mode = ModeRepository::create(
        &db,
        &ModePatch {
            name: Some("x".repeat(60)),
            system_instructions: Some("é".repeat(4000)),
            icon: Some("graduation-cap".into()),
            preferred_model_role: Some(Some(ModelRole::Research)),
            ..Default::default()
        },
    )
    .unwrap();
    let too_long_name = ModePatch {
        name: Some("x".repeat(61)),
        ..Default::default()
    };
    assert!(ModeRepository::create(&db, &too_long_name).is_err());

    let msg = invalid(
        &db,
        &mode.id,
        ModePatch {
            system_instructions: Some("a".repeat(4001)),
            ..Default::default()
        },
    );
    assert!(msg.contains("4000"), "{msg}");
    invalid(
        &db,
        &mode.id,
        ModePatch {
            description: Some("a".repeat(301)),
            ..Default::default()
        },
    );
    invalid(
        &db,
        &mode.id,
        ModePatch {
            icon: Some("Graduation Cap".into()),
            ..Default::default()
        },
    );
    invalid(
        &db,
        &mode.id,
        ModePatch {
            icon: Some("cap-".into()),
            ..Default::default()
        },
    );
    for role in [ModelRole::Transcription, ModelRole::Embedding] {
        invalid(
            &db,
            &mode.id,
            ModePatch {
                preferred_model_role: Some(Some(role)),
                ..Default::default()
            },
        );
    }
    // Nothing was written by the rejected updates.
    assert_eq!(
        ModeRepository::get(&db, &mode.id).unwrap().updated_at,
        mode.updated_at
    );
}

#[test]
fn null_clears_the_group_and_model_role_while_absent_keeps_them() {
    let db = testutil::db();
    let set = ModePatch {
        group: Some(Some("Work".into())),
        preferred_model_role: Some(Some(ModelRole::Reasoning)),
        ..Default::default()
    };
    let mode = ModeRepository::update(&db, "general", &set).unwrap();
    assert_eq!(mode.group.as_deref(), Some("Work"));

    let rename = ModePatch {
        name: Some("Everyday".into()),
        ..Default::default()
    };
    let mode = ModeRepository::update(&db, "general", &rename).unwrap();
    assert_eq!(mode.group.as_deref(), Some("Work"));
    assert_eq!(mode.preferred_model_role, Some(ModelRole::Reasoning));

    let clear = ModePatch {
        group: Some(None),
        preferred_model_role: Some(None),
        ..Default::default()
    };
    ModeRepository::update(&db, "general", &clear).unwrap();
    let stored = ModeRepository::get(&db, "general").unwrap();
    assert_eq!(stored.group, None);
    assert_eq!(stored.preferred_model_role, None);

    // A blank group is no group.
    let blank = ModePatch {
        group: Some(Some("  ".into())),
        ..Default::default()
    };
    assert_eq!(
        ModeRepository::update(&db, "general", &blank)
            .unwrap()
            .group,
        None
    );
}
