//! Epoch-bound proposal drafts. Writes do not change learning inputs or models.
use super::*;
impl Store {
    pub(super) fn initialize_coaching(&self) -> Result<()> {
        self.conn()?.execute_batch("CREATE TABLE IF NOT EXISTS coaching_answers(workspace TEXT NOT NULL,request TEXT NOT NULL,batch TEXT NOT NULL,question TEXT NOT NULL,record_id TEXT NOT NULL,PRIMARY KEY(workspace,request),FOREIGN KEY(workspace,record_id) REFERENCES records(workspace,id) ON DELETE CASCADE);")?;
        Ok(())
    }
    pub fn coaching_answer(
        &self,
        workspace: Uuid,
        batch: Uuid,
        question: Uuid,
        request: Uuid,
    ) -> Result<Option<Record>> {
        let prior:Option<(String,String,String)>=self.conn()?.query_row("SELECT batch,question,record_id FROM coaching_answers WHERE workspace=?1 AND request=?2",params![workspace.to_string(),request.to_string()],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        match prior {
            None => Ok(None),
            Some((b, q, id)) => {
                if b != batch.to_string() || q != question.to_string() {
                    return Err(StoreError::RequestConflict);
                }
                Ok(Some(self.get(
                    workspace,
                    Uuid::parse_str(&id).map_err(|_| StoreError::InvalidInput)?,
                )?))
            }
        }
    }
    pub fn propose_coaching_answer(
        &mut self,
        workspace: Uuid,
        batch: Uuid,
        question: Uuid,
        request: Uuid,
        payload: Value,
    ) -> Result<Record> {
        self.write(
            workspace,
            request,
            0,
            request,
            Some(payload),
            false,
            Some((batch, question)),
        )
    }

    pub fn save_coaching_batch(
        &mut self,
        workspace: Uuid,
        request: Uuid,
        epoch: u64,
        payload: Value,
    ) -> Result<Document> {
        let serialized = readable_json(&payload)?;
        if serialized.len() > 256 * 1024 || payload["epoch"].as_u64() != Some(epoch) {
            return Err(StoreError::InvalidInput);
        }
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: u64 = tx.query_row("SELECT epoch FROM metadata", [], |r| r.get(0))?;
        if current != epoch {
            return Err(StoreError::RevisionConflict);
        }
        let prior: Option<String> = tx
            .query_row(
                "SELECT payload FROM documents WHERE workspace=?1 AND kind='coaching' AND id=?2",
                params![workspace.to_string(), request.to_string()],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(prior) = prior {
            let prior: Value = serde_json::from_str(&prior)?;
            if prior["request"] != payload["request"] {
                return Err(StoreError::RequestConflict);
            }
        } else {
            let count: u32 = tx.query_row(
                "SELECT count(*) FROM documents WHERE workspace=?1 AND kind='coaching'",
                [workspace.to_string()],
                |r| r.get(0),
            )?;
            if count >= 32 {
                return Err(StoreError::InvalidInput);
            }
            tx.execute(
                "INSERT INTO documents VALUES(?1,'coaching',?2,1,?3)",
                params![workspace.to_string(), request.to_string(), serialized],
            )?;
        }
        tx.commit()?;
        self.get_document(DocumentKind::Coaching, workspace, request)
    }
    pub fn adopt_coaching(&mut self, workspace: Uuid, id: Uuid, epoch: u64) -> Result<Document> {
        let tx = self
            .conn_mut()?
            .transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let current: u64 = tx.query_row("SELECT epoch FROM metadata", [], |r| r.get(0))?;
        if current != epoch {
            return Err(StoreError::RevisionConflict);
        }
        let text: String = tx
            .query_row(
                "SELECT payload FROM documents WHERE workspace=?1 AND kind='coaching' AND id=?2",
                params![workspace.to_string(), id.to_string()],
                |r| r.get(0),
            )
            .optional()?
            .ok_or(StoreError::NotFound)?;
        let mut payload: Value = serde_json::from_str(&text)?;
        if payload["epoch"].as_u64() != Some(epoch) {
            return Err(StoreError::RevisionConflict);
        }
        payload["adopted"] = Value::Bool(true);
        tx.execute(
            "UPDATE documents SET payload=?3 WHERE workspace=?1 AND kind='coaching' AND id=?2",
            params![
                workspace.to_string(),
                id.to_string(),
                readable_json(&payload)?
            ],
        )?;
        tx.commit()?;
        self.get_document(DocumentKind::Coaching, workspace, id)
    }
}
