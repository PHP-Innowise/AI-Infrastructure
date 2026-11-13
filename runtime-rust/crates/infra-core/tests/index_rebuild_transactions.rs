//! Full and incremental rebuilds preserve the index and unfinished local work.
use infra_core::{ErrorKind, Roots, RuntimeContext, StartTask, Store, domain, index};
use rusqlite::{Connection, functions::FunctionFlags, params};
use std::{
    collections::BTreeMap,
    fs,
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

type Snapshot = BTreeMap<String, Vec<Vec<String>>>;

struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

struct Fixture {
    // Close SQLite before Directory removes the fixture on Windows.
    store: Store,
    context: RuntimeContext,
    directory: Directory,
}

impl Fixture {
    fn new() -> Self {
        let directory = Directory(
            std::env::temp_dir().join(format!("infra-index-transaction-{}", domain::uuid4())),
        );
        fs::create_dir_all(directory.0.join("docs")).unwrap();
        let context =
            RuntimeContext::new(Roots::installed(&directory.0).unwrap(), "local", "governed")
                .unwrap();
        let mut store = Store::open(directory.0.join("context.db"), Default::default()).unwrap();
        store
            .start(StartTask {
                task_id: "TASK-INDEX-KEEP".into(),
                goal: "Preserve unfinished local work during indexing".into(),
                files: vec!["docs/a.md".into()],
                ..Default::default()
            })
            .unwrap();
        store.conn.execute("INSERT INTO turn_deltas(task_id,created_at,files) VALUES('TASK-INDEX-KEEP',?1,'[\"docs/a.md\"]')", [domain::utc_now()]).unwrap();
        let mut fixture = Self {
            store,
            context,
            directory,
        };
        fixture.write("a", "# Alpha\nAlphaCache retains alpha values.\n");
        fixture.write("b", "# Beta\nBetaCache retains beta values.\n");
        fixture.write("c", "# Gamma\nGammaCache retains gamma values.\n");
        assert_eq!(fixture.refresh(false).documents, 3);
        fixture.store.conn.execute_batch(
            "INSERT INTO document_metadata SELECT 'orphan.md',category,privacy,owner,authority,lifecycle,source_hash,record_id,conflicts,source_fingerprints,updated_at,confidence,attestation FROM document_metadata WHERE path='docs/a.md';
             INSERT INTO document_links VALUES('orphan.md','absent.md','source');
             INSERT INTO identifier_terms VALUES('orphan.md','OrphanCache','orphan cache');",
        ).unwrap();
        fixture
    }

    fn write(&self, name: &str, body: &str) {
        fs::write(self.directory.0.join(format!("docs/{name}.md")), body).unwrap();
    }

    fn refresh(&mut self, incremental: bool) -> index::IndexReport {
        index::refresh_index(&mut self.store, &self.context, incremental).unwrap()
    }

    fn change_two_sources(&self) {
        self.write(
            "a",
            "# Alpha changed\nChangedAlphaCache retains the new alpha values.\n",
        );
        self.write(
            "b",
            "# Beta changed\nChangedBetaCache retains the new beta values.\n",
        );
    }

    fn install_fault(&self) -> Arc<AtomicBool> {
        let observed = Arc::new(AtomicBool::new(false));
        let trigger_observed = Arc::clone(&observed);
        self.store
            .conn
            .create_scalar_function(
                "index_rebuild_observe_first_insert",
                1,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    trigger_observed.store(context.get::<bool>(0)?, Ordering::SeqCst);
                    Ok(0_i64)
                },
            )
            .unwrap();
        self.store.conn.execute_batch(
            "CREATE TEMP TRIGGER index_rebuild_fault BEFORE INSERT ON document_metadata
             WHEN NEW.path='docs/b.md'
             BEGIN
               SELECT index_rebuild_observe_first_insert(EXISTS(
                 SELECT 1 FROM documents WHERE path='docs/a.md' AND content LIKE '%ChangedAlphaCache%'
               ));
               SELECT RAISE(ABORT,'index-rebuild-fault-after-a')
                 WHERE EXISTS(SELECT 1 FROM documents WHERE path='docs/a.md' AND content LIKE '%ChangedAlphaCache%');
               SELECT RAISE(ABORT,'index-rebuild-fault-before-a');
             END;",
        ).unwrap();
        observed
    }

    fn install_source_state_fault(&self) -> Arc<AtomicBool> {
        let expected_hash =
            infra_core::query::sha256(&fs::read(self.directory.0.join("docs/a.md")).unwrap());
        let observed = Arc::new(AtomicBool::new(false));
        let trigger_observed = Arc::clone(&observed);
        self.store
            .conn
            .create_scalar_function(
                "index_rebuild_observe_new_metadata",
                1,
                FunctionFlags::SQLITE_UTF8,
                move |context| {
                    trigger_observed.store(context.get::<bool>(0)?, Ordering::SeqCst);
                    Ok(0_i64)
                },
            )
            .unwrap();
        // This late fault leaves the rowid-delete path enabled: it cannot
        // change documents during the earlier deletion phase.
        self.store.conn.execute_batch(&format!(
            "CREATE TEMP TRIGGER index_rebuild_fault BEFORE INSERT ON document_source_state
             WHEN NEW.path='docs/b.md'
             BEGIN
               SELECT index_rebuild_observe_new_metadata(EXISTS(
                 SELECT 1 FROM document_metadata WHERE path='docs/a.md' AND source_hash='{expected_hash}'
               ));
               SELECT RAISE(ABORT,'index-rebuild-source-state-fault');
             END;",
        )).unwrap();
        observed
    }

    fn assert_orphans(&self) {
        for table in ["document_metadata", "document_links", "identifier_terms"] {
            let count: i64 = self
                .store
                .conn
                .query_row(
                    &format!("SELECT COUNT(*) FROM {table} WHERE path='orphan.md'"),
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(count, 1, "pre-existing orphan in {table}");
        }
    }
}

fn rows(conn: &Connection, sql: &str) -> Vec<Vec<String>> {
    let mut statement = conn.prepare(sql).unwrap();
    let width = statement.column_count();
    let mut rows = statement
        .query_map([], |row| {
            (0..width)
                .map(|column| Ok(format!("{:?}", row.get_ref(column)?)))
                .collect::<rusqlite::Result<Vec<_>>>()
        })
        .unwrap()
        .collect::<rusqlite::Result<Vec<_>>>()
        .unwrap();
    rows.sort();
    rows
}

fn local_state(conn: &Connection) -> Snapshot {
    ["working_tasks", "turn_deltas", "task_bindings"]
        .map(|table| {
            (
                table.into(),
                rows(conn, &format!("SELECT rowid,* FROM {table}")),
            )
        })
        .into()
}

fn logical_index(conn: &Connection) -> Snapshot {
    [
        "documents",
        "document_metadata",
        "document_links",
        "identifier_terms",
        "document_source_state",
    ]
    .map(|table| (table.into(), rows(conn, &format!("SELECT * FROM {table}"))))
    .into()
}

fn database_snapshot(conn: &Connection) -> Snapshot {
    // Use the owning SQLite connection. A separate database FD could release
    // this process's POSIX locks, invalidating the ambient-transaction check.
    let tables = {
        let mut statement = conn.prepare("PRAGMA main.table_list").unwrap();
        statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, bool>(4)?,
                ))
            })
            .unwrap()
            .collect::<rusqlite::Result<Vec<_>>>()
            .unwrap()
    };
    tables
        .into_iter()
        .filter(|(schema, _, kind, _)| schema == "main" && kind != "view")
        .map(|(_, table, _, without_rowid)| {
            let quoted = table.replace('"', "\"\"");
            let fields = if without_rowid { "*" } else { "rowid,*" };
            let image = rows(conn, &format!("SELECT {fields} FROM main.\"{quoted}\""));
            (table, image)
        })
        .collect()
}

#[test]
fn full_partial_and_noop_refresh_preserve_documents_orphans_and_local_work() {
    let mut fixture = Fixture::new();
    let local = local_state(&fixture.store.conn);
    assert_eq!(local["working_tasks"].len(), 1);
    assert_eq!(local["turn_deltas"].len(), 1);
    let before_noop = database_snapshot(&fixture.store.conn);
    let noop = fixture.refresh(true);
    assert_eq!(noop.reused, 3);
    assert_eq!(before_noop, database_snapshot(&fixture.store.conn));

    let retained_rowid: i64 = fixture
        .store
        .conn
        .query_row(
            "SELECT rowid FROM documents WHERE path='docs/c.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    fixture.write(
        "a",
        "# Alpha changed\nChangedAlphaCache retains the updated values.\n",
    );
    fs::remove_file(fixture.directory.0.join("docs/b.md")).unwrap();
    fixture.write("d", "# Delta\nDeltaCache retains delta values.\n");
    let partial = fixture.refresh(true);
    assert_eq!(
        (partial.documents, partial.removed, partial.reused),
        (3, 1, 1)
    );
    let actual_rowid: i64 = fixture
        .store
        .conn
        .query_row(
            "SELECT rowid FROM documents WHERE path='docs/c.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(actual_rowid, retained_rowid);
    assert!(
        index::load_document(&fixture.store.conn, "docs/b.md")
            .unwrap()
            .is_none()
    );
    assert!(
        index::load_document(&fixture.store.conn, "docs/a.md")
            .unwrap()
            .unwrap()
            .content
            .contains("ChangedAlphaCache")
    );
    assert_eq!(local, local_state(&fixture.store.conn));
    fixture.assert_orphans();

    let partial_image = logical_index(&fixture.store.conn);
    let generation = fixture.store.state().unwrap()["index-generation"].clone();
    let full = fixture.refresh(false);
    assert_eq!((full.documents, full.reused), (3, 0));
    assert_eq!(partial_image, logical_index(&fixture.store.conn));
    assert_ne!(
        generation,
        fixture.store.state().unwrap()["index-generation"]
    );
    assert_eq!(local, local_state(&fixture.store.conn));
    fixture.assert_orphans();
}

#[test]
fn legacy_index_upgrade_rolls_back_with_metadata_and_unfinished_work_on_late_failure() {
    for ambient in [false, true] {
        let mut fixture = Fixture::new();
        fixture
            .store
            .conn
            .execute(
                "DELETE FROM index_state WHERE key='index-document-format'",
                [],
            )
            .unwrap();
        if ambient {
            fixture
                .store
                .conn
                .execute_batch(
                    "BEGIN IMMEDIATE; UPDATE working_tasks SET progress='outer progress'",
                )
                .unwrap();
        }
        let observed = fixture.install_source_state_fault();
        let before = database_snapshot(&fixture.store.conn);
        let local = local_state(&fixture.store.conn);
        let error = index::refresh_index(&mut fixture.store, &fixture.context, true).unwrap_err();
        assert!(
            observed.load(Ordering::SeqCst),
            "unchanged sources must be reindexed for the legacy format"
        );
        assert_eq!(error.kind, ErrorKind::Unavailable);
        assert_eq!(before, database_snapshot(&fixture.store.conn));
        assert_eq!(local, local_state(&fixture.store.conn));
        assert_eq!(fixture.store.conn.is_autocommit(), !ambient);
        fixture
            .store
            .conn
            .execute_batch("DROP TRIGGER temp.index_rebuild_fault")
            .unwrap();
        assert_eq!(fixture.refresh(true).documents, 3);
        assert!(
            fixture
                .store
                .state()
                .unwrap()
                .contains_key("index-document-format")
        );
        assert_eq!(local, local_state(&fixture.store.conn));
        if ambient {
            fixture.store.conn.execute_batch("ROLLBACK").unwrap();
            assert!(
                !fixture
                    .store
                    .state()
                    .unwrap()
                    .contains_key("index-document-format")
            );
        }
    }
}

#[test]
fn legacy_empty_index_still_commits_the_new_document_format() {
    let mut fixture = Fixture::new();
    for name in ["a", "b", "c"] {
        fs::remove_file(fixture.directory.0.join(format!("docs/{name}.md"))).unwrap();
    }
    assert_eq!(fixture.refresh(false).documents, 0);
    fixture
        .store
        .conn
        .execute(
            "UPDATE index_state SET value='older-format' WHERE key='index-document-format'",
            [],
        )
        .unwrap();
    let generation = fixture.store.state().unwrap()["index-generation"].clone();
    let local = local_state(&fixture.store.conn);
    assert_eq!(fixture.refresh(true).documents, 0);
    assert_ne!(
        generation,
        fixture.store.state().unwrap()["index-generation"]
    );
    assert_ne!(
        Some("older-format"),
        fixture
            .store
            .state()
            .unwrap()
            .get("index-document-format")
            .map(String::as_str)
    );
    assert_eq!(local, local_state(&fixture.store.conn));
}

#[test]
fn failed_full_and_partial_rebuilds_restore_all_rows_with_or_without_an_ambient_transaction() {
    for ambient in [false, true] {
        for incremental in [false, true] {
            let mut fixture = Fixture::new();
            let before_outer = database_snapshot(&fixture.store.conn);
            if ambient {
                fixture.store.conn.execute_batch(
                    "BEGIN IMMEDIATE;
                     UPDATE working_tasks SET progress='outer progress';
                     INSERT INTO turn_deltas(task_id,created_at,files) SELECT task_id,created_at,files FROM turn_deltas;",
                ).unwrap();
            }
            fixture.change_two_sources();
            let observed_first_insert = fixture.install_fault();
            let before = database_snapshot(&fixture.store.conn);
            let local = local_state(&fixture.store.conn);
            let error = index::refresh_index(&mut fixture.store, &fixture.context, incremental)
                .unwrap_err();
            // The public error deliberately omits SQLite trigger text. This
            // observation survives rollback and proves the actual insert path
            // reached the fault after the first changed document was visible.
            assert!(observed_first_insert.load(Ordering::SeqCst));
            assert_eq!(error.kind, ErrorKind::Unavailable);
            assert_eq!(error.message, "Local SQLite operation failed");
            assert_eq!(fixture.store.conn.is_autocommit(), !ambient);
            assert_eq!(
                before,
                database_snapshot(&fixture.store.conn),
                "rollback: ambient={ambient}, incremental={incremental}"
            );
            assert_eq!(local, local_state(&fixture.store.conn));
            fixture.assert_orphans();

            fixture
                .store
                .conn
                .execute_batch("DROP TRIGGER temp.index_rebuild_fault")
                .unwrap();
            assert_eq!(fixture.refresh(incremental).documents, 3);
            assert_eq!(fixture.store.conn.is_autocommit(), !ambient);
            assert_eq!(local, local_state(&fixture.store.conn));
            if ambient {
                fixture.store.conn.execute_batch("ROLLBACK").unwrap();
                assert_eq!(before_outer, database_snapshot(&fixture.store.conn));
            }
        }
    }
}

fn duplicate_document(conn: &Connection, rowid: i64, path: &str) {
    conn.execute(
        "INSERT INTO documents(rowid,path,layer,kind,title,summary,content)
         SELECT ?1,path,layer,kind,title,summary,content FROM documents WHERE path=?2 LIMIT 1",
        params![rowid, path],
    )
    .unwrap();
}

#[test]
fn partial_rebuild_deletes_all_changed_and_removed_rowids_but_keeps_retained_duplicates() {
    let mut fixture = Fixture::new();
    fixture
        .store
        .conn
        .execute(
            "UPDATE documents SET rowid=?1 WHERE path='docs/a.md'",
            [i64::MIN],
        )
        .unwrap();
    duplicate_document(&fixture.store.conn, i64::MAX, "docs/a.md");
    duplicate_document(&fixture.store.conn, 0, "docs/b.md");
    fixture
        .store
        .conn
        .execute("UPDATE documents SET rowid=-9 WHERE path='docs/c.md'", [])
        .unwrap();
    duplicate_document(&fixture.store.conn, -8, "docs/c.md");
    let retained = rows(
        &fixture.store.conn,
        "SELECT rowid,* FROM documents WHERE path='docs/c.md'",
    );
    let local = local_state(&fixture.store.conn);
    fixture.write(
        "a",
        "# Updated alpha\nChangedAlphaCache holds the replacement value.\n",
    );
    fs::remove_file(fixture.directory.0.join("docs/b.md")).unwrap();
    let report = fixture.refresh(true);
    assert_eq!((report.documents, report.removed, report.reused), (3, 1, 1));
    assert_eq!(
        retained,
        rows(
            &fixture.store.conn,
            "SELECT rowid,* FROM documents WHERE path='docs/c.md'"
        )
    );
    let count: i64 = fixture
        .store
        .conn
        .query_row(
            "SELECT COUNT(*) FROM documents WHERE path='docs/a.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        rows(
            &fixture.store.conn,
            "SELECT rowid FROM documents WHERE path='docs/b.md'"
        )
        .is_empty()
    );
    assert!(
        rows(
            &fixture.store.conn,
            "SELECT path FROM documents WHERE documents MATCH '\"AlphaCache\"'"
        )
        .is_empty()
    );
    assert_eq!(local, local_state(&fixture.store.conn));
    fixture.assert_orphans();
}

#[test]
fn partial_rebuild_preserves_retained_rowids_at_both_signed_limits() {
    for rowid in [i64::MIN, i64::MAX] {
        let mut fixture = Fixture::new();
        fixture
            .store
            .conn
            .execute(
                "UPDATE documents SET rowid=?1 WHERE path='docs/c.md'",
                [rowid],
            )
            .unwrap();
        let retained = rows(
            &fixture.store.conn,
            "SELECT rowid,* FROM documents WHERE path='docs/c.md'",
        );
        fixture.write(
            "a",
            "# Updated alpha\nChangedAlphaCache holds another value.\n",
        );
        assert_eq!(fixture.refresh(true).reused, 2);
        assert_eq!(
            retained,
            rows(
                &fixture.store.conn,
                "SELECT rowid,* FROM documents WHERE path='docs/c.md'"
            )
        );
    }
}

#[test]
fn main_and_temp_auxiliary_triggers_keep_path_deletion_semantics() {
    for temporary in [false, true] {
        for incremental in [false, true] {
            let mut fixture = Fixture::new();
            let observed = Arc::new(AtomicBool::new(false));
            let trigger_observed = Arc::clone(&observed);
            fixture
                .store
                .conn
                .create_scalar_function(
                    "index_rebuild_observe_later_path",
                    1,
                    FunctionFlags::SQLITE_UTF8,
                    move |context| {
                        trigger_observed.store(context.get::<bool>(0)?, Ordering::SeqCst);
                        Ok(0_i64)
                    },
                )
                .unwrap();
            fixture.store.conn.execute_batch(&format!(
            "CREATE {}TRIGGER insert_later_path AFTER DELETE ON DOCUMENT_METADATA
             WHEN OLD.path='docs/a.md'
             BEGIN
               INSERT INTO documents(path,layer,kind,title,summary,content)
                 VALUES('docs/b.md','semantic','spec','Trigger row','Trigger summary','STALE_TRIGGER_ROW');
               SELECT index_rebuild_observe_later_path(EXISTS(
                 SELECT 1 FROM documents WHERE path='docs/b.md' AND content='STALE_TRIGGER_ROW'
               ));
             END;",
            if temporary { "TEMP " } else { "" },
        )).unwrap();
            let local = local_state(&fixture.store.conn);
            fixture.change_two_sources();
            assert_eq!(fixture.refresh(incremental).documents, 3);
            assert!(
                observed.load(Ordering::SeqCst),
                "the auxiliary trigger must introduce a later-path row"
            );
            let count: i64 = fixture
                .store
                .conn
                .query_row(
                    "SELECT COUNT(*) FROM documents WHERE path='docs/b.md'",
                    [],
                    |row| row.get(0),
                )
                .unwrap();
            assert_eq!(
                count, 1,
                "the subsequent path deletion must also remove the trigger's row"
            );
            assert!(
                rows(
                    &fixture.store.conn,
                    "SELECT path FROM documents WHERE content='STALE_TRIGGER_ROW'"
                )
                .is_empty()
            );
            assert_eq!(local, local_state(&fixture.store.conn));
            fixture.assert_orphans();
        }
    }
}

#[test]
fn fast_full_and_partial_rebuilds_roll_back_late_faults_with_or_without_an_ambient_transaction() {
    for ambient in [false, true] {
        for incremental in [false, true] {
            let mut fixture = Fixture::new();
            let before_outer = database_snapshot(&fixture.store.conn);
            if ambient {
                fixture.store.conn.execute_batch(
                "BEGIN IMMEDIATE;
                 UPDATE working_tasks SET progress='outer progress';
                 INSERT INTO turn_deltas(task_id,created_at,files) SELECT task_id,created_at,files FROM turn_deltas;",
            ).unwrap();
            }
            fixture.change_two_sources();
            let observed = fixture.install_source_state_fault();
            let before = database_snapshot(&fixture.store.conn);
            let local = local_state(&fixture.store.conn);
            let error = index::refresh_index(&mut fixture.store, &fixture.context, incremental)
                .unwrap_err();
            assert!(
                observed.load(Ordering::SeqCst),
                "new document metadata must be visible at the late fault"
            );
            assert_eq!(error.kind, ErrorKind::Unavailable);
            assert_eq!(error.message, "Local SQLite operation failed");
            assert_eq!(fixture.store.conn.is_autocommit(), !ambient);
            assert_eq!(before, database_snapshot(&fixture.store.conn));
            assert_eq!(local, local_state(&fixture.store.conn));
            fixture.assert_orphans();
            fixture
                .store
                .conn
                .execute_batch("DROP TRIGGER temp.index_rebuild_fault")
                .unwrap();
            assert_eq!(fixture.refresh(incremental).documents, 3);
            assert_eq!(local, local_state(&fixture.store.conn));
            if ambient {
                fixture.store.conn.execute_batch("ROLLBACK").unwrap();
                assert_eq!(before_outer, database_snapshot(&fixture.store.conn));
            }
        }
    }
}
