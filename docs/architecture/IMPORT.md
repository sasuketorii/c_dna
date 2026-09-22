# Conversation export import

The engine supports the existing ChatGPT `mapping` and Claude `chat_messages`
JSON adapters. These are fixture-tested formats, not universal provider schema
claims. Supply a JSON array of conversations; ZIP, remote URLs, attachments, and
filesystem traversal are not supported by the parser or workflow API.

## Host integration

`cdna_app::import_workflow::import_page` accepts a mutable Store, workspace UUID,
provider, export bytes, selected conversation IDs, optional cursor, page size, and
preview flag. The host must authenticate the caller and authorize human access to
that workspace **before** invoking it. The workflow additionally checks workspace
existence. The cursor is an input-binding checksum, not an authorization token.

Every operation parses the export once, with the existing 16 MiB raw-input,
100,000 message, 10,000 conversation, and 256 KiB message-text bounds. It holds the
bounded parsed export in memory; this is not streaming parsing. Send the exact
same bytes again on continuation. A local CLI file path is handled by the host;
this module only receives bytes and never opens paths or fetches URLs. The CLI's
16 MiB file allowance does not increase the existing 256 KiB JSON command/web
request envelope. JSON escaping and envelope fields consume that smaller budget.

The default page size is a host choice (recommended 1000). The API accepts 1–1000
sources and also stops at 16 MiB of serialized source payloads, matching Store's
atomic batch limit; individual serialized sources must fit Store's 1 MiB limit.
A long metadata field can hit that serialized limit even when text alone fits.

## CLI and command entry points

For an existing unlocked owner vault and workspace, preview the first page:

```sh
cdna --vault data/local/vault import --provider chatgpt --workspace "$WORKSPACE_ID" --conversation "$CONVERSATION_ID" --page-size 1000 --preview conversations.json
```

Repeat without `--preview` to commit that page. For subsequent pages add
`--cursor "$NEXT_CURSOR"` using the successful commit response. `--conversation`
may be repeated, or omitted to import all conversations. Use `--provider claude`
for Claude exports. The command prints one page, so a 2500-source export needs
three successful commits with the default page size (or more if bytes limit it).

The authenticated `import_sources` JSON command accepts `workspace_id`,
`provider` (`chat_gpt` or `claude`), `export_json` (JSON as a string), `preview`,
`selected_conversation_ids`, `cursor`, and `page_size`. Selection and cursor
are optional; page size defaults to 1000. It uses the same workflow subject to
the smaller command envelope limit above.

## Preview, commit, and resume

1. Preview with `cursor: null`, `preview: true`, and an explicit workspace.
2. Inspect `issues` and `complete`. If parsing produced any issue, commit fails
   before writes; valid-looking rows are never silently partially imported.
3. Commit that same page with `preview: false` and the **same input cursor**.
   A preview's `next_cursor` points past the previewed rows, not at them.
4. Persist the returned `next_cursor` only after successful commit. Continue
   with it until `has_more` is false. Retry the previous input cursor after an
   ambiguous transport failure.

Cursors bind export SHA-256, provider, workspace, extractor version, and the
canonical sorted/deduplicated conversation selection. Empty selection means all
conversations. Unknown selected IDs match nothing and contribute to
`skipped_conversations`; selection changes require restarting. Foreign cursors
fail before writes. Page size may change on continuation. A cursor does not prove
that earlier pages were committed, and `has_more: false` only describes the
remaining rows after this page, not a durable whole-import completion record.

`total_sources` is the number of valid unique sources across the selected export;
`offset`, `page_count`, and `sources` describe this page. `duplicates`, `issues`,
`skipped_conversations`, and `complete` describe parsing of the entire selected
export, not cumulative database writes. `page_bytes` sums serialized source
payload bytes, not the entire response. `inserted` and `skipped` are this commit's
actual Store counts; both are zero for preview. Skips include existing documents
and tombstones; the response does not pretend all parsed sources were inserted.

Each page commits in one Store transaction. Failure leaves that page unwritten;
already committed previous pages remain. Provider conversation/message IDs retain
stable source identities; missing IDs use artifact hash and source pointer.
Retries preserve existing documents, owner edits, and deletion tombstones and
never resurrect deleted sources. Imported text remains untrusted,
`review_pending: true`, and `training_eligible: false`.

## Focused acceptance

`cargo test -p cdna-app --test import_paging` covers both providers with 1001 and
2500 messages, exact persisted counts, duplicate page retries, deletion followed
by retry, selection canonicalization, foreign cursors/workspaces, malformed
export rejection before writes, and serialized-byte page splitting. CLI and
command-boundary wiring use this API; the focused test above exercises the
public workflow and Store, not macOS Keychain interaction or live HTTP uploads.
