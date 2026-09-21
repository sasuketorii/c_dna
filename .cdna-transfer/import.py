"""One-time, checksum-verified transfer of source generated in the authorized workspace."""
import base64
import hashlib
import json
import lzma
from pathlib import Path, PurePosixPath

root = Path.cwd().resolve()
parts = sorted((root / '.cdna-transfer').glob('*.b64'))
assert [p.name for p in parts] == [f'{i:02}.b64' for i in range(11)]
encoded = ''.join(p.read_text().strip() for p in parts)
blob = base64.b64decode(encoded, validate=True)
assert hashlib.sha256(blob).hexdigest() == '57dbefaa28a687068f9bff891322d1cc1ce1f2218a5c34ddd1c51d77516cea8b', 'Transfer checksum mismatch'
dec = lzma.LZMADecompressor(memlimit=256*1024*1024)
raw = dec.decompress(blob, max_length=1024*1024)
assert dec.eof and not dec.unused_data and len(raw) < 1024*1024

def unique(pairs):
    obj = {}
    for k, v in pairs:
        assert k not in obj, 'Duplicate path'
        obj[k] = v
    return obj

files = json.loads(raw, object_pairs_hook=unique)
assert isinstance(files, dict) and len(files) == 46
for name, text in files.items():
    p = PurePosixPath(name)
    assert not p.is_absolute() and '..' not in p.parts and '\\' not in name
    assert p.parts[0] in {'Cargo.toml', 'rust-toolchain.toml', 'crates', 'contracts', 'learner', 'apps'}
    dest = root / p
    assert root in dest.resolve().parents
    assert not any(parent.is_symlink() for parent in [dest, *dest.parents])
    assert isinstance(text, str) and len(text.encode()) < 256*1024
for name, text in files.items():
    dest = root / name
    dest.parent.mkdir(parents=True, exist_ok=True)
    dest.write_text(text, encoding='utf-8')
print(f'Validated and materialized {len(files)} source files; no private dataset included.')
