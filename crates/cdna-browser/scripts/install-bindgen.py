"""Install the exact official wasm-bindgen CLI into this crate, never global PATH."""
import io
from pathlib import Path
import platform
import tarfile
import urllib.request

VERSION = '0.2.128'
crate = Path(__file__).resolve().parents[1]
arch = {'arm64': 'aarch64', 'aarch64': 'aarch64', 'x86_64': 'x86_64', 'AMD64': 'x86_64'}[platform.machine()]
system = {'Darwin': 'apple-darwin', 'Linux': 'unknown-linux-musl'}[platform.system()]
name = f'wasm-bindgen-{VERSION}-{arch}-{system}'
url = f'https://github.com/wasm-bindgen/wasm-bindgen/releases/download/{VERSION}/{name}.tar.gz'
with urllib.request.urlopen(url, timeout=90) as response:
    archive = response.read(64 * 1024 * 1024 + 1)
if len(archive) > 64 * 1024 * 1024:
    raise ValueError('release archive too large')
output = crate/'.tools'
output.mkdir(exist_ok=True)
with tarfile.open(fileobj=io.BytesIO(archive), mode='r:gz') as source:
    for member in source.getmembers():
        if Path(member.name).name == 'wasm-bindgen' and member.isfile():
            content = source.extractfile(member)
            if content is None:
                raise ValueError('missing binary')
            destination = output/'wasm-bindgen'
            destination.write_bytes(content.read(64 * 1024 * 1024))
            destination.chmod(0o755)
            break
    else:
        raise ValueError('binary not found')
print(f'Installed wasm-bindgen {VERSION} in crate-local .tools')
