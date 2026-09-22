# Native engine host

The Tauri shell reuses the Rust CLI and learner. Normal launch uses Tauri's
application-data directory for `vault`, creates it once with `cdna init`, and
opens it with `cdna playground`. The existing macOS Keychain implementation owns
the encryption key. An OS session lock prevents two native instances initializing
or opening the same app vault. Existing directories, partial initialization and
symlinks are never reset or deleted. Initialization failure retains existing files
for diagnosis; it does not silently create a replacement vault.

`--demo` explicitly selects the existing disposable test vault. It is suitable for
development and bundle checks and never opens the persistent app-data vault.

## Build and verification

Use the development harness guard for heavy commands. Put `$HOME/.cargo/bin`,
`/usr/bin` and `/bin` before Homebrew in the command-local PATH; the host has an
`rm` wrapper that breaks native dependency builds.

```sh
pnpm --dir apps/desktop build
cargo build -p cdna-app --release -j 2
node apps/desktop/src-tauri/scripts/prepare.mjs
pnpm --dir apps/desktop tauri:build
cargo test -p cdna-desktop-shell -j 2
python3 apps/desktop/src-tauri/scripts/bundle-smoke.py
```

The preparer copies only the frontend, built CLI, first-party learner source and
locked production dependencies. It does not copy the development venv. It reuses
uv-managed CPython **3.14.7** or downloads that exact python-build-standalone
runtime into temporary storage with `--no-bin --no-registry`. No global Python
installation is changed. `uv export --frozen --no-dev --no-emit-project` and
`uv pip install --require-hashes --only-binary :all: --link-mode copy` install the
lockfile's wheels, retaining their licenses. The source package is installed
without editable metadata or a repository-path `.pth` file.

LightGBM's macOS wheel expects a host OpenMP library. The preparer copies the
already installed Homebrew **libomp 23.1.1** and its LLVM license, changes lookup
to `@loader_path`, and ad-hoc signs those modified dylibs. It fails if the version
differs. It does not install or upgrade Homebrew packages. Distribution metadata
records the Python build, package versions and lockfile hash; `uv.lock` and the
first-party license are included. This recipe currently targets macOS arm64.

The smoke copies the built app to a fresh temporary location and runs its own
backend with a disposable vault. macOS sandbox policy denies repository,
Homebrew, and user uv/mise runtime reads. It verifies authenticated UI/API access,
403 without the session token, then propose → confirm → train → provisional
ranking with an entirely synthetic fixture. Imports include LightGBM. This is
engine/distribution evidence, not evidence of personal prediction accuracy.

## Security and lifecycle

The backend binds literal loopback on an ephemeral port and announces a 64-hex
session fragment. The shell validates scheme, host, port, path and fragment;
rejects query/userinfo; and restricts navigation to the resulting origin. No
Tauri shell, filesystem, remote or opener capability is exposed to the page.
The learner runs via the fixed bundled interpreter with Python isolation flags.

Initialization and backend readiness have 120-second deadlines to accommodate
Keychain interaction. On exit, the owned backend process group receives SIGINT,
then SIGKILL after 250 ms, and the original child is reaped. Initialization also
uses owned-process cleanup on timeout. The native session lock is released by
file-descriptor close, so stale lock files do not block later launches.

Unit tests cover readiness validation, process cleanup, first initialization,
existing/partial/dangling-symlink preservation and exclusive session locking.
Actual persistent Keychain onboarding and cancellation in a clean user account
remain separate host acceptance: checks must never operate on a current user's
vault. No Developer ID signing, notarization, installer or clean-machine
acceptance is claimed. Runtime version pins must be reviewed before release.

References:

- [Tauri resources](https://v2.tauri.app/develop/resources/)
- [Tauri sidecars](https://v2.tauri.app/develop/sidecar/)
- [uv managed Python](https://docs.astral.sh/uv/concepts/python-versions/)

The bundled shell page avoids embedding the React resources twice. The icon is
an unchanged development placeholder from the official create-tauri-app template;
its original MIT license is retained in `icons/LICENSE_MIT`.
