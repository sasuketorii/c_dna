#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
use std::{
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::Duration,
};
use tauri::{Manager, RunEvent, WebviewUrl, WebviewWindowBuilder};

struct Backend {
    child: Mutex<Option<Child>>,
    stopping: Arc<AtomicBool>,
}
impl Backend {
    fn stop(&self) {
        self.stopping.store(true, Ordering::Release);
        if let Ok(mut guard) = self.child.lock() {
            if let Some(mut child) = guard.take() {
                #[cfg(unix)]
                // The process group was created for this owned, unreaped child only.
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGINT);
                }
                // Do not reap before killing the owned process group: this prevents
                // PID reuse from turning cleanup into a signal to another process.
                #[cfg(unix)]
                std::thread::sleep(Duration::from_millis(250));
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.stop();
    }
}

fn ready_url(line: &[u8]) -> Option<tauri::Url> {
    let text = std::str::from_utf8(line).ok()?.trim();
    let url: tauri::Url = text.strip_prefix("C-DNA: ")?.parse().ok()?;
    let token = url.fragment()?.strip_prefix("session=")?;
    if url.scheme() != "http"
        || url.host_str() != Some("127.0.0.1")
        || url.port().is_none()
        || url.path() != "/"
        || url.query().is_some()
        || !url.username().is_empty()
        || url.password().is_some()
        || token.len() != 64
        || !token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(url)
}

// Hold the lock for the entire native session. No stale-lock deletion or vault reset.
#[cfg(unix)]
fn session_lock(directory: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::{fs::OpenOptionsExt, io::AsRawFd};
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(directory.join("native.lock"))?;
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(file)
}

fn initialize_if_missing(
    executable: &Path,
    vault: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    match std::fs::symlink_metadata(vault) {
        Ok(_) => return Ok(()), // Existing, partial and symlink vaults are NEVER replaced.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.into()),
    }
    let mut command = Command::new(executable);
    command
        .arg("--vault")
        .arg(vault)
        .arg("init")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let child = command.spawn()?;
    let backend = Backend {
        child: Mutex::new(Some(child)),
        stopping: Arc::new(AtomicBool::new(false)),
    };
    let started = std::time::Instant::now();
    loop {
        {
            let mut guard = backend
                .child
                .lock()
                .map_err(|_| "initialization lock failed")?;
            if let Some(status) = guard
                .as_mut()
                .ok_or("initialization process missing")?
                .try_wait()?
            {
                // Already reaped: Drop must not signal this pid/group again.
                guard.take();
                return if status.success() {
                    Ok(())
                } else {
                    Err("vault initialization failed; existing files retained".into())
                };
            }
        }
        if started.elapsed() > Duration::from_secs(120) {
            return Err("vault initialization timed out; existing files retained".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn main() {
    let app = tauri::Builder::default()
        .setup(|app| {
            let executable = std::env::current_exe()?
                .parent()
                .ok_or("missing application directory")?
                .join(if cfg!(windows) { "cdna.exe" } else { "cdna" });
            let assets = app.path().resource_dir()?.join("ui");
            let dev_assets = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../dist");
            let assets = if assets.join("index.html").is_file() {
                assets
            } else if cfg!(debug_assertions) {
                dev_assets
            } else {
                return Err("bundled UI assets missing".into());
            };
            let learner = app.path().resource_dir()?.join("learner");
            if !learner.join("python/bin/python3").is_file() {
                return Err("bundled Python learner missing; run tauri:prepare".into());
            }
            // Demo is always opt-in; normal launch never creates disposable user data.
            let demo = std::env::args_os().any(|arg| arg == "--demo");
            let mut command = Command::new(&executable);
            command.arg("--learner-dir").arg(&learner);
            if !demo {
                let directory = app.path().app_data_dir()?;
                let mut builder = std::fs::DirBuilder::new();
                builder.recursive(true);
                #[cfg(unix)]
                {
                    use std::os::unix::fs::DirBuilderExt;
                    builder.mode(0o700);
                }
                builder.create(&directory)?;
                #[cfg(unix)]
                app.manage(session_lock(&directory)?);
                let vault = directory.join("vault");
                initialize_if_missing(&executable, &vault)?;
                command.arg("--vault").arg(vault);
            }
            command.arg("playground");
            if demo {
                command.arg("--demo");
            }
            command
                .args(["--port", "0", "--assets-dir"])
                .arg(&assets)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::piped());
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                command.process_group(0);
            }
            let mut child = command.spawn()?;
            let stderr = child.stderr.take().ok_or("sidecar stderr missing")?;
            let stopping = Arc::new(AtomicBool::new(false));
            let backend = Backend {
                child: Mutex::new(Some(child)),
                stopping: stopping.clone(),
            };
            let (send, recv) = mpsc::sync_channel(1);
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                let mut reader = BufReader::new(stderr);
                let mut sent = false;
                loop {
                    let mut line = Vec::new();
                    match (&mut reader).take(4097).read_until(b'\n', &mut line) {
                        Ok(0) | Err(_) => break,
                        Ok(_) if line.len() > 4096 => break,
                        Ok(_) => {
                            if !sent {
                                if let Some(url) = ready_url(&line) {
                                    if send.send(url).is_err() {
                                        break;
                                    }
                                    sent = true;
                                }
                            }
                        }
                    }
                }
                if sent && !stopping.load(Ordering::Acquire) {
                    handle.exit(1);
                }
            });
            let url = recv
                .recv_timeout(Duration::from_secs(120))
                .map_err(|_| "local backend did not become ready")?;
            let origin = url.origin();
            WebviewWindowBuilder::new(app, "main", WebviewUrl::External(url))
                .title(if demo {
                    "C-DNA — 開発用デモ"
                } else {
                    "C-DNA"
                })
                .inner_size(1200.0, 820.0)
                .min_inner_size(760.0, 600.0)
                .devtools(cfg!(debug_assertions))
                .on_navigation(move |next| next.origin() == origin)
                .build()?;
            app.manage(backend);
            #[cfg(debug_assertions)]
            if std::env::var_os("CDNA_DESKTOP_SMOKE").as_deref() == Some(std::ffi::OsStr::new("1"))
            {
                let handle = app.handle().clone();
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(1));
                    eprintln!(
                        "C-DNA desktop smoke: native window and owned loopback backend ready"
                    );
                    handle.exit(0);
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("C-DNA desktop startup failed");
    app.run(|handle, event| {
        if matches!(event, RunEvent::Exit | RunEvent::ExitRequested { .. }) {
            if let Some(backend) = handle.try_state::<Backend>() {
                backend.stop();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn readiness_is_bound_to_loopback_and_opaque_token() {
        let token = "a".repeat(64);
        assert!(
            ready_url(format!("C-DNA: http://127.0.0.1:4317/#session={token}").as_bytes())
                .is_some()
        );
        for url in [
            format!("https://example.org/#session={token}"),
            format!("http://localhost:4317/#session={token}"),
            format!("http://127.0.0.1:4317/?remote=1#session={token}"),
            "http://127.0.0.1:4317/#session=short".into(),
        ] {
            assert!(ready_url(format!("C-DNA: {url}").as_bytes()).is_none());
        }
    }
}

#[cfg(all(test, unix))]
mod lifecycle_tests {
    use super::*;
    use std::os::unix::process::CommandExt;
    #[test]
    fn owned_backend_is_reaped_and_stop_is_idempotent() {
        let mut command = Command::new("/bin/sleep");
        command.arg("30").process_group(0);
        let child = command.spawn().unwrap();
        let pid = child.id();
        let backend = Backend {
            child: Mutex::new(Some(child)),
            stopping: Arc::new(AtomicBool::new(false)),
        };
        backend.stop();
        backend.stop();
        assert!(backend.child.lock().unwrap().is_none());
        assert!(backend.stopping.load(Ordering::Acquire));
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
    }
}

#[cfg(all(test, unix))]
mod onboarding_tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    #[test]
    fn initialize_once_preserves_existing_and_partial_vaults() {
        let directory = std::env::temp_dir().join(format!(
            "cdna-native-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&directory).unwrap();
        let executable = directory.join("init");
        std::fs::write(
            &executable,
            "#!/bin/sh\n/bin/mkdir \"$2\" && /usr/bin/touch \"$2/created\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o700)).unwrap();
        let vault = directory.join("vault");
        initialize_if_missing(&executable, &vault).unwrap();
        assert!(vault.join("created").exists());
        std::fs::write(vault.join("created"), "preserve").unwrap();
        initialize_if_missing(Path::new("/does-not-exist"), &vault).unwrap();
        assert_eq!(
            std::fs::read_to_string(vault.join("created")).unwrap(),
            "preserve"
        );
        let partial = directory.join("partial");
        std::fs::create_dir(&partial).unwrap();
        initialize_if_missing(Path::new("/does-not-exist"), &partial).unwrap();
        let dangling = directory.join("dangling");
        symlink(directory.join("absent"), &dangling).unwrap();
        initialize_if_missing(Path::new("/does-not-exist"), &dangling).unwrap();
        let lock = session_lock(&directory).unwrap();
        assert!(session_lock(&directory).is_err());
        drop(lock);
        assert!(session_lock(&directory).is_ok());
        std::fs::remove_dir_all(directory).unwrap();
    }
}
