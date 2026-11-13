//! Releases as GitHub has them: tarballs, and a local server answering as the releases API.

use super::bin;
use heavy_cmd::install::asset;
use heavy_cmd::serde_json;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

/// A binary that only says it is `tag`.
pub fn fake(tag: &str) -> String {
    format!("#!/bin/sh\necho \"heavy-cmd {tag}\"\n")
}

/// A release tarball in `dir`, `heavy-cmd/bin/heavy-cmd` in it: this build, or `script` when given.
pub fn release(dir: &Path, script: Option<&str>) -> PathBuf {
    static COUNT: AtomicU32 = AtomicU32::new(0);
    let n = COUNT.fetch_add(1, Ordering::SeqCst);
    let stage = dir.join(format!("stage-{n}"));
    let bin_dir = stage.join("heavy-cmd").join("bin");
    std::fs::create_dir_all(&bin_dir).unwrap();
    let file = bin_dir.join("heavy-cmd");
    match script {
        Some(script) => std::fs::write(&file, script).unwrap(),
        None => drop(std::fs::copy(bin(), &file).unwrap()),
    }
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
    let out = dir.join(format!("release-{n}.tar.gz"));
    let packed = Command::new("tar").arg("-czf").arg(&out).arg("-C").arg(&stage).arg("heavy-cmd").status().unwrap();
    assert!(packed.success());
    std::fs::remove_dir_all(&stage).unwrap();
    out
}

/// GitHub's releases as a local server: `/latest` answers as the API's `releases/latest` for the release
/// set newest, `/no-asset` with a release with no asset for this build, and the asset is served from `/download/`.
pub struct Releases {
    pub base: String,
    newest: Arc<Mutex<Option<(String, PathBuf)>>>,
}

impl Releases {
    pub fn serve() -> Releases {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let newest = Arc::new(Mutex::new(None));
        let (shared, at) = (newest.clone(), base.clone());
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                answer(stream, &shared, &at);
            }
        });
        Releases { base, newest }
    }

    pub fn api(&self) -> String {
        format!("{}/latest", self.base)
    }

    /// The newest release is now `tag`, its asset `tarball`.
    pub fn set(&self, tag: &str, tarball: PathBuf) {
        *self.newest.lock().unwrap() = Some((tag.to_string(), tarball));
    }
}

fn answer(stream: TcpStream, newest: &Mutex<Option<(String, PathBuf)>>, base: &str) {
    let mut reader = BufReader::new(&stream);
    let mut request = String::new();
    let _ = reader.read_line(&mut request);
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
            break;
        }
    }
    let path = request.split_whitespace().nth(1).unwrap_or("").to_string();
    let newest = newest.lock().unwrap().clone();
    let name = asset();
    let (status, body) = match &newest {
        Some((tag, _)) if path == "/latest" => {
            let url = format!("{base}/download/{tag}/{name}");
            let json = serde_json::json!({ "tag_name": tag, "assets": [{ "name": name, "browser_download_url": url }] });
            ("200 OK", serde_json::to_vec_pretty(&json).unwrap())
        }
        Some((tag, tarball)) if path == format!("/download/{tag}/{name}") => ("200 OK", std::fs::read(tarball).unwrap()),
        _ if path == "/no-asset" => ("200 OK", br#"{"tag_name": "v0.2.0", "assets": []}"#.to_vec()),
        _ => ("404 Not Found", Vec::new()),
    };
    let mut stream = &stream;
    let _ = write!(stream, "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let _ = stream.write_all(&body);
}
