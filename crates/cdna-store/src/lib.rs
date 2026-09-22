//! Encrypted local persistence. Trust authorization belongs to the local application boundary.
use rusqlite::{Connection, OpenFlags, OptionalExtension, params};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{path::Path, time::Duration};
use uuid::Uuid;
use zeroize::Zeroizing;

mod matching;
mod coaching;
mod source_deletion;
pub use matching::{MatchingRecords, matching_signature};

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("vault locked")]
    Locked,
    #[error("revision conflict")]
    RevisionConflict,
    #[error("grant denied")]
    GrantDenied,
    #[error("record not found")]
    NotFound,
    #[error("invalid input")]
    InvalidInput,
    #[error("request ID reused for a different operation")]
    RequestConflict,
    #[error("OS credential store unavailable or credential missing")]
    Keychain,
    #[error("storage failure")]
    Database(#[from] rusqlite::Error),
    #[error("filesystem failure")]
    Io(#[from] std::io::Error),
    #[error("invalid JSON")]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, StoreError>;
// Match the reader's JSON depth limit before entering a write transaction.
fn readable_json(value: &Value) -> Result<String> {
    let serialized = serde_json::to_string(value)?;
    if serialized.len() > 1024 * 1024 {
        return Err(StoreError::InvalidInput);
    }
    let _: Value = serde_json::from_str(&serialized)?;
    Ok(serialized)
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    Pending,
    Confirmed,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub workspace_id: Uuid,
    pub id: Uuid,
    pub revision: u64,
    pub status: Status,
    pub payload: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tombstone {
    pub workspace_id: Uuid,
    pub id: Uuid,
}
pub struct Store {
    connection: Option<Connection>,
}

impl Store {
    fn conn(&self) -> Result<&Connection> {
        self.connection.as_ref().ok_or(StoreError::Locked)
    }
    fn conn_mut(&mut self) -> Result<&mut Connection> {
        self.connection.as_mut().ok_or(StoreError::Locked)
    }
    pub fn create(path: impl AsRef<Path>, key: &Zeroizing<String>) -> Result<Self> {
        if key.len() < 32 {
            return Err(StoreError::InvalidInput);
        }
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        drop(options.open(path.as_ref())?);
        let mut store = Self::open_raw(path.as_ref(), key)?;
        let tx = store.conn_mut()?.transaction()?;
        tx.execute_batch("CREATE TABLE metadata(version INTEGER NOT NULL, epoch INTEGER NOT NULL); INSERT INTO metadata VALUES(1,0);
        CREATE TABLE workspaces(id TEXT PRIMARY KEY, name TEXT NOT NULL DEFAULT 'Workspace');
        CREATE TABLE documents(workspace TEXT NOT NULL REFERENCES workspaces(id),kind TEXT NOT NULL,id TEXT NOT NULL,revision INTEGER NOT NULL CHECK(revision>0),payload TEXT NOT NULL,PRIMARY KEY(workspace,kind,id));
        CREATE TABLE document_tombstones(workspace TEXT NOT NULL,kind TEXT NOT NULL,id TEXT NOT NULL,PRIMARY KEY(workspace,kind,id));
        CREATE TABLE grants(id TEXT PRIMARY KEY,workspace TEXT NOT NULL REFERENCES workspaces(id), scopes TEXT NOT NULL,expires_at INTEGER NOT NULL,revoked INTEGER NOT NULL CHECK(revoked IN (0,1)),token_hash TEXT NOT NULL UNIQUE);
        CREATE TABLE records(workspace TEXT NOT NULL REFERENCES workspaces(id), id TEXT NOT NULL, revision INTEGER NOT NULL CHECK(revision>0), PRIMARY KEY(workspace,id));
        CREATE TABLE revisions(workspace TEXT NOT NULL, id TEXT NOT NULL, revision INTEGER NOT NULL, status TEXT NOT NULL CHECK(status IN ('pending','confirmed')), payload TEXT NOT NULL, PRIMARY KEY(workspace,id,revision), FOREIGN KEY(workspace,id) REFERENCES records(workspace,id) ON DELETE CASCADE);
        CREATE TABLE requests(workspace TEXT NOT NULL, request TEXT NOT NULL, operation TEXT NOT NULL, id TEXT NOT NULL, revision INTEGER NOT NULL, PRIMARY KEY(workspace,request), FOREIGN KEY(workspace,id) REFERENCES records(workspace,id) ON DELETE CASCADE);
        CREATE TABLE tombstones(workspace TEXT NOT NULL, id TEXT NOT NULL, PRIMARY KEY(workspace,id));
        CREATE TABLE models(workspace TEXT NOT NULL REFERENCES workspaces(id), id TEXT NOT NULL, epoch INTEGER NOT NULL, payload TEXT NOT NULL, PRIMARY KEY(workspace,id));")?;
        tx.commit()?;
        store.initialize_matching_index()?;
        store.initialize_coaching()?;
        Ok(store)
    }
    fn open_raw(path: &Path, key: &Zeroizing<String>) -> Result<Self> {
        if key.len() < 32 {
            return Err(StoreError::InvalidInput);
        }
        let conn = Connection::open_with_flags(
            path,
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_NO_MUTEX,
        )?;
        conn.pragma_update(None, "key", key.as_str())?;
        let cipher: String = conn.pragma_query_value(None, "cipher_version", |r| r.get(0))?;
        if cipher.is_empty() {
            return Err(StoreError::InvalidInput);
        }
        conn.query_row("SELECT count(*) FROM sqlite_master", [], |_| Ok(()))?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA secure_delete=ON; PRAGMA temp_store=MEMORY; PRAGMA journal_mode=DELETE;")?;
        Ok(Self {
            connection: Some(conn),
        })
    }
    pub fn open(path: impl AsRef<Path>, key: &Zeroizing<String>) -> Result<Self> {
        let mut store = Self::open_raw(path.as_ref(), key)?;
        let version: i64 = store
            .conn()?
            .query_row("SELECT version FROM metadata", [], |r| r.get(0))?;
        if version != 1 {
            return Err(StoreError::InvalidInput);
        }
        store.initialize_matching_index()?;
        store.initialize_coaching()?;
        Ok(store)
    }
    pub fn lock(&mut self) {
        self.connection.take();
    }
    pub fn create_workspace(&self, id: Uuid) -> Result<()> {
        self.conn()?.execute(
            "INSERT OR IGNORE INTO workspaces(id) VALUES(?1)",
            [id.to_string()],
        )?;
        Ok(())
    }
    pub fn epoch(&self) -> Result<u64> {
        Ok(self
            .conn()?
            .query_row("SELECT epoch FROM metadata", [], |r| r.get(0))?)
    }
    pub fn propose(
        &mut self,
        workspace: Uuid,
        id: Uuid,
        request: Uuid,
        payload: Value,
    ) -> Result<Record> {
        self.write(workspace, id, 0, request, Some(payload), false, None)
    }
    pub fn revise(
        &mut self,
        workspace: Uuid,
        id: Uuid,
        expected: u64,
        request: Uuid,
        payload: Value,
    ) -> Result<Record> {
        if expected == 0 {
            return Err(StoreError::InvalidInput);
        }
        self.write(workspace, id, expected, request, Some(payload), false, None)
    }
    /// Only the trusted user-facing host may invoke this operation after explicit confirmation.
    pub fn confirm(
        &mut self,
        workspace: Uuid,
        id: Uuid,
        expected: u64,
        request: Uuid,
    ) -> Result<Record> {
        if expected == 0 {
            return Err(StoreError::InvalidInput);
        }
        self.write(workspace, id, expected, request, None, true, None)
    }
    #[allow(clippy::too_many_arguments)]
    fn write(
        &mut self,
        w: Uuid,
        id: Uuid,
        expected: u64,
        req: Uuid,
        payload: Option<Value>,
        confirm: bool,
        coaching: Option<(Uuid, Uuid)>,
    ) -> Result<Record> {
        if expected >= i64::MAX as u64 {
            return Err(StoreError::InvalidInput);
        }
        if let Some(value) = &payload {
            readable_json(value)?;
        }
        let operation = serde_json::to_string(&(id, expected, confirm, &payload))?;
        let operation = if let Some(origin)=coaching { serde_json::to_string(&(operation,origin))? } else {operation};
        if operation.len() > 1024 * 1024 {
            return Err(StoreError::InvalidInput);
        }
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ws = w.to_string();
        let rid = id.to_string();
        let prior: Option<(String, u64)> = tx
            .query_row(
                "SELECT operation,revision FROM requests WHERE workspace=?1 AND request=?2",
                params![ws, req.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        if let Some((op, rev)) = prior {
            if op != operation {
                return Err(StoreError::RequestConflict);
            }
            let record = read_record(&tx, w, id, Some(rev))?;
            source_deletion::check_source(&tx, &ws, &record.payload)?;
            return Ok(record);
        }
        let deleted: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM tombstones WHERE workspace=?1 AND id=?2)",
            params![ws, rid],
            |r| r.get(0),
        )?;
        if deleted {
            return Err(StoreError::NotFound);
        }
        let current: Option<u64> = tx
            .query_row(
                "SELECT revision FROM records WHERE workspace=?1 AND id=?2",
                params![ws, rid],
                |r| r.get(0),
            )
            .optional()?;
        if current.unwrap_or(0) != expected {
            return Err(StoreError::RevisionConflict);
        }
        let value = match payload {
            Some(p) => p,
            None => {
                let r = read_record(&tx, w, id, None)?;
                if r.status != Status::Pending {
                    return Err(StoreError::InvalidInput);
                }
                r.payload
            }
        };
        source_deletion::check_source(&tx, &ws, &value)?;
        let revision = expected + 1;
        let status = if confirm {
            Status::Confirmed
        } else {
            Status::Pending
        };
        if expected == 0 {
            tx.execute(
                "INSERT INTO records VALUES(?1,?2,?3)",
                params![ws, rid, revision],
            )?;
        } else {
            tx.execute(
                "UPDATE records SET revision=?3 WHERE workspace=?1 AND id=?2",
                params![ws, rid, revision],
            )?;
        }
        tx.execute(
            "INSERT INTO revisions VALUES(?1,?2,?3,?4,?5)",
            params![
                ws,
                rid,
                revision,
                if confirm { "confirmed" } else { "pending" },
                serde_json::to_string(&value)?
            ],
        )?;
        tx.execute(
            "INSERT INTO requests VALUES(?1,?2,?3,?4,?5)",
            params![ws, req.to_string(), operation, rid, revision],
        )?;
        matching::sync_record(&tx, &ws, &rid, revision, status, &value)?;
        if let Some((batch,question))=coaching {
            tx.execute("INSERT INTO coaching_answers VALUES(?1,?2,?3,?4,?5)",params![ws,req.to_string(),batch.to_string(),question.to_string(),rid])?;
        }
        // Every mutation invalidates workspace derivatives; corrections advance the vault epoch.
        invalidate_workspace(&tx, &ws)?;
        tx.execute("UPDATE metadata SET epoch=epoch+1", [])?;
        tx.commit()?;
        Ok(Record {
            workspace_id: w,
            id,
            revision,
            status,
            payload: value,
        })
    }
    pub fn get(&self, w: Uuid, id: Uuid) -> Result<Record> {
        read_record(self.conn()?, w, id, None)
    }
    pub fn list(&self, w: Uuid, limit: u32, offset: u32) -> Result<Vec<Record>> {
        if limit == 0 || limit > 1000 || offset > 1_000_000 {
            return Err(StoreError::InvalidInput);
        }
        let mut stmt = self
            .conn()?
            .prepare("SELECT id FROM records WHERE workspace=?1 ORDER BY id LIMIT ?2 OFFSET ?3")?;
        let ids = stmt
            .query_map(params![w.to_string(), limit, offset], |r| {
                r.get::<_, String>(0)
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        let mut records = Vec::new();
        let mut bytes = 0;
        for id in ids {
            let record = self.get(
                w,
                Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
            )?;
            bytes += serde_json::to_vec(&record)?.len();
            if bytes > 16 * 1024 * 1024 {
                return Err(StoreError::InvalidInput);
            }
            records.push(record);
        }
        Ok(records)
    }
    /// Current confirmed revisions, ordered by record ID. A short page may be
    /// byte-limited; consumers continue from the last ID until an empty page.
    /// Snapshot consumers must also compare the vault epoch before and after.
    pub fn confirmed_page(&self, w: Uuid, after: Option<Uuid>, limit: u32) -> Result<Vec<Record>> {
        if limit == 0 || limit > 256 { return Err(StoreError::InvalidInput); }
        let mut statement = self.conn()?.prepare(
            "SELECT r.id,r.revision,v.payload FROM records r JOIN revisions v ON v.workspace=r.workspace AND v.id=r.id AND v.revision=r.revision WHERE r.workspace=?1 AND r.id>?2 AND v.status='confirmed' ORDER BY r.id LIMIT ?3"
        )?;
        let mut rows = statement.query(params![w.to_string(), after.map(|id|id.to_string()).unwrap_or_default(), limit])?;
        let mut records = Vec::new();
        let mut bytes = 0usize;
        while let Some(row) = rows.next()? {
            let payload: String = row.get(2)?;
            if payload.len() > 1024*1024 { return Err(StoreError::InvalidInput); }
            if bytes + payload.len() > 8*1024*1024 { break; }
            bytes += payload.len();
            records.push(Record {
                workspace_id: w,
                id: Uuid::parse_str(&row.get::<_, String>(0)?).map_err(|_|StoreError::InvalidInput)?,
                revision: row.get(1)?,
                status: Status::Confirmed,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(records)
    }
    pub fn delete(&mut self, w: Uuid, id: Uuid, expected: u64) -> Result<()> {
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let record = read_record(&tx, w, id, None)?;
        if record.revision != expected {
            return Err(StoreError::RevisionConflict);
        }
        purge(
            &tx,
            &Tombstone {
                workspace_id: w,
                id,
            },
        )?;
        tx.commit()?;
        Ok(())
    }
    pub fn tombstones(&self) -> Result<Vec<Tombstone>> {
        let mut stmt = self
            .conn()?
            .prepare("SELECT workspace,id FROM tombstones")?;
        let pairs = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        pairs
            .into_iter()
            .map(|(w, id)| {
                Ok(Tombstone {
                    workspace_id: Uuid::parse_str(&w).map_err(|_| StoreError::InvalidInput)?,
                    id: Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
                })
            })
            .collect()
    }
    pub fn save_model(
        &mut self,
        w: Uuid,
        id: Uuid,
        expected_epoch: u64,
        payload: Value,
    ) -> Result<()> {
        let payload = readable_json(&payload)?;
        if payload.len() > 1024 * 1024 {
            return Err(StoreError::InvalidInput);
        }
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let epoch: u64 = tx.query_row("SELECT epoch FROM metadata", [], |r| r.get(0))?;
        if epoch != expected_epoch {
            return Err(StoreError::RevisionConflict);
        }
        tx.execute("INSERT INTO models VALUES(?1,?2,?3,?4) ON CONFLICT(workspace,id) DO UPDATE SET epoch=excluded.epoch,payload=excluded.payload",params![w.to_string(),id.to_string(),epoch,payload])?;
        tx.commit()?;
        Ok(())
    }
    pub fn model(&self, w: Uuid, id: Uuid) -> Result<Value> {
        let value:String=self.conn()?.query_row("SELECT payload FROM models WHERE workspace=?1 AND id=?2 AND epoch=(SELECT epoch FROM metadata)",params![w.to_string(),id.to_string()],|r|r.get(0)).optional()?.ok_or(StoreError::NotFound)?;
        Ok(serde_json::from_str(&value)?)
    }
    pub fn export_jsonl(&self, w: Uuid, limit: u32, offset: u32) -> Result<String> {
        let mut output = String::new();
        for record in self.list(w, limit, offset)? {
            output.push_str(&serde_json::to_string(&record)?);
            output.push('\n');
        }
        Ok(output)
    }
    fn encrypted_temporary(
        &self,
        path: &Path,
        key: &Zeroizing<String>,
    ) -> Result<tempfile::NamedTempFile> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        let temporary = tempfile::Builder::new()
            .prefix(".cdna-pending-")
            .tempfile_in(parent)?;
        let mut destination = Self::open_raw(temporary.path(), key)?;
        {
            let backup = rusqlite::backup::Backup::new(self.conn()?, destination.conn_mut()?)?;
            backup.run_to_completion(128, Duration::from_millis(1), None)?;
        }
        destination.check_integrity()?;
        destination.lock();
        temporary.as_file().sync_all()?;
        Ok(temporary)
    }
    fn check_integrity(&self) -> Result<()> {
        let integrity: String = self
            .conn()?
            .query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        if integrity != "ok"
            || self
                .conn()?
                .prepare("PRAGMA foreign_key_check")?
                .exists([])?
        {
            return Err(StoreError::InvalidInput);
        }
        Ok(())
    }
    pub fn backup(&self, path: impl AsRef<Path>, recovery_key: &Zeroizing<String>) -> Result<()> {
        let temporary = self.encrypted_temporary(path.as_ref(), recovery_key)?;
        temporary
            .persist_noclobber(path.as_ref())
            .map_err(|e| StoreError::Io(e.error))?;
        Ok(())
    }
    pub fn restore(
        backup: impl AsRef<Path>,
        recovery_key: &Zeroizing<String>,
        new_path: impl AsRef<Path>,
        new_key: &Zeroizing<String>,
        known_tombstones: &[Tombstone],
        known_documents: &[DocumentTombstone],
    ) -> Result<Self> {
        let source = Self::open(backup, recovery_key)?;
        source.check_integrity()?;
        let temporary = source.encrypted_temporary(new_path.as_ref(), new_key)?;
        let mut restored = Self::open(temporary.path(), new_key)?;
        let tx = restored
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        for tombstone in known_tombstones {
            purge(&tx, tombstone)?;
        }
        for tombstone in known_documents {
            tx.execute(
                "DELETE FROM documents WHERE workspace=?1 AND kind=?2 AND id=?3",
                params![
                    tombstone.workspace_id.to_string(),
                    tombstone.kind.as_str(),
                    tombstone.id.to_string()
                ],
            )?;
            tx.execute(
                "INSERT OR IGNORE INTO document_tombstones VALUES(?1,?2,?3)",
                params![
                    tombstone.workspace_id.to_string(),
                    tombstone.kind.as_str(),
                    tombstone.id.to_string()
                ],
            )?;
        }
        source_deletion::purge_tombstoned_sources(&tx)?;
        if !known_documents.is_empty() {
            tx.execute("UPDATE metadata SET epoch=epoch+1", [])?;
        }
        tx.execute(
            "DELETE FROM documents WHERE id=?1",
            [Self::EVOLUTION_DOCUMENT_ID.to_string()],
        )?;
        tx.execute("DELETE FROM models", [])?;
        tx.execute("DELETE FROM documents WHERE kind='coaching'", [])?;
        tx.execute("UPDATE grants SET revoked=1", [])?;
        tx.commit()?;
        restored.check_integrity()?;
        restored.lock();
        temporary.as_file().sync_all()?;
        temporary
            .persist_noclobber(new_path.as_ref())
            .map_err(|e| StoreError::Io(e.error))?;
        Self::open(new_path, new_key)
    }
}
fn read_record(conn: &Connection, w: Uuid, id: Uuid, revision: Option<u64>) -> Result<Record> {
    let row:Option<(u64,String,String)>=conn.query_row("SELECT v.revision,v.status,v.payload FROM revisions v JOIN records r ON r.workspace=v.workspace AND r.id=v.id WHERE v.workspace=?1 AND v.id=?2 AND v.revision=COALESCE(?3,r.revision)",params![w.to_string(),id.to_string(),revision],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
    let (revision, status, payload) = row.ok_or(StoreError::NotFound)?;
    Ok(Record {
        workspace_id: w,
        id,
        revision,
        status: match status.as_str() {
            "pending" => Status::Pending,
            "confirmed" => Status::Confirmed,
            _ => return Err(StoreError::InvalidInput),
        },
        payload: serde_json::from_str(&payload)?,
    })
}
// Derived snapshots can contain source text: epoch invalidation alone does not
// erase deleted/corrected inputs. Keep this purge in the input transaction.
fn invalidate_workspace(conn: &Connection, workspace: &str) -> Result<()> {
    conn.execute("DELETE FROM documents WHERE kind='coaching'", [])?;
    conn.execute("DELETE FROM models WHERE workspace=?1", [workspace])?;
    conn.execute(
        "DELETE FROM documents WHERE workspace=?1 AND id=?2",
        params![workspace, Store::EVOLUTION_DOCUMENT_ID.to_string()],
    )?;
    Ok(())
}
fn purge(conn: &Connection, t: &Tombstone) -> Result<()> {
    conn.execute(
        "DELETE FROM records WHERE workspace=?1 AND id=?2",
        params![t.workspace_id.to_string(), t.id.to_string()],
    )?;
    invalidate_workspace(conn, &t.workspace_id.to_string())?;
    conn.execute(
        "INSERT OR IGNORE INTO tombstones VALUES(?1,?2)",
        params![t.workspace_id.to_string(), t.id.to_string()],
    )?;
    conn.execute("UPDATE metadata SET epoch=epoch+1", [])?;
    Ok(())
}

impl Store {
    pub fn workspaces(&self) -> Result<Vec<Uuid>> {
        let mut statement = self
            .conn()?
            .prepare("SELECT id FROM workspaces ORDER BY id")?;
        let ids = statement
            .query_map([], |r| r.get::<_, String>(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        ids.into_iter()
            .map(|id| Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput))
            .collect()
    }
    pub fn list_models(&self, w: Uuid, limit: u32) -> Result<Vec<(Uuid, Value)>> {
        if limit == 0 || limit > 1000 {
            return Err(StoreError::InvalidInput);
        }
        let mut statement=self.conn()?.prepare("SELECT id,payload FROM models WHERE workspace=?1 AND epoch=(SELECT epoch FROM metadata) ORDER BY id LIMIT ?2")?;
        let rows = statement.query_map(params![w.to_string(), limit], |r| {
            Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
        })?;
        let mut output = Vec::new();
        let mut bytes = 0;
        for row in rows {
            let (id, p) = row?;
            bytes += p.len();
            if bytes > 16 * 1024 * 1024 {
                return Err(StoreError::InvalidInput);
            }
            output.push((
                Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
                serde_json::from_str(&p)?,
            ));
        }
        Ok(output)
    }
}
/// OS credential store. The account is a vault UUID, not a filesystem path.
#[cfg(target_os = "macos")]
pub mod keychain {
    use super::*;
    const SERVICE: &str = "org.cdna.vault";
    pub fn load(vault_id: Uuid) -> Result<Zeroizing<String>> {
        let bytes = Zeroizing::new(
            security_framework::passwords::generic_password(
                security_framework::passwords::PasswordOptions::new_generic_password(
                    SERVICE,
                    &vault_id.to_string(),
                ),
            )
            .map_err(|_| StoreError::Keychain)?,
        );
        let secret = std::str::from_utf8(&bytes).map_err(|_| StoreError::Keychain)?;
        if secret.len() < 32 {
            return Err(StoreError::InvalidInput);
        }
        Ok(Zeroizing::new(secret.to_owned()))
    }
    /// Caller creates a fresh vault UUID. Existing credentials are never overwritten.
    pub fn create(vault_id: Uuid) -> Result<Zeroizing<String>> {
        match security_framework::passwords::generic_password(
            security_framework::passwords::PasswordOptions::new_generic_password(
                SERVICE,
                &vault_id.to_string(),
            ),
        ) {
            Ok(_) => return Err(StoreError::InvalidInput),
            Err(error) if error.code() == -25300 => {}
            Err(_) => return Err(StoreError::Keychain),
        }
        let secret = Zeroizing::new(format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        ));
        security_framework::passwords::set_generic_password(
            SERVICE,
            &vault_id.to_string(),
            secret.as_bytes(),
        )
        .map_err(|_| StoreError::Keychain)?;
        Ok(secret)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn key(c: char) -> Zeroizing<String> {
        Zeroizing::new(c.to_string().repeat(64))
    }
    #[test]
    fn ciphertext_revision_scope_and_recovery() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("vault.db");
        let mut store = Store::create(&path, &key('a')).unwrap();
        let w = Uuid::new_v4();
        let other = Uuid::new_v4();
        let id = Uuid::new_v4();
        let req = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        store.create_workspace(other).unwrap();
        let payload = serde_json::json!({"secret":"UNIQUE_PLAINTEXT_CANARY_12345"});
        let r = store.propose(w, id, req, payload.clone()).unwrap();
        assert_eq!(r.status, Status::Pending);
        assert_eq!(
            store.propose(w, id, req, payload.clone()).unwrap().revision,
            1
        );
        assert!(matches!(
            store.propose(w, id, req, Value::Null),
            Err(StoreError::RequestConflict)
        ));
        assert!(matches!(store.get(other, id), Err(StoreError::NotFound)));
        assert!(matches!(
            store.confirm(w, id, 8, Uuid::new_v4()),
            Err(StoreError::RevisionConflict)
        ));
        assert_eq!(store.get(w, id).unwrap().revision, 1);
        store.confirm(w, id, 1, Uuid::new_v4()).unwrap();
        let epoch = store.epoch().unwrap();
        let model = Uuid::new_v4();
        store.save_model(w, model, epoch, payload.clone()).unwrap();
        let backup = dir.path().join("backup.db");
        store.backup(&backup, &key('b')).unwrap();
        store.revise(w, id, 2, Uuid::new_v4(), payload).unwrap();
        assert!(store.model(w, model).is_err());
        store.delete(w, id, 3).unwrap();
        let tombstones = store.tombstones().unwrap();
        let restored = Store::restore(
            &backup,
            &key('b'),
            dir.path().join("restored.db"),
            &key('c'),
            &tombstones,
            &[],
        )
        .unwrap();
        assert!(matches!(restored.get(w, id), Err(StoreError::NotFound)));
        assert!(restored.list_models(w, 10).unwrap().is_empty());
        assert!(Store::open(&path, &key('z')).is_err());
        for p in [&path, &backup] {
            let bytes = std::fs::read(p).unwrap();
            assert!(!bytes.starts_with(b"SQLite format"));
            assert!(
                !bytes
                    .windows(b"UNIQUE_PLAINTEXT_CANARY_12345".len())
                    .any(|x| x == b"UNIQUE_PLAINTEXT_CANARY_12345")
            );
        }
        store.lock();
        assert!(matches!(store.get(w, id), Err(StoreError::Locked)));
    }
    #[test]
    fn failed_restore_never_publishes_or_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("original");
        let store = Store::create(&original, &key('a')).unwrap();
        let backup = dir.path().join("backup");
        store.backup(&backup, &key('b')).unwrap();
        let target = dir.path().join("restored");
        assert!(Store::restore(&backup, &key('x'), &target, &key('c'), &[], &[]).is_err());
        assert!(!target.exists());
        let bad = dir.path().join("corrupt");
        std::fs::write(&bad, b"corrupt-backup").unwrap();
        assert!(Store::restore(&bad, &key('b'), &target, &key('c'), &[], &[]).is_err());
        assert!(!target.exists());
        std::fs::write(&target, b"existing-owner-file").unwrap();
        assert!(Store::restore(&backup, &key('b'), &target, &key('c'), &[], &[]).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"existing-owner-file");
        assert!(store.backup(&target, &key('b')).is_err());
        assert_eq!(std::fs::read(&target).unwrap(), b"existing-owner-file");
        assert!(Store::open(&original, &key('a')).is_ok());
    }
    #[test]
    fn missing_workspace_rolls_back() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::create(dir.path().join("v"), &key('a')).unwrap();
        let w = Uuid::new_v4();
        let id = Uuid::new_v4();
        assert!(store.propose(w, id, Uuid::new_v4(), Value::Null).is_err());
        assert_eq!(store.epoch().unwrap(), 0);
        store.create_workspace(w).unwrap();
        store.propose(w, id, Uuid::new_v4(), Value::Null).unwrap();
    }
}

/// UTC expiry is expressed as seconds since the Unix epoch.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Grant {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub scopes: Vec<String>,
    pub expires_at: i64,
    pub revoked: bool,
    pub token_hash: String,
}
pub const GRANT_SCOPES: &[&str] = &[
    "decision:read",
    "decision:propose",
    "judgment:infer",
    "learning:read",
    "policy:read",
    "profile:read",
    "question:propose",
];
fn now_unix() -> Result<i64> {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| StoreError::InvalidInput)?
        .as_secs();
    i64::try_from(seconds).map_err(|_| StoreError::InvalidInput)
}
fn valid_token_hash(hash: &str) -> bool {
    hash.len() == 64
        && hash
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
impl Store {
    pub fn create_grant(&mut self, grant: Grant) -> Result<()> {
        if grant.revoked
            || grant.expires_at <= now_unix()?
            || !valid_token_hash(&grant.token_hash)
            || grant.scopes.is_empty()
            || grant.scopes.len() > GRANT_SCOPES.len()
            || grant
                .scopes
                .iter()
                .any(|scope| !GRANT_SCOPES.contains(&scope.as_str()))
        {
            return Err(StoreError::InvalidInput);
        }
        let mut unique = std::collections::HashSet::new();
        if !grant.scopes.iter().all(|scope| unique.insert(scope)) {
            return Err(StoreError::InvalidInput);
        }
        self.conn()?.execute("INSERT INTO grants(id,workspace,scopes,expires_at,revoked,token_hash) VALUES(?1,?2,?3,?4,0,?5)",params![grant.id.to_string(),grant.workspace_id.to_string(),serde_json::to_string(&grant.scopes)?,grant.expires_at,grant.token_hash])?;
        Ok(())
    }
    pub fn authorize_grant(&self, token_hash: &str) -> Result<Grant> {
        if !valid_token_hash(token_hash) {
            return Err(StoreError::GrantDenied);
        }
        let row: Option<(String, String, String, i64, bool)> = self
            .conn()?
            .query_row(
                "SELECT id,workspace,scopes,expires_at,revoked FROM grants WHERE token_hash=?1",
                [token_hash],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?)),
            )
            .optional()?;
        let (id, w, scopes, expires_at, revoked) = row.ok_or(StoreError::GrantDenied)?;
        if revoked || expires_at <= now_unix()? {
            return Err(StoreError::GrantDenied);
        }
        let scopes: Vec<String> = serde_json::from_str(&scopes)?;
        if scopes.is_empty() || scopes.iter().any(|s| !GRANT_SCOPES.contains(&s.as_str())) {
            return Err(StoreError::GrantDenied);
        }
        Ok(Grant {
            id: Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
            workspace_id: Uuid::parse_str(&w).map_err(|_| StoreError::InvalidInput)?,
            scopes,
            expires_at,
            revoked,
            token_hash: token_hash.to_owned(),
        })
    }
    pub fn revoke_grant(&mut self, id: Uuid) -> Result<()> {
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let changed = tx.execute(
            "UPDATE grants SET revoked=1 WHERE id=?1 AND revoked=0",
            [id.to_string()],
        )?;
        if changed > 0 {
            tx.execute("DELETE FROM documents WHERE kind='coaching'", [])?;
            tx.execute("UPDATE metadata SET epoch=epoch+1", [])?;
        }
        tx.commit()?;
        Ok(())
    }
    pub fn set_workspace_name(&self, id: Uuid, name: &str) -> Result<()> {
        if name.trim().is_empty() || name.len() > 256 || name.chars().any(char::is_control) {
            return Err(StoreError::InvalidInput);
        }
        if self.conn()?.execute(
            "UPDATE workspaces SET name=?2 WHERE id=?1",
            params![id.to_string(), name],
        )? == 0
        {
            return Err(StoreError::NotFound);
        }
        Ok(())
    }
    pub fn workspace_details(&self) -> Result<Vec<(Uuid, String)>> {
        let mut stmt = self
            .conn()?
            .prepare("SELECT id,name FROM workspaces ORDER BY id LIMIT 1000")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        rows.map(|row| {
            let (id, name) = row?;
            Ok((
                Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
                name,
            ))
        })
        .collect()
    }
}
#[cfg(test)]
mod grant_tests {
    use super::*;
    #[test]
    fn grant_revocation_expiry_and_scope_are_live() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            Store::create(dir.path().join("v"), &Zeroizing::new("x".repeat(64))).unwrap();
        let w = Uuid::new_v4();
        let other = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        store.create_workspace(other).unwrap();
        let grant = Grant {
            id: Uuid::new_v4(),
            workspace_id: w,
            scopes: vec!["decision:read".into()],
            expires_at: now_unix().unwrap() + 3600,
            revoked: false,
            token_hash: "a".repeat(64),
        };
        store.create_grant(grant.clone()).unwrap();
        assert_eq!(
            store
                .authorize_grant(&grant.token_hash)
                .unwrap()
                .workspace_id,
            w
        );
        assert_ne!(
            store
                .authorize_grant(&grant.token_hash)
                .unwrap()
                .workspace_id,
            other
        );
        assert!(matches!(
            store.authorize_grant(&"b".repeat(64)),
            Err(StoreError::GrantDenied)
        ));
        let backup = dir.path().join("grants-backup");
        let recovery = Zeroizing::new("r".repeat(64));
        store.backup(&backup, &recovery).unwrap();
        let restored = Store::restore(
            &backup,
            &recovery,
            dir.path().join("grants-restored"),
            &Zeroizing::new("n".repeat(64)),
            &[],
            &[],
        )
        .unwrap();
        assert!(matches!(
            restored.authorize_grant(&grant.token_hash),
            Err(StoreError::GrantDenied)
        ));
        let epoch = store.epoch().unwrap();
        store.revoke_grant(grant.id).unwrap();
        assert_eq!(store.epoch().unwrap(), epoch + 1);
        store.revoke_grant(grant.id).unwrap();
        assert_eq!(store.epoch().unwrap(), epoch + 1);
        assert!(matches!(
            store.authorize_grant(&grant.token_hash),
            Err(StoreError::GrantDenied)
        ));
        let mut expired = grant.clone();
        expired.id = Uuid::new_v4();
        expired.token_hash = "c".repeat(64);
        expired.expires_at = now_unix().unwrap() - 1;
        assert!(store.create_grant(expired).is_err());
        let mut live = grant.clone();
        live.id = Uuid::new_v4();
        live.token_hash = "d".repeat(64);
        store.create_grant(live.clone()).unwrap();
        store
            .conn()
            .unwrap()
            .execute(
                "UPDATE grants SET expires_at=1 WHERE id=?1",
                [live.id.to_string()],
            )
            .unwrap();
        assert!(matches!(
            store.authorize_grant(&live.token_hash),
            Err(StoreError::GrantDenied)
        ));
        let mut bad = grant;
        bad.id = Uuid::new_v4();
        bad.token_hash = "e".repeat(64);
        bad.scopes = vec!["decision:confirm".into()];
        assert!(store.create_grant(bad).is_err());
        store.set_workspace_name(w, "Named workspace").unwrap();
        assert!(
            store
                .workspace_details()
                .unwrap()
                .contains(&(w, "Named workspace".into()))
        );
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    Policy,
    Source,
    Assessment,
    Consent,
    Improvement,
    Coaching,
}
impl DocumentKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Policy => "policy",
            Self::Source => "source",
            Self::Assessment => "assessment",
            Self::Consent => "consent",
            Self::Improvement => "improvement",
            Self::Coaching => "coaching",
        }
    }
    fn parse(value: &str) -> Result<Self> {
        match value {
            "policy" => Ok(Self::Policy),
            "source" => Ok(Self::Source),
            "assessment" => Ok(Self::Assessment),
            "consent" => Ok(Self::Consent),
            "improvement" => Ok(Self::Improvement),
            "coaching" => Ok(Self::Coaching),
            _ => Err(StoreError::InvalidInput),
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: DocumentKind,
    pub revision: u64,
    pub payload: Value,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DocumentTombstone {
    pub id: Uuid,
    pub workspace_id: Uuid,
    pub kind: DocumentKind,
}
impl Store {
    pub fn put_document(
        &mut self,
        kind: DocumentKind,
        w: Uuid,
        id: Uuid,
        expected_revision: u64,
        payload: Value,
    ) -> Result<Document> {
        if id == Self::EVOLUTION_DOCUMENT_ID || kind == DocumentKind::Coaching {
            return Err(StoreError::InvalidInput);
        }
        if expected_revision >= i64::MAX as u64 {
            return Err(StoreError::InvalidInput);
        }
        let serialized = readable_json(&payload)?;
        if serialized.len() > 1024 * 1024 {
            return Err(StoreError::InvalidInput);
        }
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let args = params![w.to_string(), kind.as_str(), id.to_string()];
        let tombstoned:bool=tx.query_row("SELECT EXISTS(SELECT 1 FROM document_tombstones WHERE workspace=?1 AND kind=?2 AND id=?3)",args,|r|r.get(0))?;
        if tombstoned {
            return Err(StoreError::NotFound);
        }
        let current: Option<u64> = tx
            .query_row(
                "SELECT revision FROM documents WHERE workspace=?1 AND kind=?2 AND id=?3",
                args,
                |r| r.get(0),
            )
            .optional()?;
        if current.unwrap_or(0) != expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        let revision = expected_revision + 1;
        tx.execute("INSERT INTO documents VALUES(?1,?2,?3,?4,?5) ON CONFLICT(workspace,kind,id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",params![w.to_string(),kind.as_str(),id.to_string(),revision,serialized])?;
        invalidate_workspace(&tx, &w.to_string())?;
        tx.execute("UPDATE metadata SET epoch=epoch+1", [])?;
        tx.commit()?;
        Ok(Document {
            id,
            workspace_id: w,
            kind,
            revision,
            payload,
        })
    }
    pub fn get_document(&self, kind: DocumentKind, w: Uuid, id: Uuid) -> Result<Document> {
        let row: Option<(u64, String)> = self
            .conn()?
            .query_row(
                "SELECT revision,payload FROM documents WHERE workspace=?1 AND kind=?2 AND id=?3",
                params![w.to_string(), kind.as_str(), id.to_string()],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let (revision, payload) = row.ok_or(StoreError::NotFound)?;
        Ok(Document {
            id,
            workspace_id: w,
            kind,
            revision,
            payload: serde_json::from_str(&payload)?,
        })
    }
    pub fn list_documents(
        &self,
        kind: DocumentKind,
        w: Uuid,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Document>> {
        if limit == 0 || limit > 1000 || offset > 1_000_000 {
            return Err(StoreError::InvalidInput);
        }
        let mut statement=self.conn()?.prepare("SELECT id,revision,payload FROM documents WHERE workspace=?1 AND kind=?2 ORDER BY id LIMIT ?3 OFFSET ?4")?;
        let rows =
            statement.query_map(params![w.to_string(), kind.as_str(), limit, offset], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, u64>(1)?,
                    r.get::<_, String>(2)?,
                ))
            })?;
        let mut bytes = 0;
        let mut result = Vec::new();
        for row in rows {
            let (id, revision, payload) = row?;
            bytes += payload.len();
            if bytes > 16 * 1024 * 1024 {
                return Err(StoreError::InvalidInput);
            }
            result.push(Document {
                id: Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
                workspace_id: w,
                kind,
                revision,
                payload: serde_json::from_str(&payload)?,
            });
        }
        Ok(result)
    }
    pub fn delete_document(
        &mut self,
        kind: DocumentKind,
        w: Uuid,
        id: Uuid,
        expected_revision: u64,
    ) -> Result<()> {
        if id == Self::EVOLUTION_DOCUMENT_ID || kind == DocumentKind::Coaching {
            return Err(StoreError::InvalidInput);
        }
        if kind == DocumentKind::Source {
            return self
                .delete_source_cascade(w, id, expected_revision)
                .map(|_| ());
        }
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: Option<u64> = tx
            .query_row(
                "SELECT revision FROM documents WHERE workspace=?1 AND kind=?2 AND id=?3",
                params![w.to_string(), kind.as_str(), id.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        let current = current.ok_or(StoreError::NotFound)?;
        if current != expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        tx.execute(
            "DELETE FROM documents WHERE workspace=?1 AND kind=?2 AND id=?3",
            params![w.to_string(), kind.as_str(), id.to_string()],
        )?;
        tx.execute(
            "INSERT INTO document_tombstones VALUES(?1,?2,?3)",
            params![w.to_string(), kind.as_str(), id.to_string()],
        )?;
        invalidate_workspace(&tx, &w.to_string())?;
        tx.execute("UPDATE metadata SET epoch=epoch+1", [])?;
        tx.commit()?;
        Ok(())
    }
    pub fn document_tombstones(&self) -> Result<Vec<DocumentTombstone>> {
        let mut stmt = self
            .conn()?
            .prepare("SELECT workspace,kind,id FROM document_tombstones")?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
            ))
        })?;
        rows.map(|row| {
            let (w, kind, id) = row?;
            Ok(DocumentTombstone {
                workspace_id: Uuid::parse_str(&w).map_err(|_| StoreError::InvalidInput)?,
                kind: DocumentKind::parse(&kind)?,
                id: Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
            })
        })
        .collect()
    }
}
#[cfg(test)]
mod document_tests {
    use super::*;
    #[test]
    fn documents_are_revisioned_bounded_scoped_and_restore_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let key = Zeroizing::new("a".repeat(64));
        let mut store = Store::create(dir.path().join("v"), &key).unwrap();
        let w = Uuid::new_v4();
        let other = Uuid::new_v4();
        let id = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        store.create_workspace(other).unwrap();
        store
            .put_document(DocumentKind::Policy, w, id, 0, Value::Null)
            .unwrap();
        assert!(matches!(
            store.get_document(DocumentKind::Source, w, id),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            store.get_document(DocumentKind::Policy, other, id),
            Err(StoreError::NotFound)
        ));
        assert!(matches!(
            store.put_document(DocumentKind::Policy, w, id, 0, Value::Null),
            Err(StoreError::RevisionConflict)
        ));
        assert!(
            store
                .list_documents(DocumentKind::Policy, w, 1001, 0)
                .is_err()
        );
        assert!(
            store
                .put_document(
                    DocumentKind::Source,
                    w,
                    id,
                    0,
                    Value::String("x".repeat(1024 * 1024))
                )
                .is_err()
        );
        store
            .put_document(
                DocumentKind::Policy,
                w,
                id,
                1,
                serde_json::json!({"name":"updated"}),
            )
            .unwrap();
        store
            .put_document(DocumentKind::Source, w, id, 0, Value::Null)
            .unwrap();
        let backup = dir.path().join("backup");
        store.backup(&backup, &key).unwrap();
        assert!(matches!(
            store.delete_document(DocumentKind::Policy, w, id, 1),
            Err(StoreError::RevisionConflict)
        ));
        store
            .delete_document(DocumentKind::Policy, w, id, 2)
            .unwrap();
        assert!(store.get_document(DocumentKind::Source, w, id).is_ok());
        let restored = Store::restore(
            backup,
            &key,
            dir.path().join("restored"),
            &key,
            &[],
            &store.document_tombstones().unwrap(),
        )
        .unwrap();
        assert!(matches!(
            restored.get_document(DocumentKind::Policy, w, id),
            Err(StoreError::NotFound)
        ));
        assert!(restored.get_document(DocumentKind::Source, w, id).is_ok());
        assert!(matches!(
            store.put_document(DocumentKind::Policy, w, id, 0, Value::Null),
            Err(StoreError::NotFound)
        ));
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchInsertResult {
    pub inserted: usize,
    pub skipped: usize,
}
impl Store {
    /// Atomic create-only import. Existing owner edits and deletion tombstones win.
    pub fn insert_documents_batch(
        &mut self,
        kind: DocumentKind,
        workspace: Uuid,
        documents: Vec<(Uuid, Value)>,
    ) -> Result<BatchInsertResult> {
        if kind == DocumentKind::Coaching || documents.len() > 1000
            || documents
                .iter()
                .any(|(id, _)| *id == Self::EVOLUTION_DOCUMENT_ID)
        {
            return Err(StoreError::InvalidInput);
        }
        let mut total = 0usize;
        let prepared = documents
            .into_iter()
            .map(|(id, payload)| {
                let payload = readable_json(&payload)?;
                total = total
                    .checked_add(payload.len())
                    .ok_or(StoreError::InvalidInput)?;
                if payload.len() > 1024 * 1024 || total > 16 * 1024 * 1024 {
                    return Err(StoreError::InvalidInput);
                }
                Ok((id, payload))
            })
            .collect::<Result<Vec<_>>>()?;
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let ws = workspace.to_string();
        let exists: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM workspaces WHERE id=?1)",
            [&ws],
            |r| r.get(0),
        )?;
        if !exists {
            return Err(StoreError::NotFound);
        }
        let mut result = BatchInsertResult {
            inserted: 0,
            skipped: 0,
        };
        {
            let mut insert=tx.prepare("INSERT INTO documents(workspace,kind,id,revision,payload) SELECT ?1,?2,?3,1,?4 WHERE NOT EXISTS(SELECT 1 FROM document_tombstones WHERE workspace=?1 AND kind=?2 AND id=?3) ON CONFLICT(workspace,kind,id) DO NOTHING")?;
            for (id, payload) in prepared {
                if insert.execute(params![ws, kind.as_str(), id.to_string(), payload])? == 1 {
                    result.inserted += 1;
                } else {
                    result.skipped += 1;
                }
            }
        }
        if result.inserted > 0 {
            invalidate_workspace(&tx, &ws)?;
            tx.execute("UPDATE metadata SET epoch=epoch+1", [])?;
        }
        tx.commit()?;
        Ok(result)
    }
}
#[cfg(test)]
mod batch_tests {
    use super::*;
    #[test]
    fn import_batch_is_atomic_preserves_owner_edits_and_deletions() {
        let dir = tempfile::tempdir().unwrap();
        let mut store =
            Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
        let w = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        let id = Uuid::new_v4();
        let second = Uuid::new_v4();
        let epoch = store.epoch().unwrap();
        assert_eq!(
            store
                .insert_documents_batch(
                    DocumentKind::Source,
                    w,
                    vec![
                        (id, Value::String("original".into())),
                        (second, Value::Null)
                    ]
                )
                .unwrap(),
            BatchInsertResult {
                inserted: 2,
                skipped: 0
            }
        );
        assert_eq!(store.epoch().unwrap(), epoch + 1);
        store
            .put_document(
                DocumentKind::Source,
                w,
                id,
                1,
                Value::String("owner corrected".into()),
            )
            .unwrap();
        store
            .delete_document(DocumentKind::Source, w, second, 1)
            .unwrap();
        let epoch = store.epoch().unwrap();
        assert_eq!(
            store
                .insert_documents_batch(
                    DocumentKind::Source,
                    w,
                    vec![(id, Value::Null), (second, Value::Null)]
                )
                .unwrap(),
            BatchInsertResult {
                inserted: 0,
                skipped: 2
            }
        );
        assert_eq!(store.epoch().unwrap(), epoch);
        assert_eq!(
            store
                .get_document(DocumentKind::Source, w, id)
                .unwrap()
                .payload,
            Value::String("owner corrected".into())
        );
        assert!(store.get_document(DocumentKind::Source, w, second).is_err());
        let fresh = Uuid::new_v4();
        assert!(
            store
                .insert_documents_batch(
                    DocumentKind::Source,
                    w,
                    vec![
                        (fresh, Value::Null),
                        (Uuid::new_v4(), Value::String("x".repeat(1024 * 1024)))
                    ]
                )
                .is_err()
        );
        assert!(store.get_document(DocumentKind::Source, w, fresh).is_err());
        assert_eq!(store.epoch().unwrap(), epoch);
        assert!(
            store
                .insert_documents_batch(DocumentKind::Source, Uuid::new_v4(), vec![])
                .is_err()
        );
        // A database failure after the first row must roll back that row and epoch.
        store.conn().unwrap().execute_batch("CREATE TRIGGER reject_import BEFORE INSERT ON documents WHEN NEW.payload='\"fail\"' BEGIN SELECT RAISE(ABORT,'injected'); END;").unwrap();
        assert!(
            store
                .insert_documents_batch(
                    DocumentKind::Source,
                    w,
                    vec![
                        (fresh, Value::Null),
                        (Uuid::new_v4(), Value::String("fail".into()))
                    ]
                )
                .is_err()
        );
        assert!(store.get_document(DocumentKind::Source, w, fresh).is_err());
        assert_eq!(store.epoch().unwrap(), epoch);
    }
}

impl Store {
    /// Reserved derived-state identity; never writable through generic document APIs.
    pub const EVOLUTION_DOCUMENT_ID: Uuid = Uuid::from_u128(0xcdfa81ae_7c42_481b_a8bd_1f3387259351);
    pub fn put_evolution_document(
        &mut self,
        workspace: Uuid,
        id: Uuid,
        expected_revision: u64,
        expected_epoch: u64,
        payload: Value,
    ) -> Result<Document> {
        if id != Self::EVOLUTION_DOCUMENT_ID || expected_revision >= i64::MAX as u64 {
            return Err(StoreError::InvalidInput);
        }
        let serialized = readable_json(&payload)?;
        if serialized.len() > 1024 * 1024 {
            return Err(StoreError::InvalidInput);
        }
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let epoch: u64 = tx.query_row("SELECT epoch FROM metadata", [], |r| r.get(0))?;
        if epoch != expected_epoch {
            return Err(StoreError::RevisionConflict);
        }
        let current:Option<u64>=tx.query_row("SELECT revision FROM documents WHERE workspace=?1 AND kind='improvement' AND id=?2",params![workspace.to_string(),id.to_string()],|r|r.get(0)).optional()?;
        if current.unwrap_or(0) != expected_revision {
            return Err(StoreError::RevisionConflict);
        }
        let revision = expected_revision + 1;
        tx.execute("INSERT INTO documents(workspace,kind,id,revision,payload) VALUES(?1,'improvement',?2,?3,?4) ON CONFLICT(workspace,kind,id) DO UPDATE SET revision=excluded.revision,payload=excluded.payload",params![workspace.to_string(),id.to_string(),revision,serialized])?;
        tx.commit()?;
        Ok(Document {
            id,
            workspace_id: workspace,
            kind: DocumentKind::Improvement,
            revision,
            payload,
        })
    }
}
#[cfg(test)]
mod evolution_tests {
    use super::*;
    #[test]
    fn evolution_checks_epoch_revision_and_cannot_bypass_or_restore() {
        let dir = tempfile::tempdir().unwrap();
        let key = Zeroizing::new("a".repeat(64));
        let mut store = Store::create(dir.path().join("vault"), &key).unwrap();
        let w = Uuid::new_v4();
        store.create_workspace(w).unwrap();
        let epoch = store.epoch().unwrap();
        let id = Store::EVOLUTION_DOCUMENT_ID;
        let model = Uuid::new_v4();
        store.save_model(w, model, epoch, Value::Null).unwrap();
        let doc = store
            .put_evolution_document(w, id, 0, epoch, serde_json::json!({"epoch":epoch}))
            .unwrap();
        assert_eq!(doc.revision, 1);
        assert_eq!(store.epoch().unwrap(), epoch);
        assert!(store.model(w, model).is_ok());
        assert!(matches!(
            store.put_evolution_document(w, id, 0, epoch, Value::Null),
            Err(StoreError::RevisionConflict)
        ));
        assert!(matches!(
            store.put_evolution_document(w, id, 1, epoch + 1, Value::Null),
            Err(StoreError::RevisionConflict)
        ));
        assert_eq!(
            store
                .get_document(DocumentKind::Improvement, w, id)
                .unwrap()
                .revision,
            1
        );
        for kind in [
            DocumentKind::Improvement,
            DocumentKind::Policy,
            DocumentKind::Source,
            DocumentKind::Assessment,
            DocumentKind::Consent,
        ] {
            assert!(matches!(
                store.put_document(kind, w, id, 1, Value::Null),
                Err(StoreError::InvalidInput)
            ));
            assert!(matches!(
                store.delete_document(kind, w, id, 1),
                Err(StoreError::InvalidInput)
            ));
            assert!(matches!(
                store.insert_documents_batch(kind, w, vec![(id, Value::Null)]),
                Err(StoreError::InvalidInput)
            ));
        }
        assert!(matches!(
            store.put_evolution_document(w, Uuid::new_v4(), 0, epoch, Value::Null),
            Err(StoreError::InvalidInput)
        ));
        let backup = dir.path().join("backup");
        store.backup(&backup, &key).unwrap();
        let restored =
            Store::restore(&backup, &key, dir.path().join("restored"), &key, &[], &[]).unwrap();
        assert!(matches!(
            restored.get_document(DocumentKind::Improvement, w, id),
            Err(StoreError::NotFound)
        ));
        store
            .put_document(DocumentKind::Policy, w, Uuid::new_v4(), 0, Value::Null)
            .unwrap();
        assert!(matches!(
            store.put_evolution_document(w, id, 1, epoch, Value::Null),
            Err(StoreError::RevisionConflict)
        ));
    }
}
