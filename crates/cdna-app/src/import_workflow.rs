//! Bounded, atomic import pages. Hosts must authenticate the human/workspace before
//! calling this API; cursors bind input identity, and are not authorization tokens.
use anyhow::{Result, ensure};
use cdna_store::{DocumentKind, Store};
use serde::Serialize;
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::importer::{self, ImportBatch, Provider};

pub const MAX_PAGE_SOURCES: usize = 1000;
pub const MAX_PAGE_BYTES: usize = 16 * 1024 * 1024;

#[derive(Debug, Serialize)]
pub struct ImportPage {
    #[serde(flatten)]
    pub batch: ImportBatch,
    pub total_sources: usize,
    pub offset: usize,
    pub page_count: usize,
    /// Sum of serialized source payload bytes, matching the Store transaction limit.
    pub page_bytes: usize,
    pub has_more: bool,
    pub next_cursor: Option<String>,
    pub inserted: usize,
    /// Existing documents and deletion tombstones skipped by this transaction.
    pub skipped: usize,
    pub preview: bool,
}

/// Parse once per operation (at most 16 MiB), then preview or atomically commit one
/// page. Parser issues prevent *all* writes in this operation; inspect a preview to
/// diagnose them. Retrying the same input/cursor preserves existing edits/tombstones.
#[allow(clippy::too_many_arguments)]
pub fn import_page(
    store: &mut Store,
    workspace_id: Uuid,
    provider: Provider,
    bytes: &[u8],
    selected_conversation_ids: &[String],
    cursor: Option<&str>,
    page_size: usize,
    preview: bool,
) -> Result<ImportPage> {
    ensure!(
        (1..=MAX_PAGE_SOURCES).contains(&page_size),
        "IMPORT_PAGE_SIZE_LIMIT"
    );
    ensure!(store.workspaces()?.contains(&workspace_id), "SCOPE_DENIED");
    ensure!(
        selected_conversation_ids.len() <= 10_000,
        "IMPORT_SELECTION_LIMIT"
    );
    ensure!(
        selected_conversation_ids
            .iter()
            .all(|id| !id.is_empty() && id.len() <= 512),
        "IMPORT_SELECTION_INVALID"
    );
    ensure!(
        cursor.is_none_or(|c| c.len() <= 100),
        "IMPORT_CURSOR_INVALID"
    );
    let mut batch = importer::parse_export(provider, bytes, selected_conversation_ids)?;
    let mut selection = selected_conversation_ids.to_vec();
    selection.sort_unstable();
    selection.dedup();
    let binding = format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(&(
            "cdna-import-page-1",
            workspace_id,
            provider,
            &batch.artifact_hash,
            &batch.extractor_version,
            selection,
        ))?)
    );
    let total_sources = batch.sources.len();
    let offset = if let Some(cursor) = cursor {
        let parts: Vec<_> = cursor.split(':').collect();
        ensure!(
            parts.len() == 3 && parts[0] == "1" && parts[2] == binding,
            "IMPORT_CURSOR_MISMATCH"
        );
        let offset = parts[1]
            .parse::<usize>()
            .map_err(|_| anyhow::anyhow!("IMPORT_CURSOR_INVALID"))?;
        ensure!(offset < total_sources, "IMPORT_CURSOR_INVALID");
        offset
    } else {
        0
    };
    ensure!(
        preview || batch.complete,
        "IMPORT_INCOMPLETE: inspect preview issues; no sources written"
    );
    let mut documents = Vec::with_capacity(page_size.min(total_sources - offset));
    let mut page_bytes = 0;
    for source in batch.sources.iter().skip(offset).take(page_size) {
        let payload = serde_json::to_value(source)?;
        let size = serde_json::to_vec(&payload)?.len();
        ensure!(size <= 1024 * 1024, "IMPORT_SOURCE_SIZE_LIMIT");
        if page_bytes + size > MAX_PAGE_BYTES {
            break;
        }
        page_bytes += size;
        documents.push((source.source_id, payload));
    }
    let page_count = documents.len();
    let end = offset + page_count;
    let has_more = end < total_sources;
    let next_cursor = has_more.then(|| format!("1:{end}:{binding}"));
    // Move only the selected page; discard the other parsed source payloads.
    batch.sources = batch
        .sources
        .into_iter()
        .skip(offset)
        .take(page_count)
        .collect();
    let (inserted, skipped) = if preview {
        (0, 0)
    } else {
        let result = store.insert_documents_batch(DocumentKind::Source, workspace_id, documents)?;
        (result.inserted, result.skipped)
    };
    Ok(ImportPage {
        batch,
        total_sources,
        offset,
        page_count,
        page_bytes,
        has_more,
        next_cursor,
        inserted,
        skipped,
        preview,
    })
}
