//! Explicit trusted-host CLI adapter; never dispatch through MCP or Engine::execute.
use crate::evolution_bridge::EvolutionBridge;
use anyhow::{Context, Result, ensure};
use cdna_evolution::{Candidate, Dataset, EvaluationReceipt, HumanApproval, Signed};
use cdna_store::Store;
use ed25519_dalek::VerifyingKey;
use serde::Deserialize;
use serde_json::{Value, json};
use uuid::Uuid;

pub const MAX_COMMAND_BYTES: usize = 1024 * 1024;

#[derive(Deserialize)]
#[serde(tag = "operation", rename_all = "snake_case", deny_unknown_fields)]
enum Command {
    PrepareCandidate {
        workspace_id: Uuid,
        model_id: Uuid,
        dataset: Dataset,
        inference: cdna_evolution::InferenceSupport,
    },
    Status {
        workspace_id: Uuid,
    },
    Register {
        workspace_id: Uuid,
        expected_revision: u64,
        candidate: Candidate,
        artifact: Vec<u8>,
    },
    Evaluate {
        workspace_id: Uuid,
        expected_revision: u64,
        receipt: Signed<EvaluationReceipt>,
        dataset: Dataset,
    },
    Approve {
        workspace_id: Uuid,
        expected_revision: u64,
        approval: Signed<HumanApproval>,
    },
    Activate {
        workspace_id: Uuid,
        expected_revision: u64,
        candidate_id: Uuid,
    },
    Rollback {
        workspace_id: Uuid,
        expected_revision: u64,
        candidate_id: Uuid,
    },
    Invalidate {
        workspace_id: Uuid,
        expected_revision: u64,
    },
}

/// Keys must be pinned by host configuration, not obtained from command JSON.
/// The host must bound raw stdin before decoding Value. The vault epoch is this
/// adapter's conservative input revision: any vault input change stales evidence.
pub fn run(
    store: &mut Store,
    command: Value,
    evaluator_public_key: [u8; 32],
    human_public_key: [u8; 32],
) -> Result<Value> {
    ensure!(
        serde_json::to_vec(&command)?.len() <= MAX_COMMAND_BYTES,
        "EVOLUTION_INVALID: command too large"
    );
    let command: Command =
        serde_json::from_value(command).context("EVOLUTION_INVALID: malformed command")?;
    let bridge = EvolutionBridge::new(
        VerifyingKey::from_bytes(&evaluator_public_key)
            .context("EVOLUTION_UNTRUSTED: evaluator key")?,
        VerifyingKey::from_bytes(&human_public_key).context("EVOLUTION_UNTRUSTED: human key")?,
    )?;
    let input_revision = store.epoch()?;
    let (workspace, snapshot, candidate_id) = match command {
        Command::PrepareCandidate {workspace_id,model_id,dataset,inference} => {
            // Conversion is unsigned and read-only. It cannot activate anything.
            let artifact=store.model(workspace_id,model_id)?;
            ensure!(artifact["state"]=="provisional", "EVOLUTION_INVALID: expected provisional learner artifact");
            let model=&artifact["model"];
            if let Some(domain)=model["training_domain"].as_str() {
                ensure!(inference.domains.len()==1 && inference.domains[0]==domain,
                    "EVOLUTION_INVALID: conversion must preserve trained domain scope");
            } else {
                ensure!(model["training_domain"].is_null(),"EVOLUTION_INVALID: malformed trained domain scope");
            }
            ensure!(model["calibration"]["method"]=="sigmoid_pairwise"
                && model["calibration"]["slope"].as_f64()==Some(inference.slope)
                && model["abstention_margin"].as_f64()==Some(inference.min_margin)
                && model["calibration_scope"]==serde_json::to_value(&inference.domains)?,
                "EVOLUTION_INVALID: conversion must preserve learner calibration");
            let masks:Vec<u16>=serde_json::from_value(model["supported_candidate_missing_masks"].clone())?;
            ensure!(inference.missing_patterns.iter().all(|mask|masks.contains(&(mask & 63))),
                "EVOLUTION_INVALID: conversion expands learner missing support");
            ensure!(dataset.workspace_id==workspace_id && dataset.input_revision==input_revision && dataset.deletion_epoch==input_revision,
                "EVOLUTION_INVALID: stale or foreign dataset");
            dataset.validate().map_err(anyhow::Error::msg)?;
            let config=cdna_evolution::ModelConfig {
                feature_version:model["feature_version"].as_str().context("EVOLUTION_INVALID: missing feature version")?.into(),
                weights:serde_json::from_value(model["weights"].clone())?,inference:Some(inference),
            };
            let bytes=cdna_evolution::signing_bytes(&config).map_err(anyhow::Error::msg)?;
            let candidate=Candidate {id:Uuid::new_v4(),workspace_id,input_revision,deletion_epoch:input_revision,
                artifact_hash:cdna_evolution::hash(&bytes),training_dataset_id:dataset.id,
                training_dataset_hash:cdna_evolution::content_hash(&dataset).map_err(anyhow::Error::msg)?,config};
            // Reuse the canonical configuration and artifact validation, without writing state.
            let mut validation=cdna_evolution::Evolution::new(workspace_id,input_revision,input_revision);
            validation.register(candidate.clone(),&bytes).map_err(anyhow::Error::msg)?;
            ensure!(store.epoch()?==input_revision,"EVOLUTION_INVALID: input changed during conversion");
            return Ok(json!({"candidate":candidate,"artifact":bytes,"state":"unsigned_candidate","promotion_eligible":false,"required":["register","independent_signed_evaluation","explicit_signed_human_approval","activate"]}));
        },
        Command::Status { workspace_id: w } => (w, bridge.load(store, w, input_revision)?, None),
        Command::Register {
            workspace_id: w,
            expected_revision: r,
            candidate,
            artifact,
        } => {
            let id = candidate.id;
            (
                w,
                bridge.register(store, w, input_revision, r, candidate, &artifact)?,
                Some(id),
            )
        }
        Command::Evaluate {
            workspace_id: w,
            expected_revision: r,
            receipt,
            dataset,
        } => {
            let id = receipt.payload.candidate_id;
            (
                w,
                bridge.evaluate(store, w, input_revision, r, receipt, dataset)?,
                Some(id),
            )
        }
        Command::Approve {
            workspace_id: w,
            expected_revision: r,
            approval,
        } => {
            let id = approval.payload.candidate_id;
            (
                w,
                bridge.approve(store, w, input_revision, r, approval)?,
                Some(id),
            )
        }
        Command::Activate {
            workspace_id: w,
            expected_revision: r,
            candidate_id: id,
        } => (
            w,
            bridge.activate(store, w, input_revision, r, id)?,
            Some(id),
        ),
        Command::Rollback {
            workspace_id: w,
            expected_revision: r,
            candidate_id: id,
        } => (
            w,
            bridge.rollback(store, w, input_revision, r, id)?,
            Some(id),
        ),
        Command::Invalidate {
            workspace_id: w,
            expected_revision: r,
        } => (w, bridge.invalidate(store, w, input_revision, r)?, None),
    };
    // No answers, case rows, signatures, domains, weights or trust material.
    Ok(json!({
        "workspace_id": workspace,
        "revision": snapshot.revision,
        "input_revision": snapshot.input_revision,
        "deletion_epoch": snapshot.deletion_epoch,
        "active_candidate_id": snapshot.active_candidate().map(|c| c.id),
        "candidate_id": candidate_id,
        "candidate_state": candidate_id.and_then(|id| snapshot.state(id)),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdna_evolution::{ModelConfig, hash, signing_bytes};
    use cdna_store::DocumentKind;
    use ed25519_dalek::{Signer, SigningKey};
    use zeroize::Zeroizing;

    fn keys() -> ([u8; 32], [u8; 32]) {
        (
            SigningKey::from_bytes(&[51; 32]).verifying_key().to_bytes(),
            SigningKey::from_bytes(&[52; 32]).verifying_key().to_bytes(),
        )
    }
    fn invoke(store: &mut Store, command: Value) -> Result<Value> {
        let (e, h) = keys();
        run(store, command, e, h)
    }
    fn fixture() -> (tempfile::TempDir, Store, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::create(
            dir.path().join("vault.db"),
            &Zeroizing::new("synthetic-test-vault-key-".repeat(4)),
        )
        .unwrap();
        let workspace = Uuid::new_v4();
        store.create_workspace(workspace).unwrap();
        (dir, store, workspace)
    }
    fn registration(w: Uuid, epoch: u64) -> Value {
        let config = ModelConfig {
                inference: None,
            feature_version: "1.0".into(),
            weights: vec![0.; 17],
        };
        let artifact = signing_bytes(&config).unwrap();
        json!({"operation":"register","workspace_id":w,"expected_revision":0,
            "candidate":Candidate { id:Uuid::new_v4(), workspace_id:w,input_revision:epoch,deletion_epoch:epoch,
                artifact_hash:hash(&artifact),training_dataset_id:Uuid::new_v4(),training_dataset_hash:hash(b"test-only"),config }, "artifact":artifact})
    }
    #[test]
    fn malformed_and_request_trust_are_rejected() {
        let (_dir, mut store, w) = fixture();
        for command in [
            json!({"operation":"status","workspace_id":w,"human_public_key":keys().1}),
            json!({"operation":"status","workspace_id":w,"input_revision":0}),
            json!({"operation":"approve","workspace_id":w,"expected_revision":0,"approval":{"approved":true}}),
        ] {
            assert!(invoke(&mut store, command).is_err());
        }
        let mut command = registration(w, store.epoch().unwrap());
        command["candidate"]["trusted"] = json!(true);
        assert!(invoke(&mut store, command).is_err());
        assert_eq!(
            invoke(&mut store, json!({"operation":"status","workspace_id":w})).unwrap()["revision"],
            0
        );
    }
    #[test]
    fn unsigned_and_unpinned_approval_cannot_promote() {
        let (_dir, mut store, w) = fixture();
        let command = registration(w, store.epoch().unwrap());
        let id = command["candidate"]["id"].clone();
        invoke(&mut store, command.clone()).unwrap();
        let payload = HumanApproval {
            workspace_id: w,
            input_revision: store.epoch().unwrap(),
            deletion_epoch: store.epoch().unwrap(),
            candidate_id: serde_json::from_value(id.clone()).unwrap(),
            artifact_hash: command["candidate"]["artifact_hash"]
                .as_str()
                .unwrap()
                .into(),
            receipt_hash: hash(b"no receipt"),
            human_id: Uuid::new_v4(),
        };
        for signature in [
            vec![],
            SigningKey::from_bytes(&[53; 32])
                .sign(&signing_bytes(&payload).unwrap())
                .to_bytes()
                .to_vec(),
        ] {
            assert!(invoke(&mut store,json!({"operation":"approve","workspace_id":w,"expected_revision":1,"approval":{"payload":payload,"signature":signature}})).is_err());
        }
        assert!(invoke(&mut store,json!({"operation":"activate","workspace_id":w,"expected_revision":1,"candidate_id":id})).is_err());
        let status = invoke(&mut store, json!({"operation":"status","workspace_id":w})).unwrap();
        assert_eq!(status["revision"], 1);
        assert!(status["active_candidate_id"].is_null());
    }
    #[test]
    fn cross_epoch_requires_explicit_invalidation_and_fresh_candidate() {
        let (_dir, mut store, w) = fixture();
        let command = registration(w, store.epoch().unwrap());
        let result = invoke(&mut store, command.clone()).unwrap();
        assert_eq!(result["candidate_state"], "candidate");
        let other = Uuid::new_v4();
        store.create_workspace(other).unwrap();
        store
            .put_document(
                DocumentKind::Source,
                other,
                Uuid::new_v4(),
                0,
                json!({"input":"changed"}),
            )
            .unwrap();
        assert!(invoke(&mut store, json!({"operation":"status","workspace_id":w})).is_err());
        let result = invoke(
            &mut store,
            json!({"operation":"invalidate","workspace_id":w,"expected_revision":1}),
        )
        .unwrap();
        assert_eq!(result["input_revision"], store.epoch().unwrap());
        let mut stale = command;
        stale["expected_revision"] = json!(2);
        assert!(invoke(&mut store, stale).is_err());
        assert!(result.get("cases").is_none());
        assert!(result.get("config").is_none());
    }
    #[test]
    fn changed_workspace_purges_evidence_and_rejects_old_candidate() {
        let (_dir, mut store, w) = fixture();
        let command = registration(w, store.epoch().unwrap());
        invoke(&mut store, command.clone()).unwrap();
        store
            .put_document(
                DocumentKind::Source,
                w,
                Uuid::new_v4(),
                0,
                json!({"input":"changed"}),
            )
            .unwrap();
        let status = invoke(&mut store, json!({"operation":"status","workspace_id":w})).unwrap();
        assert_eq!(status["revision"], 0);
        assert!(status["active_candidate_id"].is_null());
        assert!(invoke(&mut store, command).is_err());
    }
    #[test]
    fn foreign_workspace_and_shared_trust_key_are_rejected() {
        let (_dir, mut store, w) = fixture();
        let mut command = registration(w, store.epoch().unwrap());
        command["candidate"]["workspace_id"] = json!(Uuid::new_v4());
        assert!(invoke(&mut store, command).is_err());
        let (e, _) = keys();
        assert!(
            run(
                &mut store,
                json!({"operation":"status","workspace_id":w}),
                e,
                e
            )
            .is_err()
        );
    }
}
