use cdna_app::{
    import_workflow::{MAX_PAGE_BYTES, import_page},
    importer::Provider,
};
use cdna_store::{DocumentKind, Store};
use serde_json::json;
use uuid::Uuid;
use zeroize::Zeroizing;

fn fixture() -> (tempfile::TempDir, Store, Uuid) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::create(dir.path().join("vault"), &Zeroizing::new("a".repeat(64))).unwrap();
    let w = Uuid::new_v4();
    store.create_workspace(w).unwrap();
    (dir, store, w)
}
fn export(provider: Provider, count: usize) -> Vec<u8> {
    let value = match provider {
        Provider::Claude => {
            json!([{"uuid":"c1","chat_messages":(0..count).map(|i| json!({"uuid":format!("m{i}"),"sender":"human","text":format!("message {i}")})).collect::<Vec<_>>()}])
        }
        Provider::ChatGpt => {
            json!([{"id":"c1","mapping":(0..count).map(|i| (format!("node{i:06}"),json!({"message":{"id":format!("m{i}"),"author":{"role":"user"},"content":{"parts":[format!("message {i}")]}}}))).collect::<serde_json::Map<_,_>>()}])
        }
    };
    serde_json::to_vec(&value).unwrap()
}
#[test]
fn both_providers_page_1001_and_2500_exactly_and_retry_without_resurrection() {
    for provider in [Provider::ChatGpt, Provider::Claude] {
        for count in [1001, 2500] {
            let (_dir, mut store, w) = fixture();
            let bytes = export(provider, count);
            let preview =
                import_page(&mut store, w, provider, &bytes, &[], None, 1000, true).unwrap();
            assert_eq!(preview.total_sources, count);
            assert_eq!(preview.page_count, 1000);
            assert!(preview.has_more);
            assert_eq!(preview.inserted, 0);
            assert!(
                store
                    .list_documents(DocumentKind::Source, w, 1000, 0)
                    .unwrap()
                    .is_empty()
            );
            let mut cursor = None;
            let mut inserted = 0;
            let mut ids = std::collections::HashSet::new();
            loop {
                let page = import_page(
                    &mut store,
                    w,
                    provider,
                    &bytes,
                    &[],
                    cursor.as_deref(),
                    1000,
                    false,
                )
                .unwrap();
                assert!(page.batch.complete);
                assert!(page.page_count <= 1000 && page.page_bytes <= MAX_PAGE_BYTES);
                inserted += page.inserted;
                for source in &page.batch.sources {
                    assert!(ids.insert(source.source_id));
                }
                let retry = import_page(
                    &mut store,
                    w,
                    provider,
                    &bytes,
                    &[],
                    cursor.as_deref(),
                    1000,
                    false,
                )
                .unwrap();
                assert_eq!(retry.inserted, 0);
                assert_eq!(retry.skipped, page.page_count);
                assert_eq!(retry.next_cursor, page.next_cursor);
                cursor = page.next_cursor;
                if !page.has_more {
                    break;
                }
            }
            assert_eq!(inserted, count);
            assert_eq!(ids.len(), count);
            let stored: usize = (0..count)
                .step_by(1000)
                .map(|offset| {
                    store
                        .list_documents(DocumentKind::Source, w, 1000, offset as u32)
                        .unwrap()
                        .len()
                })
                .sum();
            assert_eq!(stored, count);
            let id = preview.batch.sources[0].source_id;
            store.delete_source_cascade(w, id, 1).unwrap();
            let retry =
                import_page(&mut store, w, provider, &bytes, &[], None, 1000, false).unwrap();
            assert_eq!(retry.inserted, 0);
            assert_eq!(retry.skipped, 1000);
            assert!(store.get_document(DocumentKind::Source, w, id).is_err());
        }
    }
}
#[test]
fn cursor_binds_export_provider_selection_and_workspace() {
    let (_dir, mut store, w) = fixture();
    let bytes = export(Provider::Claude, 3);
    let page = import_page(&mut store, w, Provider::Claude, &bytes, &[], None, 1, true).unwrap();
    let cursor = page.next_cursor.as_deref();
    let w2 = Uuid::new_v4();
    store.create_workspace(w2).unwrap();
    for target in [w2, Uuid::new_v4()] {
        assert!(
            import_page(
                &mut store,
                target,
                Provider::Claude,
                &bytes,
                &[],
                cursor,
                1,
                false
            )
            .is_err()
        );
    }
    assert!(
        import_page(
            &mut store,
            w,
            Provider::Claude,
            &export(Provider::Claude, 4),
            &[],
            cursor,
            1,
            false
        )
        .is_err()
    );
    assert!(
        import_page(
            &mut store,
            w,
            Provider::ChatGpt,
            &bytes,
            &[],
            cursor,
            1,
            false
        )
        .is_err()
    );
    assert!(
        import_page(
            &mut store,
            w,
            Provider::Claude,
            &bytes,
            &["c1".into()],
            cursor,
            1,
            false
        )
        .is_err()
    );
    assert!(
        store
            .list_documents(DocumentKind::Source, w, 1000, 0)
            .unwrap()
            .is_empty()
    );
    for size in [0, 1001] {
        assert!(
            import_page(
                &mut store,
                w,
                Provider::Claude,
                &bytes,
                &[],
                None,
                size,
                false
            )
            .is_err()
        );
    }
}
#[test]
fn selection_is_canonical_and_incomplete_exports_never_partially_commit() {
    let (_dir, mut store, w) = fixture();
    let bytes = export(Provider::Claude, 3);
    let selected = vec!["c1".into(), "missing".into()];
    let page = import_page(
        &mut store,
        w,
        Provider::Claude,
        &bytes,
        &selected,
        None,
        1,
        true,
    )
    .unwrap();
    let reversed = vec!["missing".into(), "c1".into(), "c1".into()];
    assert!(
        import_page(
            &mut store,
            w,
            Provider::Claude,
            &bytes,
            &reversed,
            page.next_cursor.as_deref(),
            1,
            false
        )
        .is_ok()
    );
    let skipped = import_page(
        &mut store,
        w,
        Provider::Claude,
        &bytes,
        &["missing".into()],
        None,
        1,
        true,
    )
    .unwrap();
    assert_eq!(skipped.total_sources, 0);
    assert_eq!(skipped.batch.skipped_conversations, 1);
    assert!(!skipped.has_more);
    let malformed = serde_json::to_vec(&json!([{"uuid":"other","chat_messages":[{"uuid":"ok","sender":"human","text":"valid"},{"uuid":"bad","sender":"human","text":"bad","created_at":"invalid"}]}])).unwrap();
    let preview = import_page(
        &mut store,
        w,
        Provider::Claude,
        &malformed,
        &[],
        None,
        1,
        true,
    )
    .unwrap();
    assert!(!preview.batch.complete);
    assert_eq!(preview.batch.issues.len(), 1);
    assert!(
        import_page(
            &mut store,
            w,
            Provider::Claude,
            &malformed,
            &[],
            None,
            1,
            false
        )
        .is_err()
    );
    assert!(
        store
            .get_document(DocumentKind::Source, w, preview.batch.sources[0].source_id)
            .is_err()
    );
}
#[test]
fn serialized_byte_budget_splits_even_below_thousand_messages() {
    let (_dir, mut store, w) = fixture();
    // A long conversation ID appears once in input, but in every source payload.
    let text = "x".repeat(250_000);
    let bytes = serde_json::to_vec(&json!([{"uuid":"c".repeat(512),"chat_messages":(0..67).map(|i|json!({"uuid":format!("{i}"),"sender":"human","text":text})).collect::<Vec<_>>()}])).unwrap();
    let page = import_page(
        &mut store,
        w,
        Provider::Claude,
        &bytes,
        &[],
        None,
        1000,
        false,
    )
    .unwrap();
    assert!(page.page_bytes <= MAX_PAGE_BYTES);
    assert_eq!(page.total_sources, 67);
    assert!(page.has_more);
    assert!(page.page_count < 67);
    let tail = import_page(
        &mut store,
        w,
        Provider::Claude,
        &bytes,
        &[],
        page.next_cursor.as_deref(),
        1000,
        false,
    )
    .unwrap();
    assert!(!tail.has_more);
    assert_eq!(page.inserted + tail.inserted, 67);
}
