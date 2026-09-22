#!/usr/bin/env python3
"""Bounded, synthetic-only local extraction experiment. Run via REVH heavy guard."""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import platform
import resource
import signal
import socket
import subprocess
import time
import urllib.request
import urllib.error
import jsonschema

MODEL_SHA = '57d1997790d1744fba5b40a7317df71ea5e2acee28c47e78f0cce39c0703f8cf'
MODEL_SIZE = 563036064
BASE = Path(__file__).resolve().parent.parent
BENCH = BASE / 'benchmarks/natural-language'
MODEL_DIR = Path('/tmp/cdna-nl-model-experiment')
PORT = 18765
ORIGIN = f'http://127.0.0.1:{PORT}'
# Never use environment proxy settings for loopback requests.
HTTP = urllib.request.build_opener(urllib.request.ProxyHandler({}))

def sha(path):
    h = hashlib.sha256()
    with path.open('rb') as f:
        for part in iter(lambda: f.read(1024 * 1024), b''):
            h.update(part)
    return h.hexdigest()

def request(path, data=None, timeout=120):
    encoded = None if data is None else json.dumps(data, ensure_ascii=False).encode()
    req = urllib.request.Request(ORIGIN + path, data=encoded, headers={'Content-Type': 'application/json'})
    with HTTP.open(req, timeout=timeout) as res:
        raw = res.read(256 * 1024 + 1)
        if len(raw) > 256 * 1024:
            raise ValueError('response exceeds 256 KiB')
        return json.loads(raw)

def unique_object(pairs):
    obj = {}
    for key, value in pairs:
        if key in obj:
            raise ValueError('duplicate JSON key: ' + key)
        obj[key] = value
    return obj

def percentile(values, pct):
    return sorted(values)[max(0, math.ceil(len(values) * pct) - 1)] if values else None

def main():
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument('--server', default='/opt/homebrew/bin/llama-server')
    ap.add_argument('--out', default='benchmarks/natural-language/results/qwen35-08b-q4.json')
    args = ap.parse_args()
    model = MODEL_DIR / 'Qwen3.5-0.8B-Q4_0.gguf'
    if model.stat().st_size != MODEL_SIZE or sha(model) != MODEL_SHA:
        raise SystemExit('model size or LFS SHA256 mismatch')
    # Fail closed if occupied: never attach to, reuse, or terminate someone else's server.
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', PORT))
    source_files = [BENCH / p for p in ['system-prompt.txt', 'schema.json', 'holdout.json']]
    hashes = {str(p.relative_to(BASE)): sha(p) for p in source_files}
    schema = json.loads((BENCH / 'schema.json').read_text())
    validator = jsonschema.Draft202012Validator(schema)
    prompt = (BENCH / 'system-prompt.txt').read_text().rstrip('\n')
    cases = json.loads((BENCH / 'holdout.json').read_text())
    command = [args.server, '--model', str(model), '--alias', 'cdna-qwen35-08b-q4', '--host', '127.0.0.1', '--port', str(PORT), '--parallel', '1', '--threads', '4', '--threads-batch', '4', '--threads-http', '1', '--ctx-size', '4096', '--n-predict', '768', '--gpu-layers', '0', '--batch-size', '256', '--ubatch-size', '128', '--jinja', '--reasoning', 'off', '--no-ui', '--cors-origins', 'http://127.0.0.1:18765', '--no-cors-credentials']
    result = {'experiment': 'synthetic Japanese business extraction; not personalized scoring', 'model_sha256': MODEL_SHA, 'input_hashes': hashes, 'server_binary_sha256': sha(Path(args.server).resolve()), 'platform': platform.platform(), 'max_tokens': 768, 'sampling': {'temperature': 0, 'seed': 42}, 'cases': [], 'semantic_adjudication': 'pending; JSON validity is not correctness', 'launch_argv': [Path(args.server).name, *command[1:]], 'model_path_note': 'Model file is experiment-local temporary storage; not distributed'}
    # Avoid absolute paths in durable artifacts.
    result['launch_argv'][2] = '$MODEL_DIR/Qwen3.5-0.8B-Q4_0.gguf'
    out = BASE / args.out
    out.parent.mkdir(parents=True, exist_ok=True)
    proc = None
    started = time.monotonic()
    def interrupted(signum, _frame):
        raise KeyboardInterrupt(f'signal {signum}')
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    try:
        with (BENCH / 'results/server.log').open('w') as log:
            proc = subprocess.Popen(command, stdout=log, stderr=subprocess.STDOUT, start_new_session=True)
            result['owned_pid'] = proc.pid
            for _ in range(120):
                if proc.poll() is not None:
                    raise RuntimeError(f'owned server exited {proc.returncode}')
                try:
                    if request('/health', timeout=1).get('status') == 'ok':
                        break
                except (OSError, ValueError):
                    pass
                time.sleep(0.5)
            else:
                raise TimeoutError('server startup timeout')
            result['startup_seconds'] = time.monotonic() - started
            result['models'] = request('/v1/models')
            props = request('/props')
            template = props.get('chat_template', '')
            (BENCH / 'chat-template.jinja').write_text(template)
            result['chat_template_sha256'] = hashlib.sha256(template.encode()).hexdigest()
            (MODEL_DIR / 'ready.json').write_text(json.dumps({'pid': proc.pid, 'origin': ORIGIN, 'alias': 'cdna-qwen35-08b-q4'}))
            print(f'READY {ORIGIN} pid={proc.pid}', flush=True)
            for case in cases:
                if time.monotonic() - started > 900:
                    raise TimeoutError('total experiment budget exceeded')
                row = {'id': case['id'], 'input': case['text'], 'oracle': case['oracle']}
                payload = {'model': 'cdna-qwen35-08b-q4', 'messages': [{'role': 'system', 'content': prompt}, {'role': 'user', 'content': case['text']}], 'response_format': {'type': 'json_schema', 'json_schema': {'name': 'extraction', 'strict': True, 'schema': schema}}, 'chat_template_kwargs': {'enable_thinking': False}, 'max_tokens': 768, 'temperature': 0, 'seed': 42, 'stream': False}
                begin = time.monotonic()
                try:
                    response = request('/v1/chat/completions', payload)
                    row['response'] = response
                    choice = response['choices'][0]
                    proposal = json.loads(choice['message']['content'], object_pairs_hook=unique_object)
                    validator.validate(proposal)
                    row['schema_valid'] = True
                    row['proposal'] = proposal
                    row['empty_candidates_abstention'] = not proposal['candidates']
                    row['finish_reason'] = choice['finish_reason']
                    if choice['finish_reason'] != 'stop':
                        row['schema_valid'] = False
                        row['error'] = 'generation did not finish normally'
                except (OSError, ValueError, KeyError, IndexError, jsonschema.ValidationError) as exc:
                    row['schema_valid'] = False
                    row['error'] = str(exc)[:2000]
                row['input_to_validated_result_seconds'] = time.monotonic() - begin
                result['cases'].append(row)
                out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
                print(f"{case['id']}: {row['input_to_validated_result_seconds']:.3f}s schema_valid={row['schema_valid']}", flush=True)
    except BaseException as exc:
        result['experiment_error'] = f'{type(exc).__name__}: {exc}'
        raise
    finally:
        if proc is not None and proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=10)
            except subprocess.TimeoutExpired:
                proc.kill()
                proc.wait(timeout=5)
        result['server_returncode'] = None if proc is None else proc.returncode
        result['owned_process_exited'] = proc is not None and proc.poll() is not None
        usage = resource.getrusage(resource.RUSAGE_CHILDREN)
        result['server_resources'] = {'user_cpu_seconds': usage.ru_utime, 'system_cpu_seconds': usage.ru_stime, 'peak_rss_bytes': usage.ru_maxrss if platform.system() == 'Darwin' else usage.ru_maxrss * 1024, 'scope': 'whole owned server lifetime, includes load and all requests, excludes Python'}
        latencies = [r['input_to_validated_result_seconds'] for r in result['cases'] if r['schema_valid']]
        result['summary'] = {'n': len(result['cases']), 'schema_valid': len(latencies), 'invalid_or_transport_error': len(result['cases']) - len(latencies), 'valid_result_p50_seconds': percentile(latencies, .5), 'valid_result_p95_seconds': percentile(latencies, .95), 'percentile_method': 'nearest rank; small heterogeneous single-pass corpus, not production SLO evidence', 'semantic_error_count': None, 'semantic_correct_count': None, 'abstention_count': sum(r.get('empty_candidates_abstention', False) for r in result['cases'])}
        result['input_hashes_unchanged'] = all(sha(p) == hashes[str(p.relative_to(BASE))] for p in source_files)
        result['wall_seconds'] = time.monotonic() - started
        out.write_text(json.dumps(result, ensure_ascii=False, indent=2) + '\n')
        (MODEL_DIR / 'ready.json').unlink(missing_ok=True)
        print(json.dumps(result['summary']), flush=True)

if __name__ == '__main__':
    main()
