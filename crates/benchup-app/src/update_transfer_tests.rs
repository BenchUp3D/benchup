//! Use the unchanged Velopack client shipped in 0.6.5, without BenchUp-specific
//! update preparation, to exercise transfer redirects and the renamed package.
use super::*;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use velopack::{
    bundle,
    locator::{VelopackLocator, VelopackLocatorConfig},
};

struct Server {
    base: String,
    requests: Arc<Mutex<Vec<String>>>,
    stopped: Arc<AtomicBool>,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Server {
    fn new(channel: &str) -> Self {
        let fixtures =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/update-transfer");
        let feed_name = format!("releases.{channel}.json");
        let feed = std::fs::read(fixtures.join(&feed_name)).unwrap();
        let asset: serde_json::Value = serde_json::from_slice(&feed).unwrap();
        let package_name = asset["Assets"][0]["FileName"].as_str().unwrap().to_owned();
        let package = std::fs::read(fixtures.join(&package_name)).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server_base = base.clone();
        let requests = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&requests);
        let stopped = Arc::new(AtomicBool::new(false));
        let stop = Arc::clone(&stopped);
        let worker = std::thread::spawn(move || {
            let assets = [&feed_name, &package_name].map(|name| {
                serde_json::json!({
                    "name": name,
                    "browser_download_url": format!("{server_base}/Ketchup3D/app/releases/download/v0.6.6/{name}")
                })
            });
            let releases = serde_json::to_vec(&serde_json::json!([{
                "name": "v0.6.6", "prerelease": false,
                "published_at": "2026-11-08T00:00:00Z",
                "assets": assets
            }])).unwrap();
            while !stop.load(Ordering::Relaxed) {
                let (mut stream, _) = match listener.accept() {
                    Ok(connection) => connection,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                        continue;
                    }
                    Err(error) => panic!("Mock updater server: {error}"),
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .unwrap();
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let size = match stream.read(&mut buffer) {
                        Ok(size) => size,
                        Err(_) => break,
                    };
                    if size == 0 {
                        break;
                    }
                    request.extend_from_slice(&buffer[..size]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n")
                        || request.len() >= 16384
                    {
                        break;
                    }
                }
                let request = String::from_utf8_lossy(&request);
                let path = request
                    .lines()
                    .next()
                    .and_then(|line| line.split_whitespace().nth(1))
                    .unwrap_or("");
                seen.lock().unwrap().push(path.to_owned());
                let mut location = None;
                let (status, body): (&str, &[u8]) = if let Some(tail) =
                    path.strip_prefix("/api/v3/repos/Ketchup3D/app/")
                {
                    location = Some(format!("{server_base}/api/v3/repos/BenchUp3D/app/{tail}"));
                    ("301 Moved Permanently", b"")
                } else if let Some(tail) = path.strip_prefix("/api/v3/repos/BenchUp3D/app/") {
                    location = Some(format!(
                        "{server_base}/api/v3/repos/BenchUp3D/editor/{tail}"
                    ));
                    ("301 Moved Permanently", b"")
                } else if path.starts_with("/api/v3/repos/BenchUp3D/editor/releases?") {
                    ("200 OK", &releases)
                } else if let Some(tail) = path.strip_prefix("/Ketchup3D/app/releases/download/") {
                    location = Some(format!(
                        "{server_base}/BenchUp3D/editor/releases/download/{tail}"
                    ));
                    ("302 Found", b"")
                } else if path == format!("/BenchUp3D/editor/releases/download/v0.6.6/{feed_name}")
                {
                    ("200 OK", &feed)
                } else if path
                    == format!("/BenchUp3D/editor/releases/download/v0.6.6/{package_name}")
                {
                    ("200 OK", &package)
                } else {
                    ("404 Not Found", b"Unknown route")
                };
                let redirect = location
                    .map(|url| format!("Location: {url}\r\n"))
                    .unwrap_or_default();
                let header = format!(
                    "HTTP/1.1 {status}\r\n{redirect}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream
                    .write_all(header.as_bytes())
                    .and_then(|()| stream.write_all(body));
            }
        });
        Self {
            base,
            requests,
            stopped,
            worker: Some(worker),
        }
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Relaxed);
        // Wake an idle listener; its nonblocking loop also bounds cleanup.
        let _ = TcpStream::connect(self.base.trim_start_matches("http://"));
        if let Some(worker) = self.worker.take() {
            worker.join().unwrap();
        }
    }
}

#[test]
fn shipped_065_updater_follows_transfer_and_rename_then_downloads_benchup_on_all_channels() {
    for channel in ["win-x64", "osx-arm64", "linux-x64"] {
        let server = Server::new(channel);
        let root = tempfile::tempdir().unwrap();
        let config = VelopackLocatorConfig {
            RootAppDir: root.path().into(),
            UpdateExePath: root.path().join("Update.exe"),
            PackagesDir: root.path().join("packages"),
            ManifestPath: root.path().join("sq.version"),
            CurrentBinaryDir: root.path().join("current"),
            IsPortable: true,
        };
        std::fs::create_dir_all(&config.CurrentBinaryDir).unwrap();
        std::fs::write(&config.UpdateExePath, b"inert old updater marker").unwrap();
        std::fs::write(&config.ManifestPath, format!(
            "<package><metadata><id>{}</id><version>0.6.5</version><title>KetchUp3D</title><channel>{channel}</channel><mainExe>ketchup-app.exe</mainExe></metadata></package>",
            crate::identity::LEGACY_APP_ID)).unwrap();
        let manager = UpdateManager::new(
            GithubSource::new(&format!("{}/Ketchup3D/app", server.base), None, false),
            None,
            Some(config.clone()),
        )
        .unwrap();
        let UpdateCheck::UpdateAvailable(update) = manager.check_for_updates().unwrap() else {
            panic!("0.6.5 must discover BenchUp through the transferred repository");
        };
        assert_eq!(update.TargetFullRelease.PackageId, crate::identity::APP_ID);
        assert!(update.BaseRelease.is_none());
        assert!(update.DeltasToTarget.is_empty());
        // Deliberately do NOT call prepare_update: the old app did not have it.
        manager.download_updates(&update, None).unwrap();
        let pending = manager.get_update_pending_restart().unwrap();
        assert_eq!(pending.PackageId, crate::identity::APP_ID);
        let file = config.PackagesDir.join(&pending.FileName);
        let manifest = bundle::load_bundle_from_file(&file)
            .unwrap()
            .read_manifest()
            .unwrap();
        let executable = if channel == "win-x64" {
            "benchup-app.exe"
        } else {
            "benchup-app"
        };
        assert_eq!(manifest.main_exe, executable);
        // The apply helper uses this same manifest handoff to restart the new exe.
        let locator = VelopackLocator::new(&config)
            .unwrap()
            .clone_self_with_new_manifest(&manifest);
        assert_eq!(
            locator.get_main_exe_path(),
            config.CurrentBinaryDir.join(executable)
        );
        // Once applied, the new installation checks the new repository directly.
        let new_manifest = format!(
            "<package><metadata><id>{}</id><version>0.6.6</version><channel>{channel}</channel><mainExe>{executable}</mainExe></metadata></package>",
            crate::identity::APP_ID
        );
        std::fs::write(&config.ManifestPath, new_manifest).unwrap();
        let new_manager = UpdateManager::new(
            GithubSource::new(&format!("{}/BenchUp3D/editor", server.base), None, false),
            None,
            Some(config),
        )
        .unwrap();
        assert_eq!(new_manager.get_app_id(), crate::identity::APP_ID);
        assert!(matches!(
            new_manager.check_for_updates().unwrap(),
            UpdateCheck::NoUpdateAvailable
        ));
        let seen = server.requests.lock().unwrap();
        assert!(
            seen.iter()
                .any(|path| path.starts_with("/api/v3/repos/Ketchup3D/app/releases?"))
        );
        assert!(
            seen.iter()
                .any(|path| path.starts_with("/api/v3/repos/BenchUp3D/app/releases?"))
        );
        assert!(
            seen.iter()
                .any(|path| path.starts_with("/Ketchup3D/app/releases/download/")
                    && path.ends_with(".nupkg"))
        );
        assert!(seen.iter().any(
            |path| path.starts_with("/BenchUp3D/editor/releases/download/")
                && path.ends_with(".nupkg")
        ));
    }
}
