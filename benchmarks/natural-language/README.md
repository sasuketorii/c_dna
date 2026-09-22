# Optional local natural-language extraction experiment

This is a bounded synthetic experiment, not an adopted model, engine dependency,
personalized ranking benchmark, UI feature, or evidence of production readiness.
The fixed prompt and JSON schema are snapshots supplied by the extraction-boundary
owner. The twelve Japanese holdout questions and their semantic oracles were
written before the corrected inference run; no prompt tuning or second model was
performed. No user data, remote inference, or cloud credentials were used.

`model-manifest.json` pins the GGUF revision, LFS SHA256, source revision,
quantization, embedded tokenizer metadata digests, source tokenizer configuration,
license, runtime build, and official references. `chat-template.jinja` is the exact
server-returned template; its digest is in the results. The upstream Apache 2.0
license is retained in `QWEN-LICENSE.txt`. Model bytes are temporary and are not
included in this repository.

The runner requires Python with `jsonschema` (experiment used 4.25.1), the pinned
llama-server build, and the SHA-verified model in the experiment directory enforced
by the runner. Set `MODEL_DIR` to that temporary directory before downloading:

```sh
curl -fL --retry 2 \
  https://huggingface.co/ggml-org/Qwen3.5-0.8B-GGUF/resolve/8fea620810c4afa23dd6443f999a48574c1611a3/Qwen3.5-0.8B-Q4_0.gguf \
  -o "$MODEL_DIR/Qwen3.5-0.8B-Q4_0.gguf"
node "$REVH" command run --guard required --class heavy --intent generate \
  --root . --task cdna-nl-experiment --out "$MODEL_DIR/guard.json" \
  --timeout-ms 1020000 --guard-wait-ms 120000 \
  -- python3 scripts/natural-language-bench.py
```

The runner independently verifies size and SHA256 before execution, starts one
loopback-only CPU server with four generation and batch threads, one HTTP thread,
one parallel slot, context 4096, and generation cap 768. It uses temperature 0 and
seed 42 for a deterministic extraction trial, deliberately differing from the
model card's general text sampling recommendations; the verdict applies only to
this configuration. It validates returned JSON against the schema, rejects
repeated keys, and records complete raw responses and latency. Schema validity
is not semantic correctness or proof of Rust domain-boundary acceptance.

The correct API is POST `http://127.0.0.1:18765/v1/chat/completions`, model alias
`cdna-qwen35-08b-q4`, nonstreaming, with:

```json
{"response_format":{"type":"json_schema","json_schema":{"name":"extraction","strict":true,"schema":{}}},"chat_template_kwargs":{"enable_thinking":false},"max_tokens":768,"temperature":0,"seed":42}
```

Replace the empty schema with `schema.json`, and supply system/user `messages`.
The pinned official server README's top-level `schema` example for type
`json_schema` does not match the pinned parser: it reads
`response_format.json_schema.schema`. The original aborted run and its malformed
responses are preserved as `results/transport-format-failure.json`; this is a
transport-format defect, not included in model accuracy or latency statistics.
Source evidence: [pinned parser](https://github.com/ggml-org/llama.cpp/blob/5266f24da/tools/server/server-common.cpp#L1168-L1178).

The script terminates and waits only for its own `Popen` child, escalating to kill
only on that same child after ten seconds, including on handled interruption.
It refuses an occupied port. An externally delivered SIGKILL cannot run Python
cleanup; the outer command guard is therefore not evidence that cleanup occurred.
The result records actual child exit, and the report records the final port check.

Latency starts immediately before HTTP input and ends after JSON/schema
validation. Loading is separate; the first request is included, shared prompt
cache reuse is enabled, and no OS cache eviction is claimed. p50/p95 use nearest
rank over valid structured responses, with all-request outcomes reported
separately. Twelve heterogeneous one-pass samples are exploratory and cannot
establish a production latency SLO. Resource usage covers the whole child server,
not just token generation. Semantic adjudication checks every frozen oracle,
including correct abstention; failed schema or transport never counts as success.
