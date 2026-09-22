//! Derived semantic lookup inside the encrypted vault; never an authority for equality.
use super::*;

#[derive(Debug)]
pub struct MatchingRecords {
    pub records: Vec<Record>,
    /// All indexed candidates in this workspace, before the row/byte bounds.
    pub total: u64,
    pub truncated: bool,
}

/// Canonical retrieval key for a serialized rank request or observation proposal.
/// IDs, timestamps, response and provenance are deliberately excluded. Fact and
/// unknown-field order matters; candidate order does not, but multiplicity does.
/// Callers must still perform their typed exact equality and winner checks.
pub fn matching_signature(payload: &Value) -> Result<String> {
    let invalid = || StoreError::InvalidInput;
    let domain = payload
        .get("domain")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let context = payload.get("context").ok_or_else(invalid)?;
    let summary = context
        .get("summary")
        .and_then(Value::as_str)
        .ok_or_else(invalid)?;
    let facts = context
        .get("facts")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let unknown = context
        .get("unknown_fields")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let candidates = payload
        .get("candidates")
        .and_then(Value::as_array)
        .ok_or_else(invalid)?;
    let facts = facts
        .iter()
        .map(|fact| {
            Ok(serde_json::json!([
                fact.get("key")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?,
                fact_value(fact.get("value").ok_or_else(invalid)?)?,
                fact.get("evidence_status")
                    .and_then(Value::as_str)
                    .ok_or_else(invalid)?,
                fact.get("unit").unwrap_or(&Value::Null)
            ]))
        })
        .collect::<Result<Vec<_>>>()?;
    let mut candidates = candidates
        .iter()
        .map(|candidate| {
            let text = candidate
                .get("text")
                .and_then(Value::as_str)
                .ok_or_else(invalid)?;
            let attrs = candidate
                .get("attributes")
                .and_then(Value::as_object)
                .ok_or_else(invalid)?;
            let attrs = attrs
                .iter()
                .map(|(key, value)| Ok((key, fact_value(value)?)))
                .collect::<Result<std::collections::BTreeMap<_, _>>>()?;
            Ok(serde_json::to_string(&(text, attrs))?)
        })
        .collect::<Result<Vec<_>>>()?;
    candidates.sort_unstable();
    let signature = serde_json::to_string(&(domain, summary, facts, unknown, candidates))?;
    if signature.len() > 1024 * 1024 {
        return Err(StoreError::InvalidInput);
    }
    Ok(signature)
}

// Domain FactValue parses scalar numbers as f64, including integer JSON input.
// Keep money objects intact (integer minor units). Normalize signed zero because
// typed fact equality treats -0.0 == 0.0. Extra candidates remain app-checked.
fn fact_value(value: &Value) -> Result<Value> {
    if let Value::Number(number) = value {
        let number = number.as_f64().ok_or(StoreError::InvalidInput)?;
        return Ok(serde_json::json!(if number == 0.0 { 0.0 } else { number }));
    }
    Ok(value.clone())
}

pub(super) fn sync_record(
    conn: &Connection,
    workspace: &str,
    id: &str,
    revision: u64,
    status: Status,
    payload: &Value,
) -> Result<()> {
    conn.execute(
        "DELETE FROM matching_observations WHERE workspace=?1 AND id=?2",
        params![workspace, id],
    )?;
    if status == Status::Confirmed {
        // The general-purpose store also accepts non-observation payloads.
        match matching_signature(payload) {
            Ok(signature) => {
                conn.execute("INSERT INTO matching_observations(workspace,id,revision,signature) VALUES(?1,?2,?3,?4)", params![workspace, id, revision, signature])?;
            }
            Err(StoreError::InvalidInput) => {}
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

const LOOKUP: &str = "SELECT m.id,m.revision,v.payload FROM matching_observations m INDEXED BY matching_observations_lookup JOIN revisions v ON v.workspace=m.workspace AND v.id=m.id AND v.revision=m.revision WHERE m.workspace=?1 AND m.signature=?2 ORDER BY m.id LIMIT ?3";

impl Store {
    pub(super) fn initialize_matching_index(&mut self) -> Result<()> {
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let exists: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='matching_observations')", [], |r| r.get(0))?;
        if !exists {
            tx.execute_batch("CREATE TABLE matching_observations(workspace TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL,signature TEXT NOT NULL,PRIMARY KEY(workspace,id),FOREIGN KEY(workspace,id) REFERENCES records(workspace,id) ON DELETE CASCADE);
                CREATE INDEX matching_observations_lookup ON matching_observations(workspace,signature,id);")?;
            // Stream one current payload at a time. DDL and the entire backfill
            // commit together; a failed or interrupted migration leaves no marker.
            let mut statement = tx.prepare("SELECT v.workspace,v.id,v.revision,v.payload FROM records r JOIN revisions v ON v.workspace=r.workspace AND v.id=r.id AND v.revision=r.revision WHERE v.status='confirmed' AND NOT EXISTS(SELECT 1 FROM tombstones t WHERE t.workspace=r.workspace AND t.id=r.id)")?;
            let mut rows = statement.query([])?;
            while let Some(row) = rows.next()? {
                let workspace: String = row.get(0)?;
                let id: String = row.get(1)?;
                let revision: u64 = row.get(2)?;
                let payload: String = row.get(3)?;
                sync_record(
                    &tx,
                    &workspace,
                    &id,
                    revision,
                    Status::Confirmed,
                    &serde_json::from_str(&payload)?,
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// Bounded indexed lookup with an exact indexed candidate count. Both queries
    /// share a read snapshot so a concurrent writer cannot hide truncation.
    /// Truncation means callers cannot conclude that observed winners are complete.
    pub fn matching_records(
        &self,
        workspace: Uuid,
        signature: &str,
        limit: u32,
    ) -> Result<MatchingRecords> {
        if limit == 0 || limit > 1000 || signature.len() > 1024 * 1024 {
            return Err(StoreError::InvalidInput);
        }
        let tx = self.conn()?.unchecked_transaction()?;
        let workspace_text = workspace.to_string();
        let total: u64 = tx.query_row("SELECT count(*) FROM matching_observations INDEXED BY matching_observations_lookup WHERE workspace=?1 AND signature=?2", params![workspace_text, signature], |r| r.get(0))?;
        let mut records = Vec::new();
        let mut bytes = 0usize;
        {
            let mut statement = tx.prepare(LOOKUP)?;
            let mut rows = statement.query(params![workspace_text, signature, limit])?;
            while let Some(row) = rows.next()? {
                let payload: String = row.get(2)?;
                bytes = bytes
                    .checked_add(payload.len())
                    .ok_or(StoreError::InvalidInput)?;
                if bytes > 16 * 1024 * 1024 {
                    break;
                }
                let id: String = row.get(0)?;
                records.push(Record {
                    workspace_id: workspace,
                    id: Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
                    revision: row.get(1)?,
                    status: Status::Confirmed,
                    payload: serde_json::from_str(&payload)?,
                });
            }
        }
        tx.commit()?;
        Ok(MatchingRecords {
            truncated: (records.len() as u64) < total,
            records,
            total,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload() -> Value {
        serde_json::from_str(include_str!("../../../contracts/fixtures/rank-valid.json")).unwrap()
    }
    fn confirmed(store: &mut Store, w: Uuid, value: Value) -> Uuid {
        let id = Uuid::new_v4();
        store.propose(w, id, Uuid::new_v4(), value).unwrap();
        store.confirm(w, id, 1, Uuid::new_v4()).unwrap();
        id
    }
    #[test]
    fn semantic_signature_matches_typed_semantics() {
        let original = payload();
        let signature = matching_signature(&original).unwrap();
        let mut changed = original.clone();
        changed["context"]["as_of"] = "2030-01-01T00:00:00Z".into();
        changed["candidates"].as_array_mut().unwrap().reverse();
        changed["candidates"][0]["id"] = "new-id".into();
        changed["response"] = serde_json::json!({"candidate_id":"anything"});
        assert_eq!(matching_signature(&changed).unwrap(), signature);
        for path in ["summary", "unknown_fields", "facts"] {
            let mut different = original.clone();
            if path == "summary" {
                different["context"][path] = "different".into();
            } else {
                different["context"][path]
                    .as_array_mut()
                    .unwrap()
                    .push(Value::Null);
            }
            assert_ne!(matching_signature(&different).ok(), Some(signature.clone()));
        }
        let mut different = original.clone();
        different["candidates"][0]["text"] = "different".into();
        assert_ne!(matching_signature(&different).unwrap(), signature);
        let mut different = original.clone();
        different["candidates"].as_array_mut().unwrap().pop();
        assert_ne!(matching_signature(&different).unwrap(), signature);
        let mut numeric = original;
        numeric["context"]["facts"] =
            serde_json::json!([{"key":"n","value":1,"evidence_status":"explicit"}]);
        numeric["candidates"][0]["attributes"] = serde_json::json!({"n":1});
        let signature = matching_signature(&numeric).unwrap();
        numeric["context"]["facts"][0]["value"] = serde_json::json!(1.0);
        numeric["context"]["facts"][0]["unit"] = Value::Null;
        numeric["candidates"][0]["attributes"]["n"] = serde_json::json!(1.0);
        assert_eq!(matching_signature(&numeric).unwrap(), signature);
    }
    #[test]
    fn mutations_backfill_restore_and_tenant_isolation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault");
        let key = Zeroizing::new("a".repeat(64));
        let mut store = Store::create(&path, &key).unwrap();
        let w = Uuid::new_v4();
        let other = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        store.create_workspace(other).unwrap();
        let value = payload();
        let signature = matching_signature(&value).unwrap();
        let id = confirmed(&mut store, w, value.clone());
        confirmed(&mut store, other, value.clone());
        assert_eq!(store.matching_records(w, &signature, 10).unwrap().total, 1);
        store
            .revise(w, id, 2, Uuid::new_v4(), value.clone())
            .unwrap();
        assert_eq!(store.matching_records(w, &signature, 10).unwrap().total, 0);
        assert_eq!(
            read_record(store.conn().unwrap(), w, id, Some(2))
                .unwrap()
                .status,
            Status::Confirmed
        );
        let request = Uuid::new_v4();
        store.confirm(w, id, 3, request).unwrap();
        let epoch = store.epoch().unwrap();
        store.confirm(w, id, 3, request).unwrap();
        assert_eq!(store.epoch().unwrap(), epoch);
        // Old-v1 fixture: absence of the derived table is the migration marker.
        store
            .conn()
            .unwrap()
            .execute_batch("DROP TABLE matching_observations")
            .unwrap();
        store.lock();
        let mut store = Store::open(&path, &key).unwrap();
        assert_eq!(store.epoch().unwrap(), epoch);
        assert_eq!(
            store.matching_records(w, &signature, 10).unwrap().records[0].revision,
            4
        );
        store.lock();
        let mut store = Store::open(&path, &key).unwrap();
        assert_eq!(store.matching_records(w, &signature, 10).unwrap().total, 1);
        let backup = dir.path().join("backup");
        store.backup(&backup, &key).unwrap();
        store.delete(w, id, 4).unwrap();
        assert_eq!(store.matching_records(w, &signature, 10).unwrap().total, 0);
        let restored = Store::restore(
            &backup,
            &key,
            dir.path().join("restored"),
            &key,
            &store.tombstones().unwrap(),
            &[],
        )
        .unwrap();
        assert_eq!(
            restored.matching_records(w, &signature, 10).unwrap().total,
            0
        );
        assert_eq!(
            restored
                .matching_records(other, &signature, 10)
                .unwrap()
                .total,
            1
        );
        assert!(store.matching_records(w, &signature, 0).is_err());
        assert!(store.matching_records(w, &signature, 1001).is_err());
        store.lock();
        assert!(matches!(
            store.matching_records(w, &signature, 1),
            Err(StoreError::Locked)
        ));
    }
    #[test]
    fn ten_thousand_unrelated_rows_use_index_and_preserve_conflicting_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
        let w = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        let value = payload();
        let signature = matching_signature(&value).unwrap();
        // Bulk fixture avoids 20k fsyncs; uses the same transactional index writer.
        let tx = store.conn_mut().unwrap().transaction().unwrap();
        for index in 0..10_000 {
            let id = Uuid::new_v4().to_string();
            let mut unrelated = value.clone();
            unrelated["context"]["summary"] = format!("unrelated-{index}").into();
            tx.execute(
                "INSERT INTO records VALUES(?1,?2,1)",
                params![w.to_string(), id],
            )
            .unwrap();
            tx.execute(
                "INSERT INTO revisions VALUES(?1,?2,1,'confirmed',?3)",
                params![
                    w.to_string(),
                    id,
                    serde_json::to_string(&unrelated).unwrap()
                ],
            )
            .unwrap();
            sync_record(&tx, &w.to_string(), &id, 1, Status::Confirmed, &unrelated).unwrap();
        }
        tx.commit().unwrap();
        for winner in ["repair_first", "ship_first"] {
            let mut observation = value.clone();
            observation["response"] =
                serde_json::json!({"response_type":"choose_one","candidate_id":winner});
            confirmed(&mut store, w, observation);
        }
        let result = store.matching_records(w, &signature, 10).unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.records.len(), 2);
        assert!(!result.truncated);
        assert_ne!(
            result.records[0].payload["response"],
            result.records[1].payload["response"]
        );
        let result = store.matching_records(w, &signature, 1).unwrap();
        assert_eq!(result.total, 2);
        assert_eq!(result.records.len(), 1);
        assert!(result.truncated);
        let plan = store
            .conn()
            .unwrap()
            .prepare(&format!("EXPLAIN QUERY PLAN {LOOKUP}"))
            .unwrap()
            .query_map(params![w.to_string(), signature, 10], |r| {
                r.get::<_, String>(3)
            })
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert!(
            plan.iter().any(|s| s.contains(
                "SEARCH m USING INDEX matching_observations_lookup (workspace=? AND signature=?)"
            )),
            "{plan:?}"
        );
        assert!(
            !plan
                .iter()
                .any(|s| s.contains("SCAN") || s.contains("TEMP B-TREE")),
            "{plan:?}"
        );
    }
    #[test]
    fn unreadable_json_never_commits_at_any_payload_entry() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
        let w = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        let mut deep = Value::Null;
        for _ in 0..150 {
            deep = serde_json::json!([deep]);
        }
        let epoch = store.epoch().unwrap();
        let id = Uuid::new_v4();
        assert!(store.propose(w, id, Uuid::new_v4(), deep.clone()).is_err());
        assert!(
            store
                .put_document(DocumentKind::Source, w, id, 0, deep.clone())
                .is_err()
        );
        assert!(
            store
                .insert_documents_batch(DocumentKind::Source, w, vec![(id, deep.clone())])
                .is_err()
        );
        assert!(store.save_model(w, id, epoch, deep.clone()).is_err());
        assert!(
            store
                .put_evolution_document(w, Store::EVOLUTION_DOCUMENT_ID, 0, epoch, deep)
                .is_err()
        );
        assert_eq!(store.epoch().unwrap(), epoch);
        assert!(store.list(w, 10, 0).unwrap().is_empty());
        assert!(
            store
                .list_documents(DocumentKind::Source, w, 10, 0)
                .unwrap()
                .is_empty()
        );
        assert!(store.list_models(w, 10).unwrap().is_empty());
    }
    #[test]
    fn backfill_failure_rolls_back_schema_and_write_failure_rolls_back_revision() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault");
        let key = Zeroizing::new("a".repeat(64));
        let mut store = Store::create(&path, &key).unwrap();
        let w = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        let id = confirmed(&mut store, w, payload());
        let epoch = store.epoch().unwrap();
        let signature = matching_signature(&payload()).unwrap();
        store.conn().unwrap().execute_batch("CREATE TRIGGER reject_matching BEFORE DELETE ON matching_observations BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(store.revise(w, id, 2, Uuid::new_v4(), payload()).is_err());
        assert_eq!(store.get(w, id).unwrap().revision, 2);
        assert_eq!(store.epoch().unwrap(), epoch);
        assert_eq!(store.matching_records(w, &signature, 10).unwrap().total, 1);
        store.conn().unwrap().execute_batch("DROP TABLE matching_observations; UPDATE revisions SET payload='invalid' WHERE status='confirmed';").unwrap();
        store.lock();
        assert!(Store::open(&path, &key).is_err());
        let raw = Store::open_raw(&path, &key).unwrap();
        let table_exists: bool = raw
            .conn()
            .unwrap()
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='matching_observations')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(!table_exists);
        assert_eq!(raw.epoch().unwrap(), epoch);
        raw.conn()
            .unwrap()
            .execute(
                "UPDATE revisions SET payload=?1 WHERE status='confirmed'",
                [serde_json::to_string(&payload()).unwrap()],
            )
            .unwrap();
        drop(raw);
        let store = Store::open(&path, &key).unwrap();
        assert_eq!(store.matching_records(w, &signature, 10).unwrap().total, 1);
    }
    #[test]
    fn byte_budget_reports_truncation_without_deserializing_all_payloads() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
        let w = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        let signature = matching_signature(&payload()).unwrap();
        let mut large = payload();
        large["unindexed_metadata"] = Value::String("x".repeat(900_000));
        for _ in 0..20 {
            confirmed(&mut store, w, large.clone());
        }
        let result = store.matching_records(w, &signature, 1000).unwrap();
        assert_eq!(result.total, 20);
        assert_eq!(result.records.len(), 18);
        assert!(result.truncated);
    }
    #[test]
    fn input_changes_purge_evolution_snapshots_only_in_own_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
        let w = Uuid::new_v4();
        let other = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        store.create_workspace(other).unwrap();
        let id = confirmed(&mut store, w, payload());
        let derived = Store::EVOLUTION_DOCUMENT_ID;
        let epoch = store.epoch().unwrap();
        for workspace in [w, other] {
            store
                .put_evolution_document(
                    workspace,
                    derived,
                    0,
                    epoch,
                    serde_json::json!({"source":"private input"}),
                )
                .unwrap();
        }
        assert_eq!(store.epoch().unwrap(), epoch);
        store.revise(w, id, 2, Uuid::new_v4(), payload()).unwrap();
        assert!(matches!(
            store.get_document(DocumentKind::Improvement, w, derived),
            Err(StoreError::NotFound)
        ));
        assert!(
            store
                .get_document(DocumentKind::Improvement, other, derived)
                .is_ok()
        );
        store
            .put_evolution_document(w, derived, 0, store.epoch().unwrap(), Value::Null)
            .unwrap();
        store.delete(w, id, 3).unwrap();
        assert!(matches!(
            store.get_document(DocumentKind::Improvement, w, derived),
            Err(StoreError::NotFound)
        ));
        store
            .put_evolution_document(w, derived, 0, store.epoch().unwrap(), Value::Null)
            .unwrap();
        store
            .put_document(DocumentKind::Source, w, Uuid::new_v4(), 0, Value::Null)
            .unwrap();
        assert!(matches!(
            store.get_document(DocumentKind::Improvement, w, derived),
            Err(StoreError::NotFound)
        ));
    }
}
