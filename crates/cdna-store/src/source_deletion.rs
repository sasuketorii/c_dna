//! Source erasure includes every historical provenance link, not only the head.
use super::*;

impl Store {
    /// Atomically erase a source and every observation derived from it in any
    /// revision. Returns the number of erased observations (excluding the source).
    pub fn delete_source_cascade(
        &mut self,
        workspace: Uuid,
        id: Uuid,
        expected_revision: u64,
    ) -> Result<usize> {
        if id == Self::EVOLUTION_DOCUMENT_ID || expected_revision >= i64::MAX as u64 {
            return Err(StoreError::InvalidInput);
        }
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ws = workspace.to_string();
        let source = id.to_string();
        let current: Option<u64> = tx
            .query_row(
                "SELECT revision FROM documents WHERE workspace=?1 AND kind='source' AND id=?2",
                params![ws, source],
                |r| r.get(0),
            )
            .optional()?;
        if current.ok_or(StoreError::NotFound)? != expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        tx.execute(
            "DELETE FROM documents WHERE workspace=?1 AND kind='source' AND id=?2",
            params![ws, source],
        )?;
        tx.execute(
            "INSERT OR IGNORE INTO document_tombstones VALUES(?1,'source',?2)",
            params![ws, source],
        )?;
        let count = purge_sources(&tx, Some(&ws), Some(&source))?;
        invalidate_workspace(&tx, &ws)?;
        tx.execute("UPDATE metadata SET epoch=epoch+1", [])?;
        tx.commit()?;
        Ok(count)
    }
}

pub(super) fn check_source(conn: &Connection, workspace: &str, payload: &Value) -> Result<()> {
    if let Some(source) = payload
        .pointer("/source/artifact_id")
        .and_then(Value::as_str)
        .and_then(|id| Uuid::parse_str(id).ok())
    {
        let deleted: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM document_tombstones WHERE workspace=?1 AND kind='source' AND id=?2)",
            params![workspace, source.to_string()], |r| r.get(0),
        )?;
        if deleted {
            return Err(StoreError::NotFound);
        }
    }
    Ok(())
}

pub(super) fn purge_tombstoned_sources(conn: &Connection) -> Result<()> {
    purge_sources(conn, None, None)?;
    Ok(())
}

fn purge_sources(
    conn: &Connection,
    workspace: Option<&str>,
    source: Option<&str>,
) -> Result<usize> {
    // SQL performs the set operation without materializing histories or source
    // text in Rust or plaintext files. Accept every UUID spelling understood by
    // the write guard (hyphenated, simple, braced and URN).
    conn.execute(
        "INSERT OR IGNORE INTO tombstones(workspace,id)
         SELECT v.workspace,v.id FROM revisions v JOIN document_tombstones d
         ON d.workspace=v.workspace AND d.kind='source'
         AND replace(replace(replace(replace(lower(json_extract(v.payload,'$.source.artifact_id')),'urn:uuid:',''),'-',''),'{',''),'}','')=replace(d.id,'-','')
         WHERE (?1 IS NULL OR d.workspace=?1) AND (?2 IS NULL OR d.id=?2)",
        params![workspace, source],
    )?;
    // Foreign keys cascade to revisions, request cache and matching index.
    Ok(conn.execute(
        "DELETE FROM records WHERE (?1 IS NULL OR workspace=?1)
         AND EXISTS(SELECT 1 FROM tombstones t WHERE t.workspace=records.workspace AND t.id=records.id)",
        [workspace],
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn payload(source: Uuid) -> Value {
        json!({"source":{"artifact_id":source},"domain":"test",
            "context":{"summary":"private","facts":[],"unknown_fields":[]},
            "candidates":[{"text":"A","attributes":{}},{"text":"B","attributes":{}}]})
    }
    fn count(store: &Store, table: &str, workspace: Uuid) -> usize {
        store
            .conn()
            .unwrap()
            .query_row(
                &format!("SELECT count(*) FROM {table} WHERE workspace=?1"),
                [workspace.to_string()],
                |r| r.get(0),
            )
            .unwrap()
    }

    #[test]
    fn cascade_erases_all_history_caches_and_derivatives_once_and_is_scoped() {
        let dir = tempfile::tempdir().unwrap();
        let key = Zeroizing::new("a".repeat(64));
        let mut s = Store::create(dir.path().join("vault"), &key).unwrap();
        let w = Uuid::new_v4();
        let other = Uuid::new_v4();
        let source = Uuid::new_v4();
        let record = Uuid::new_v4();
        for ws in [w, other] {
            s.create_workspace(ws).unwrap();
            s.put_document(DocumentKind::Source, ws, source, 0, Value::Null)
                .unwrap();
            s.propose(ws, record, Uuid::new_v4(), payload(source))
                .unwrap();
            s.confirm(ws, record, 1, Uuid::new_v4()).unwrap();
        }
        // A correction hiding the original source must not evade erasure.
        s.revise(w, record, 2, Uuid::new_v4(), payload(Uuid::new_v4()))
            .unwrap();
        s.confirm(w, record, 3, Uuid::new_v4()).unwrap();
        let second = Uuid::new_v4();
        let mut upper = payload(source);
        upper["source"]["artifact_id"] = source.to_string().to_uppercase().into();
        s.propose(w, second, Uuid::new_v4(), upper.clone()).unwrap();
        let backup = dir.path().join("backup");
        s.backup(&backup, &key).unwrap();
        let model = Uuid::new_v4();
        let epoch = s.epoch().unwrap();
        for ws in [w, other] {
            s.save_model(ws, model, epoch, Value::Null).unwrap();
            s.put_evolution_document(ws, Store::EVOLUTION_DOCUMENT_ID, 0, epoch, Value::Null)
                .unwrap();
        }
        assert!(count(&s, "matching_observations", w) > 0);
        assert_eq!(s.delete_source_cascade(w, source, 1).unwrap(), 2);
        assert_eq!(s.epoch().unwrap(), epoch + 1);
        for table in [
            "records",
            "revisions",
            "requests",
            "matching_observations",
            "models",
        ] {
            assert_eq!(count(&s, table, w), 0, "{table}");
            assert!(count(&s, table, other) > 0, "{table}");
        }
        assert!(
            s.get_document(DocumentKind::Improvement, w, Store::EVOLUTION_DOCUMENT_ID)
                .is_err()
        );
        assert!(
            s.get_document(
                DocumentKind::Improvement,
                other,
                Store::EVOLUTION_DOCUMENT_ID
            )
            .is_ok()
        );
        assert_eq!(count(&s, "tombstones", w), 2);
        assert!(matches!(
            s.propose(w, Uuid::new_v4(), Uuid::new_v4(), upper),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            s.propose(w, record, Uuid::new_v4(), Value::Null),
            Err(StoreError::NotFound)
        ));
        for spelling in [
            source.simple().to_string(),
            source.braced().to_string(),
            source.urn().to_string(),
        ] {
            let mut value = payload(source);
            value["source"]["artifact_id"] = spelling.into();
            assert!(matches!(
                s.propose(w, Uuid::new_v4(), Uuid::new_v4(), value),
                Err(StoreError::NotFound)
            ));
        }
        let unrelated = Uuid::new_v4();
        s.propose(w, unrelated, Uuid::new_v4(), Value::Null)
            .unwrap();
        assert!(matches!(
            s.revise(w, unrelated, 1, Uuid::new_v4(), payload(source)),
            Err(StoreError::NotFound)
        ));
        assert_eq!(s.get(w, unrelated).unwrap().revision, 1);
        // Source tombstones alone must discover the old backup's observations.
        let restored = Store::restore(
            &backup,
            &key,
            dir.path().join("restored"),
            &key,
            &[],
            &s.document_tombstones().unwrap(),
        )
        .unwrap();
        assert_eq!(count(&restored, "records", w), 0);
        assert_eq!(count(&restored, "tombstones", w), 2);
        assert!(restored.get(other, record).is_ok());
    }

    #[test]
    fn source_cas_and_mid_cascade_failure_roll_back_every_mutation() {
        let dir = tempfile::tempdir().unwrap();
        let mut s =
            Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
        let w = Uuid::new_v4();
        let source = Uuid::new_v4();
        let record = Uuid::new_v4();
        s.create_workspace(w).unwrap();
        s.put_document(DocumentKind::Source, w, source, 0, Value::Null)
            .unwrap();
        s.propose(w, record, Uuid::new_v4(), payload(source))
            .unwrap();
        s.confirm(w, record, 1, Uuid::new_v4()).unwrap();
        let epoch = s.epoch().unwrap();
        let model = Uuid::new_v4();
        s.save_model(w, model, epoch, Value::Null).unwrap();
        s.put_evolution_document(w, Store::EVOLUTION_DOCUMENT_ID, 0, epoch, Value::Null)
            .unwrap();
        assert!(matches!(
            s.delete_source_cascade(w, source, 2),
            Err(StoreError::RevisionConflict)
        ));
        assert!(matches!(
            s.delete_source_cascade(Uuid::new_v4(), source, 1),
            Err(StoreError::NotFound)
        ));
        // Failure after source, observation and index deletion, during invalidation.
        s.conn().unwrap().execute_batch("CREATE TRIGGER fail_delete BEFORE DELETE ON models BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(
            s.delete_document(DocumentKind::Source, w, source, 1)
                .is_err()
        );
        assert_eq!(s.epoch().unwrap(), epoch);
        assert_eq!(s.get(w, record).unwrap().revision, 2);
        assert!(s.get_document(DocumentKind::Source, w, source).is_ok());
        assert!(s.model(w, model).is_ok());
        assert!(
            s.get_document(DocumentKind::Improvement, w, Store::EVOLUTION_DOCUMENT_ID)
                .is_ok()
        );
        for table in ["tombstones", "document_tombstones"] {
            assert_eq!(count(&s, table, w), 0);
        }
        for table in ["revisions", "requests"] {
            assert_eq!(count(&s, table, w), 2);
        }
        assert_eq!(count(&s, "matching_observations", w), 1);
        s.conn()
            .unwrap()
            .execute_batch("DROP TRIGGER fail_delete")
            .unwrap();
        s.delete_document(DocumentKind::Source, w, source, 1)
            .unwrap();
        assert_eq!(count(&s, "records", w), 0);
        assert_eq!(s.epoch().unwrap(), epoch + 1);
    }
}

#[cfg(test)]
mod legacy_tests {
    use super::*;
    #[test]
    fn embedded_source_tombstone_blocks_confirmation_and_replay_and_survives_restore() {
        let dir = tempfile::tempdir().unwrap();
        let key = Zeroizing::new("a".repeat(64));
        let mut store = Store::create(dir.path().join("vault"), &key).unwrap();
        let w = Uuid::new_v4();
        let source = Uuid::new_v4();
        let id = Uuid::new_v4();
        let request = Uuid::new_v4();
        let payload = serde_json::json!({"source":{"artifact_id":source}});
        store.create_workspace(w).unwrap();
        store.propose(w, id, request, payload.clone()).unwrap();
        // Simulate a pre-cascade backup that retains labels of a deleted source.
        store
            .conn()
            .unwrap()
            .execute(
                "INSERT INTO document_tombstones VALUES(?1,'source',?2)",
                params![w.to_string(), source.to_string()],
            )
            .unwrap();
        let epoch = store.epoch().unwrap();
        assert!(matches!(
            store.confirm(w, id, 1, Uuid::new_v4()),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            store.propose(w, id, request, payload),
            Err(StoreError::NotFound)
        ));
        assert_eq!(store.epoch().unwrap(), epoch);
        let backup = dir.path().join("backup");
        store.backup(&backup, &key).unwrap();
        let restored =
            Store::restore(backup, &key, dir.path().join("restored"), &key, &[], &[]).unwrap();
        assert!(matches!(restored.get(w, id), Err(StoreError::NotFound)));
        assert_eq!(restored.tombstones().unwrap().len(), 1);
    }
}
