//! Bounded current-revision snapshots; unrelated pending records never block learning.
use crate::TrainingTask;
use anyhow::{Result, ensure};
use cdna_domain::{Domain, ObservationProposal, ObservationResponse, PairwiseOutcome, SourceKind};
use cdna_store::Store;
use serde_json::json;
use std::{path::Path, time::{Duration, Instant}};
use uuid::Uuid;

const MAX_ELIGIBLE: usize = 2000;
const MAX_SNAPSHOT_BYTES: usize = 8 * 1024 * 1024;
const MAX_SCANNED: usize = 100_000;

pub fn prepare(store: &Store, learner: &Path, workspace: Uuid) -> Result<TrainingTask> {
    prepare_for_domain(store, learner, workspace, None)
}

pub fn prepare_for_domain(store: &Store, learner: &Path, workspace: Uuid, domain: Option<Domain>) -> Result<TrainingTask> {
    ensure!(store.workspaces()?.contains(&workspace), "SCOPE_DENIED");
    let epoch = store.epoch()?;
    let started = Instant::now();
    let mut after = None;
    let mut scanned = 0;
    let mut bytes = 256;
    let mut input = Vec::new();
    let mut lineage = Vec::new();
    loop {
        ensure!(store.epoch()? == epoch, "REVISION_CONFLICT: training inputs changed");
        ensure!(started.elapsed() < Duration::from_secs(10), "CANCELLED: snapshot deadline");
        let page = store.confirmed_page(workspace, after, 128)?;
        if page.is_empty() { break; }
        for record in page {
            after = Some(record.id);
            scanned += 1;
            ensure!(scanned <= MAX_SCANNED, "INPUT_INVALID: confirmed snapshot scan limit reached");
            let proposal: ObservationProposal = serde_json::from_value(record.payload)?;
            proposal.validate()?;
            ensure!(proposal.workspace_id == workspace, "SCOPE_DENIED");
            if matches!(proposal.source.source_kind, SourceKind::AiGenerated) || proposal.model_exposure { continue; }
            if domain.as_ref().is_some_and(|selected| selected != &proposal.domain) { continue; }
            let mut row = json!({"family_id":proposal.family_id,"timestamp":proposal.source.observed_at,"domain":proposal.domain,"context":proposal.context,"candidates":proposal.candidates,"verification_state":"confirmed","model_exposure":false,"case_kind":proposal.case_kind});
            match proposal.response {
                ObservationResponse::ChooseOne { candidate_id } => {
                    row["chosen_ids"] = json!([candidate_id]); row["answer_kind"] = json!("choose_one");
                },
                ObservationResponse::ChooseSet { candidate_ids } => {
                    row["chosen_ids"] = json!(candidate_ids); row["answer_kind"] = json!("choose_set");
                },
                ObservationResponse::Pairwise { left_id, right_id, outcome } => {
                    row["chosen_ids"] = match outcome { PairwiseOutcome::Left => json!([left_id]), PairwiseOutcome::Right => json!([right_id]), _ => json!([]) };
                    row["answer_kind"] = json!("pairwise"); row["left_id"] = json!(left_id); row["right_id"] = json!(right_id); row["preference"] = json!(outcome);
                },
                ObservationResponse::Acceptability { candidate_id, outcome } => {
                    row["chosen_ids"] = json!([candidate_id]); row["answer_kind"] = json!("acceptability"); row["acceptability_outcome"] = json!(outcome); row["conditions"] = json!(proposal.reversal_conditions);
                },
                _ => continue,
            }
            // Preserve every eligible row or fail explicitly. Never truncate a
            // dataset silently, which would change time/family split semantics.
            ensure!(input.len() < MAX_ELIGIBLE, "INPUT_INVALID: eligible training snapshot exceeds 2000 records");
            bytes += serde_json::to_vec(&row)?.len() + 1;
            ensure!(bytes <= MAX_SNAPSHOT_BYTES, "INPUT_INVALID: eligible training snapshot exceeds 8 MiB");
            input.push(row);
            lineage.push(json!({"id":record.id,"revision":record.revision}));
        }
    }
    ensure!(store.epoch()? == epoch, "REVISION_CONFLICT: training inputs changed");
    ensure!(!input.is_empty(), "MODEL_NOT_READY: answer and confirm a question first");
    Ok(TrainingTask { workspace, epoch, learner:learner.to_path_buf(), input:json!({"operation":"train","feature_version":"1.0","domain":domain,"records":input}), lineage })
}

#[cfg(test)]
mod tests {
    use super::*;
    use zeroize::Zeroizing;

    fn proposal(workspace: Uuid) -> serde_json::Value {
        json!({"schema_version":"1.0","request_id":Uuid::new_v4(),"workspace_id":workspace,"family_id":Uuid::new_v4(),"case_kind":"hypothetical","domain":"product_delivery","context":{"as_of":"2026-09-01T00:00:00Z","summary":"Synthetic snapshot test","facts":[],"unknown_fields":[]},"candidates":[{"id":"a","text":"A","attributes":{}},{"id":"b","text":"B","attributes":{}}],"response":{"response_type":"choose_one","candidate_id":"a"},"source":{"artifact_id":Uuid::new_v4(),"source_kind":"human_app","occurred_at":null,"observed_at":"2026-09-01T01:00:00Z"},"model_exposure":false,"rationale_explicit":null,"reversal_conditions":[]})
    }
    #[test]
    fn large_pending_corpus_does_not_block_eligible_snapshot() {
        let dir=tempfile::tempdir().unwrap();
        let mut store=Store::create(dir.path().join("vault.db"),&Zeroizing::new("s".repeat(64))).unwrap();
        let workspace=Uuid::new_v4();store.create_workspace(workspace).unwrap();
        for _ in 0..1001 {
            let id=Uuid::new_v4();store.propose(workspace,id,Uuid::new_v4(),proposal(workspace)).unwrap();
        }
        let chosen=Uuid::new_v4();
        store.propose(workspace,chosen,Uuid::new_v4(),proposal(workspace)).unwrap();
        store.confirm(workspace,chosen,1,Uuid::new_v4()).unwrap();
        let mut exposed=proposal(workspace);exposed["model_exposure"]=json!(true);
        let exposed_id=Uuid::new_v4();
        store.propose(workspace,exposed_id,Uuid::new_v4(),exposed).unwrap();
        store.confirm(workspace,exposed_id,1,Uuid::new_v4()).unwrap();
        let snapshot=prepare(&store,Path::new("learner"),workspace).unwrap();
        assert_eq!(snapshot.input["records"].as_array().unwrap().len(),1);
        assert_eq!(snapshot.lineage[0]["id"],chosen.to_string());
        assert_eq!(snapshot.lineage[0]["revision"],2);
        let mut correction=proposal(workspace);correction["response"]["candidate_id"]=json!("b");
        store.revise(workspace,chosen,2,Uuid::new_v4(),correction).unwrap();
        assert!(prepare(&store,Path::new("learner"),workspace).is_err());
        store.confirm(workspace,chosen,3,Uuid::new_v4()).unwrap();
        let updated=prepare(&store,Path::new("learner"),workspace).unwrap();
        assert_eq!(updated.input["records"][0]["chosen_ids"],json!(["b"]));
        assert_eq!(updated.lineage[0]["revision"],4);
        assert!(prepare(&store,Path::new("learner"),Uuid::new_v4()).is_err());
    }
    #[test]
    fn pages_only_include_current_confirmed_revisions_and_workspace() {
        let dir=tempfile::tempdir().unwrap();
        let mut store=Store::create(dir.path().join("vault.db"),&Zeroizing::new("s".repeat(64))).unwrap();
        let workspace=Uuid::new_v4();let foreign=Uuid::new_v4();
        store.create_workspace(workspace).unwrap();store.create_workspace(foreign).unwrap();
        for (w,id) in [(workspace,Uuid::from_u128(1)),(foreign,Uuid::from_u128(2)),(workspace,Uuid::from_u128(3))] {
            store.propose(w,id,Uuid::new_v4(),proposal(w)).unwrap();
            store.confirm(w,id,1,Uuid::new_v4()).unwrap();
        }
        let first=store.confirmed_page(workspace,None,1).unwrap();
        assert_eq!(first[0].id,Uuid::from_u128(1));
        let second=store.confirmed_page(workspace,Some(first[0].id),1).unwrap();
        assert_eq!(second[0].id,Uuid::from_u128(3));
        assert!(store.confirmed_page(workspace,Some(second[0].id),1).unwrap().is_empty());
        assert!(store.confirmed_page(workspace,None,257).is_err());
    }
}
