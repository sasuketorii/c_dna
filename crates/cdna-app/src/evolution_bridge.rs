//! Encrypted persistence for the signed evolution state machine.
//!
//! The host supplies trusted public keys and the current input revision, and
//! authenticates the caller before invoking mutations. No signing key, human
//! label generation, evaluator, or request-derived trust configuration lives here.
//! Read through this bridge before inference; a returned snapshot is not a lease
//! to use a model after the vault changes. State and active pointer share one
//! bounded document, committed with document-revision and vault-epoch CAS.

use anyhow::{Context, Result, ensure};
use cdna_evolution::{
    Candidate, Dataset, EvaluationReceipt, Evolution, HumanApproval, Signed, State,
};
use cdna_store::{DocumentKind, Store, StoreError};
use ed25519_dalek::VerifyingKey;
use serde::Serialize;
use uuid::Uuid;

pub const EVOLUTION_DOCUMENT_ID: Uuid = Store::EVOLUTION_DOCUMENT_ID;
const MAX_STATE_BYTES: usize = 1024 * 1024;

/// Validated at the revision/epoch shown, never deserializable as trusted state.
pub struct EvolutionSnapshot {
    pub revision: u64,
    pub input_revision: u64,
    pub deletion_epoch: u64,
    evolution: Evolution,
}

// Transport summaries must not reveal held-out labels or signed case rows.
// The complete evidence graph is serialized only into the encrypted document.
impl Serialize for EvolutionSnapshot {
    fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Summary<'a> {
            revision: u64,
            input_revision: u64,
            deletion_epoch: u64,
            active_candidate: Option<&'a Candidate>,
            active_domains: Vec<&'a str>,
        }
        Summary {
            revision: self.revision,
            input_revision: self.input_revision,
            deletion_epoch: self.deletion_epoch,
            active_candidate: self.active_candidate(),
            active_domains: self.active_domains(),
        }
        .serialize(serializer)
    }
}

impl EvolutionSnapshot {
    pub fn active_candidate(&self) -> Option<&Candidate> {
        self.evolution.active_candidate()
    }

    pub fn active_domains(&self) -> Vec<&str> {
        self.evolution.active_domains()
    }

    pub fn state(&self, id: Uuid) -> Option<State> {
        self.evolution.state(id)
    }
}

/// Construct only from host-managed trust configuration, never command JSON.
pub struct EvolutionBridge {
    evaluator: VerifyingKey,
    human: VerifyingKey,
}

impl EvolutionBridge {
    pub fn new(evaluator: VerifyingKey, human: VerifyingKey) -> Result<Self> {
        ensure!(
            evaluator != human,
            "EVOLUTION_UNTRUSTED: distinct keys required"
        );
        Ok(Self { evaluator, human })
    }

    /// Missing state is empty; corrupt, foreign, stale or unsigned state errors.
    /// `input_revision` comes from the host's current input snapshot authority.
    pub fn load(
        &self,
        store: &Store,
        workspace: Uuid,
        input_revision: u64,
    ) -> Result<EvolutionSnapshot> {
        ensure!(
            store.workspaces()?.contains(&workspace),
            "EVOLUTION_INVALID: unknown workspace"
        );
        let epoch = store.epoch()?;
        let (revision, mut evolution) =
            match store.get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID) {
                Ok(document) => {
                    // Store already bounds raw document bytes. Check again before
                    // allocating the typed evidence graph and verifying signatures.
                    ensure!(
                        serde_json::to_vec(&document.payload)?.len() <= MAX_STATE_BYTES,
                        "EVOLUTION_INVALID: state too large"
                    );
                    let evolution = serde_json::from_value(document.payload)
                        .context("EVOLUTION_INVALID: malformed state")?;
                    (document.revision, evolution)
                }
                Err(StoreError::NotFound) => (0, Evolution::new(workspace, input_revision, epoch)),
                Err(error) => return Err(error.into()),
            };
        evolution
            .validate_restored(
                workspace,
                input_revision,
                epoch,
                &self.evaluator,
                &self.human,
            )
            .map_err(|error| anyhow::anyhow!("EVOLUTION_UNTRUSTED: {error}"))?;
        ensure!(
            store.epoch()? == epoch,
            "EVOLUTION_STALE: vault changed during read"
        );
        // Detect a concurrent evolution-only writer as well as an input writer.
        let latest_revision =
            match store.get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID) {
                Ok(document) => document.revision,
                Err(StoreError::NotFound) => 0,
                Err(error) => return Err(error.into()),
            };
        ensure!(
            latest_revision == revision,
            "EVOLUTION_CONFLICT: state changed during read"
        );
        ensure!(
            store.epoch()? == epoch,
            "EVOLUTION_STALE: vault changed during read"
        );
        Ok(EvolutionSnapshot {
            revision,
            input_revision,
            deletion_epoch: epoch,
            evolution,
        })
    }

    pub fn register(
        &self,
        store: &mut Store,
        workspace: Uuid,
        input_revision: u64,
        expected_revision: u64,
        candidate: Candidate,
        artifact: &[u8],
    ) -> Result<EvolutionSnapshot> {
        self.change(
            store,
            workspace,
            input_revision,
            expected_revision,
            |state| state.register(candidate, artifact),
        )
    }

    pub fn evaluate(
        &self,
        store: &mut Store,
        workspace: Uuid,
        input_revision: u64,
        expected_revision: u64,
        receipt: Signed<EvaluationReceipt>,
        dataset: Dataset,
    ) -> Result<EvolutionSnapshot> {
        self.change(
            store,
            workspace,
            input_revision,
            expected_revision,
            |state| {
                state
                    .evaluate(
                        receipt.payload.candidate_id,
                        receipt,
                        dataset,
                        &self.evaluator,
                    )
                    .map(|_| ())
            },
        )
    }

    /// The signature must come from an explicit, authenticated owner decision.
    pub fn approve(
        &self,
        store: &mut Store,
        workspace: Uuid,
        input_revision: u64,
        expected_revision: u64,
        approval: Signed<HumanApproval>,
    ) -> Result<EvolutionSnapshot> {
        self.change(
            store,
            workspace,
            input_revision,
            expected_revision,
            |state| state.approve(approval.payload.candidate_id, approval, &self.human),
        )
    }

    pub fn activate(
        &self,
        store: &mut Store,
        workspace: Uuid,
        input_revision: u64,
        expected_revision: u64,
        candidate_id: Uuid,
    ) -> Result<EvolutionSnapshot> {
        self.change(
            store,
            workspace,
            input_revision,
            expected_revision,
            |state| state.activate(candidate_id),
        )
    }

    pub fn rollback(
        &self,
        store: &mut Store,
        workspace: Uuid,
        input_revision: u64,
        expected_revision: u64,
        candidate_id: Uuid,
    ) -> Result<EvolutionSnapshot> {
        self.change(
            store,
            workspace,
            input_revision,
            expected_revision,
            |state| state.rollback(candidate_id),
        )
    }

    /// Explicit recovery after an input change. Historical bindings are used
    /// only to authenticate old evidence before quarantining it; they never
    /// authorize inference. Corrupt state still errors and is not overwritten.
    pub fn invalidate(
        &self,
        store: &mut Store,
        workspace: Uuid,
        input_revision: u64,
        expected_revision: u64,
    ) -> Result<EvolutionSnapshot> {
        let epoch = store.epoch()?;
        let document =
            store.get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID)?;
        ensure!(
            document.revision == expected_revision,
            "EVOLUTION_CONFLICT: revision mismatch"
        );
        ensure!(
            serde_json::to_vec(&document.payload)?.len() <= MAX_STATE_BYTES,
            "EVOLUTION_INVALID: state too large"
        );
        let historical_revision = document
            .payload
            .get("input_revision")
            .and_then(serde_json::Value::as_u64)
            .context("EVOLUTION_INVALID: missing input revision")?;
        let historical_epoch = document
            .payload
            .get("deletion_epoch")
            .and_then(serde_json::Value::as_u64)
            .context("EVOLUTION_INVALID: missing deletion epoch")?;
        let mut evolution: Evolution = serde_json::from_value(document.payload)
            .context("EVOLUTION_INVALID: malformed state")?;
        evolution
            .validate_restored(
                workspace,
                historical_revision,
                historical_epoch,
                &self.evaluator,
                &self.human,
            )
            .map_err(|error| anyhow::anyhow!("EVOLUTION_UNTRUSTED: {error}"))?;
        evolution
            .invalidate(input_revision, epoch)
            .map_err(|error| anyhow::anyhow!("EVOLUTION_INVALID: {error}"))?;
        self.persist(
            store,
            workspace,
            EvolutionSnapshot {
                revision: expected_revision,
                input_revision,
                deletion_epoch: epoch,
                evolution,
            },
        )
    }

    fn change(
        &self,
        store: &mut Store,
        workspace: Uuid,
        input_revision: u64,
        expected_revision: u64,
        transition: impl FnOnce(&mut Evolution) -> cdna_evolution::Result<()>,
    ) -> Result<EvolutionSnapshot> {
        let mut snapshot = self.load(store, workspace, input_revision)?;
        ensure!(
            snapshot.revision == expected_revision,
            "EVOLUTION_CONFLICT: revision mismatch"
        );
        transition(&mut snapshot.evolution)
            .map_err(|error| anyhow::anyhow!("EVOLUTION_INVALID: {error}"))?;
        self.persist(store, workspace, snapshot)
    }

    fn persist(
        &self,
        store: &mut Store,
        workspace: Uuid,
        mut snapshot: EvolutionSnapshot,
    ) -> Result<EvolutionSnapshot> {
        let payload = serde_json::to_value(&snapshot.evolution)?;
        ensure!(
            serde_json::to_vec(&payload)?.len() <= MAX_STATE_BYTES,
            "EVOLUTION_INVALID: state too large"
        );
        let document = store.put_evolution_document(
            workspace,
            EVOLUTION_DOCUMENT_ID,
            snapshot.revision,
            snapshot.deletion_epoch,
            payload,
        )?;
        snapshot.revision = document.revision;
        Ok(snapshot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cdna_evolution::{
        DatasetCase, ModelConfig, Outcome, PairedCase, Split, content_hash, hash, signing_bytes,
    };
    use ed25519_dalek::{Signer, SigningKey};
    use serde_json::json;
    use zeroize::Zeroizing;

    // Synthetic cryptographic/state-machine fixtures only: these labels and
    // timings are NOT real human evidence, accuracy measurements or promotion.
    struct Fixture {
        workspace: Uuid,
        candidate: Candidate,
        artifact: Vec<u8>,
        dataset: Dataset,
        receipt: Signed<EvaluationReceipt>,
        approval: Signed<HumanApproval>,
        evaluator: SigningKey,
        human: SigningKey,
    }

    fn signed<T: Serialize>(payload: T, key: &SigningKey) -> Signed<T> {
        let signature = key
            .sign(&signing_bytes(&payload).unwrap())
            .to_bytes()
            .to_vec();
        Signed { payload, signature }
    }

    impl Fixture {
        fn new(workspace: Uuid, epoch: u64, baseline: Option<String>) -> Self {
            let evaluator = SigningKey::from_bytes(&[31; 32]);
            let human = SigningKey::from_bytes(&[32; 32]);
            let mut cases = Vec::new();
            for index in 0..201 {
                cases.push(DatasetCase {
                    id: Uuid::new_v4(),
                    family_id: Uuid::new_v4(),
                    split: if index == 0 {
                        Split::Train
                    } else {
                        Split::Test
                    },
                    timestamp: if index == 0 { 1 } else { 2 },
                    input_hash: hash(format!("synthetic-test-input-{index}").as_bytes()),
                    answer: "synthetic-a".into(),
                    independent_human_answer: true,
                    model_exposure: false,
                    domain: "synthetic-test-only".into(),
                });
            }
            let dataset = Dataset {
                id: Uuid::new_v4(),
                workspace_id: workspace,
                input_revision: 1,
                deletion_epoch: epoch,
                cases,
            };
            let config = ModelConfig {
                inference: None,
                feature_version: "1.0".into(),
                weights: vec![if baseline.is_some() { 0.25 } else { 0. }; 17],
            };
            let artifact = signing_bytes(&config).unwrap();
            let candidate = Candidate {
                id: Uuid::new_v4(),
                workspace_id: workspace,
                input_revision: 1,
                deletion_epoch: epoch,
                artifact_hash: hash(&artifact),
                training_dataset_id: dataset.id,
                training_dataset_hash: content_hash(&dataset).unwrap(),
                config,
            };
            let outcome = Outcome {
                choice: Some("synthetic-a".into()),
                policy_violations: 0,
                latency_us: 10,
                peak_memory_bytes: 1024,
            };
            let receipt = signed(
                EvaluationReceipt {
                    id: Uuid::new_v4(),
                    workspace_id: workspace,
                    input_revision: 1,
                    deletion_epoch: epoch,
                    candidate_id: candidate.id,
                    artifact_hash: candidate.artifact_hash.clone(),
                    baseline_artifact_hash: baseline.unwrap_or_else(|| hash(b"synthetic-baseline")),
                    dataset_id: dataset.id,
                    dataset_hash: content_hash(&dataset).unwrap(),
                    evaluator_version: "synthetic-persistence-test-only".into(),
                    test_previously_exposed: false,
                    rows: dataset
                        .cases
                        .iter()
                        .filter(|case| case.split == Split::Test)
                        .map(|case| PairedCase {
                            case_id: case.id,
                            input_hash: case.input_hash.clone(),
                            candidate: outcome.clone(),
                            baseline: outcome.clone(),
                        })
                        .collect(),
                    candidate_learning_seconds: 1,
                    baseline_learning_seconds: 1,
                },
                &evaluator,
            );
            let approval = signed(
                HumanApproval {
                    workspace_id: workspace,
                    input_revision: 1,
                    deletion_epoch: epoch,
                    candidate_id: candidate.id,
                    artifact_hash: candidate.artifact_hash.clone(),
                    receipt_hash: content_hash(&receipt).unwrap(),
                    human_id: Uuid::new_v4(),
                },
                &human,
            );
            Self {
                workspace,
                candidate,
                artifact,
                dataset,
                receipt,
                approval,
                evaluator,
                human,
            }
        }

        fn bridge(&self) -> EvolutionBridge {
            EvolutionBridge::new(self.evaluator.verifying_key(), self.human.verifying_key())
                .unwrap()
        }

        fn promote(&self, store: &mut Store, revision: u64) -> EvolutionSnapshot {
            let bridge = self.bridge();
            let snapshot = bridge
                .register(
                    store,
                    self.workspace,
                    1,
                    revision,
                    self.candidate.clone(),
                    &self.artifact,
                )
                .unwrap();
            assert_eq!(snapshot.state(self.candidate.id), Some(State::Candidate));
            let snapshot = bridge
                .evaluate(
                    store,
                    self.workspace,
                    1,
                    snapshot.revision,
                    self.receipt.clone(),
                    self.dataset.clone(),
                )
                .unwrap();
            assert_eq!(snapshot.state(self.candidate.id), Some(State::Evaluated));
            let snapshot = bridge
                .approve(
                    store,
                    self.workspace,
                    1,
                    snapshot.revision,
                    self.approval.clone(),
                )
                .unwrap();
            assert_eq!(snapshot.state(self.candidate.id), Some(State::Approved));
            if revision == 0 {
                assert!(snapshot.active_candidate().is_none());
            }
            bridge
                .activate(
                    store,
                    self.workspace,
                    1,
                    snapshot.revision,
                    self.candidate.id,
                )
                .unwrap()
        }
    }

    fn vault() -> (tempfile::TempDir, Store, Zeroizing<String>, Uuid) {
        let dir = tempfile::tempdir().unwrap();
        let key = Zeroizing::new("synthetic-test-vault-key-".repeat(4));
        let store = Store::create(dir.path().join("vault.db"), &key).unwrap();
        let workspace = Uuid::new_v4();
        store.create_workspace(workspace).unwrap();
        (dir, store, key, workspace)
    }

    #[test]
    fn encrypted_promotion_reopen_and_rollback_are_atomic() {
        let (dir, mut store, key, workspace) = vault();
        let epoch = store.epoch().unwrap();
        let first = Fixture::new(workspace, epoch, None);
        let snapshot = first.promote(&mut store, 0);
        assert_eq!(snapshot.revision, 4);
        assert_eq!(snapshot.active_candidate().unwrap().id, first.candidate.id);
        assert_eq!(snapshot.active_domains(), vec!["synthetic-test-only"]);
        let summary = serde_json::to_value(&snapshot).unwrap();
        assert!(summary.get("evolution").is_none());
        assert!(!summary.to_string().contains("synthetic-a"));
        assert_eq!(summary["active_candidate"]["id"], json!(first.candidate.id));
        assert_eq!(store.epoch().unwrap(), epoch);
        store.lock();
        assert!(first.bridge().load(&store, workspace, 1).is_err());
        let mut store = Store::open(dir.path().join("vault.db"), &key).unwrap();
        let snapshot = first.bridge().load(&store, workspace, 1).unwrap();
        assert_eq!(snapshot.active_candidate().unwrap().id, first.candidate.id);
        let second = Fixture::new(
            workspace,
            epoch,
            Some(first.candidate.artifact_hash.clone()),
        );
        let snapshot = second.promote(&mut store, snapshot.revision);
        assert_eq!(snapshot.state(first.candidate.id), Some(State::Retired));
        assert_eq!(snapshot.active_candidate().unwrap().id, second.candidate.id);
        let rolled_back = first
            .bridge()
            .rollback(
                &mut store,
                workspace,
                1,
                snapshot.revision,
                first.candidate.id,
            )
            .unwrap();
        assert_eq!(rolled_back.revision, 9);
        drop(store);
        let store = Store::open(dir.path().join("vault.db"), &key).unwrap();
        let restored = first.bridge().load(&store, workspace, 1).unwrap();
        assert_eq!(restored.active_candidate().unwrap().id, first.candidate.id);
        assert_eq!(restored.state(second.candidate.id), Some(State::Retired));
        assert_eq!(store.epoch().unwrap(), epoch);
        let raw = std::fs::read(dir.path().join("vault.db")).unwrap();
        assert!(
            !raw.windows(b"synthetic-test-only".len())
                .any(|w| w == b"synthetic-test-only")
        );
    }

    #[test]
    fn restored_signature_artifact_and_pointer_tampering_fail_closed() {
        let (_dir, mut store, _key, workspace) = vault();
        let fixture = Fixture::new(workspace, store.epoch().unwrap(), None);
        fixture.promote(&mut store, 0);
        let original = store
            .get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID)
            .unwrap();
        let candidate = fixture.candidate.id.to_string();
        let mut revision = original.revision;
        for mutation in 0..4 {
            let mut payload = original.payload.clone();
            match mutation {
                0 => payload["entries"][&candidate]["evaluation"]["signature"][0] = json!(255),
                1 => {
                    payload["entries"][&candidate]["approval"]["payload"]["human_id"] =
                        json!(Uuid::new_v4())
                }
                2 => {
                    payload["entries"][&candidate]["candidate"]["config"]["weights"][0] = json!(123)
                }
                _ => payload["active"] = json!(Uuid::new_v4()),
            }
            let document = store
                .put_evolution_document(
                    workspace,
                    EVOLUTION_DOCUMENT_ID,
                    revision,
                    store.epoch().unwrap(),
                    payload,
                )
                .unwrap();
            revision = document.revision;
            assert!(
                fixture.bridge().load(&store, workspace, 1).is_err(),
                "mutation {mutation}"
            );
            assert!(
                fixture
                    .bridge()
                    .activate(&mut store, workspace, 1, revision, fixture.candidate.id)
                    .is_err()
            );
            assert_eq!(
                store
                    .get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID)
                    .unwrap()
                    .revision,
                revision
            );
        }
    }

    #[test]
    fn stale_epoch_revision_and_rotated_trust_disable_active_access() {
        let (_dir, mut store, _key, workspace) = vault();
        let fixture = Fixture::new(workspace, store.epoch().unwrap(), None);
        let snapshot = fixture.promote(&mut store, 0);
        assert!(fixture.bridge().load(&store, workspace, 2).is_err());
        let unknown_key = SigningKey::from_bytes(&[33; 32]).verifying_key();
        let other_evaluator =
            EvolutionBridge::new(unknown_key, fixture.human.verifying_key()).unwrap();
        let other_human =
            EvolutionBridge::new(fixture.evaluator.verifying_key(), unknown_key).unwrap();
        assert!(other_evaluator.load(&store, workspace, 1).is_err());
        assert!(other_human.load(&store, workspace, 1).is_err());
        let historical = store
            .get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID)
            .unwrap();
        store
            .propose(
                workspace,
                Uuid::new_v4(),
                Uuid::new_v4(),
                json!({"synthetic":true}),
            )
            .unwrap();
        assert!(store.epoch().unwrap() > snapshot.deletion_epoch);
        let purged = fixture.bridge().load(&store, workspace, 1).unwrap();
        assert_eq!(purged.revision, 0);
        assert!(purged.active_candidate().is_none());
        // Even a restored, previously valid signed document cannot reactivate
        // after the input mutation. The low-level Store injection models replay.
        let replayed = store
            .put_evolution_document(
                workspace,
                EVOLUTION_DOCUMENT_ID,
                0,
                store.epoch().unwrap(),
                historical.payload,
            )
            .unwrap();
        assert!(fixture.bridge().load(&store, workspace, 1).is_err());
        assert!(
            fixture
                .bridge()
                .rollback(
                    &mut store,
                    workspace,
                    1,
                    replayed.revision,
                    fixture.candidate.id
                )
                .is_err()
        );
        assert_eq!(
            store
                .get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID)
                .unwrap()
                .revision,
            replayed.revision
        );
        let invalidated = fixture
            .bridge()
            .invalidate(&mut store, workspace, 1, replayed.revision)
            .unwrap();
        assert!(invalidated.active_candidate().is_none());
        assert_eq!(
            invalidated.state(fixture.candidate.id),
            Some(State::Quarantined)
        );
        assert_eq!(invalidated.deletion_epoch, store.epoch().unwrap());
        assert!(
            fixture
                .bridge()
                .rollback(
                    &mut store,
                    workspace,
                    1,
                    invalidated.revision,
                    fixture.candidate.id
                )
                .is_err()
        );
        let fresh = Fixture::new(workspace, store.epoch().unwrap(), None);
        let promoted = fresh.promote(&mut store, invalidated.revision);
        assert_eq!(promoted.active_candidate().unwrap().id, fresh.candidate.id);
        assert!(
            fixture
                .bridge()
                .invalidate(&mut store, workspace, 1, promoted.revision)
                .is_err()
        );
    }

    #[test]
    fn concurrent_input_or_state_writer_cannot_commit_a_stale_transition() {
        let (dir, mut store, key, workspace) = vault();
        let mut other = Store::open(dir.path().join("vault.db"), &key).unwrap();
        let fixture = Fixture::new(workspace, store.epoch().unwrap(), None);
        let bridge = fixture.bridge();
        let result = bridge.change(&mut store, workspace, 1, 0, |state| {
            state.register(fixture.candidate.clone(), &fixture.artifact)?;
            other
                .propose(
                    workspace,
                    Uuid::new_v4(),
                    Uuid::new_v4(),
                    json!({"synthetic": true}),
                )
                .unwrap();
            Ok(())
        });
        assert!(result.is_err());
        assert!(matches!(
            store.get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID),
            Err(StoreError::NotFound)
        ));
        let fixture = Fixture::new(workspace, store.epoch().unwrap(), None);
        let snapshot = bridge
            .register(
                &mut store,
                workspace,
                1,
                0,
                fixture.candidate.clone(),
                &fixture.artifact,
            )
            .unwrap();
        let result = bridge.change(&mut store, workspace, 1, snapshot.revision, |state| {
            state.evaluate(
                fixture.candidate.id,
                fixture.receipt.clone(),
                fixture.dataset.clone(),
                &fixture.evaluator.verifying_key(),
            )?;
            let document = other
                .get_document(DocumentKind::Improvement, workspace, EVOLUTION_DOCUMENT_ID)
                .unwrap();
            other
                .put_evolution_document(
                    workspace,
                    EVOLUTION_DOCUMENT_ID,
                    document.revision,
                    other.epoch().unwrap(),
                    document.payload,
                )
                .unwrap();
            Ok(())
        });
        assert!(result.is_err());
        let restored = bridge.load(&store, workspace, 1).unwrap();
        assert_eq!(restored.revision, 2);
        assert_eq!(restored.state(fixture.candidate.id), Some(State::Candidate));
        assert!(restored.active_candidate().is_none());
    }

    #[test]
    fn missing_approval_bad_signature_foreign_workspace_and_cas_do_not_write() {
        let (_dir, mut store, _key, workspace) = vault();
        let fixture = Fixture::new(workspace, store.epoch().unwrap(), None);
        let bridge = fixture.bridge();
        assert!(
            EvolutionBridge::new(
                fixture.evaluator.verifying_key(),
                fixture.evaluator.verifying_key()
            )
            .is_err()
        );
        assert!(bridge.load(&store, Uuid::new_v4(), 1).is_err());
        let foreign = Uuid::new_v4();
        store.create_workspace(foreign).unwrap();
        assert!(
            bridge
                .register(
                    &mut store,
                    foreign,
                    1,
                    0,
                    fixture.candidate.clone(),
                    &fixture.artifact
                )
                .is_err()
        );
        let snapshot = bridge
            .register(
                &mut store,
                workspace,
                1,
                0,
                fixture.candidate.clone(),
                &fixture.artifact,
            )
            .unwrap();
        assert!(
            bridge
                .activate(
                    &mut store,
                    workspace,
                    1,
                    snapshot.revision,
                    fixture.candidate.id
                )
                .is_err()
        );
        assert!(
            bridge
                .rollback(
                    &mut store,
                    workspace,
                    1,
                    snapshot.revision,
                    fixture.candidate.id
                )
                .is_err()
        );
        assert!(
            bridge
                .evaluate(
                    &mut store,
                    workspace,
                    1,
                    0,
                    fixture.receipt.clone(),
                    fixture.dataset.clone()
                )
                .is_err()
        );
        let forged = signed(fixture.receipt.payload.clone(), &fixture.human);
        assert!(
            bridge
                .evaluate(
                    &mut store,
                    workspace,
                    1,
                    snapshot.revision,
                    forged,
                    fixture.dataset.clone()
                )
                .is_err()
        );
        assert_eq!(bridge.load(&store, workspace, 1).unwrap().revision, 1);
        let snapshot = bridge
            .evaluate(
                &mut store,
                workspace,
                1,
                1,
                fixture.receipt.clone(),
                fixture.dataset.clone(),
            )
            .unwrap();
        let forged_approval = signed(fixture.approval.payload.clone(), &fixture.evaluator);
        assert!(
            bridge
                .approve(&mut store, workspace, 1, snapshot.revision, forged_approval)
                .is_err()
        );
        assert!(
            bridge
                .activate(
                    &mut store,
                    workspace,
                    1,
                    snapshot.revision,
                    fixture.candidate.id
                )
                .is_err()
        );
        let final_snapshot = bridge.load(&store, workspace, 1).unwrap();
        assert_eq!(final_snapshot.revision, 2);
        assert!(final_snapshot.active_candidate().is_none());
    }
}
