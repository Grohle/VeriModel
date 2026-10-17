//! Model catalogue, on-disk layout, status and download.
//!
//! Models are NOT bundled with the installer. The user downloads one from the
//! Assistant panel (an explicit action, shown with size and license); it is
//! stored under `<app data>/assistant/<model id>/` and verified by SHA-256.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{AssistantError, AssistantResult};

/// A downloadable model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSpec {
    /// Folder name, e.g. "qwen2.5-0.5b-instruct-q4_k_m".
    pub id: String,
    pub name: String,
    /// "qwen2" | "llama" | "smollm" — selects the Candle model family and the chat template.
    pub family: String,
    pub gguf_url: String,
    pub gguf_sha256: String,
    pub gguf_size_bytes: u64,
    pub tokenizer_url: String,
    pub tokenizer_sha256: String,
    pub license: String,
    pub approx_ram_mb: u32,
}

/// Default model: Qwen2.5-0.5B-Instruct, Q4_K_M (~470 MB, Apache-2.0).
pub static DEFAULT_MODEL: std::sync::LazyLock<ModelSpec> = std::sync::LazyLock::new(|| {
    ModelSpec {
    id: "qwen2.5-0.5b-instruct-q4_k_m".into(),
    name: "Qwen2.5 0.5B Instruct (Q4_K_M)".into(),
    family: "qwen2".into(),
    gguf_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct-GGUF/resolve/main/qwen2.5-0.5b-instruct-q4_k_m.gguf".into(),
    gguf_sha256: "74a4da8c9fdbcd15bd1f6d01d621410d31c6fc00986f5eb687824e7b93d7a9db".into(),
    gguf_size_bytes: 491_400_032,
    tokenizer_url: "https://huggingface.co/Qwen/Qwen2.5-0.5B-Instruct/resolve/main/tokenizer.json".into(),
    tokenizer_sha256: "c0382117ea329cdf097041132f6d735924b697924d6f6fc3945713e96ce87539".into(),
    license: "Apache-2.0".into(),
    approx_ram_mb: 1400,
}
});

/// All models the UI may offer.
pub fn catalogue() -> Vec<ModelSpec> {
    vec![DEFAULT_MODEL.clone()]
}

/// Install state of a model.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct ModelStatus {
    pub model_id: String,
    pub name: String,
    pub installed: bool,
    pub model_dir: PathBuf,
    pub gguf_path: PathBuf,
    pub tokenizer_path: PathBuf,
    pub gguf_size_bytes: u64,
    pub license: String,
    pub approx_ram_mb: u32,
    /// Which inference backend this build has ("candle" or "none").
    pub backend: String,
}

/// `<app data>/assistant/<model id>`.
pub fn model_dir(app_data: &Path, spec: &ModelSpec) -> PathBuf {
    app_data.join("assistant").join(&spec.id)
}

/// Status of the default model.
pub fn status(app_data: &Path) -> ModelStatus {
    status_of(app_data, &DEFAULT_MODEL)
}

pub fn status_of(app_data: &Path, spec: &ModelSpec) -> ModelStatus {
    let dir = model_dir(app_data, spec);
    let gguf = dir.join("model.gguf");
    let tok = dir.join("tokenizer.json");
    ModelStatus {
        model_id: spec.id.clone(),
        name: spec.name.clone(),
        installed: gguf.is_file() && tok.is_file(),
        model_dir: dir,
        gguf_path: gguf,
        tokenizer_path: tok,
        gguf_size_bytes: spec.gguf_size_bytes,
        license: spec.license.clone(),
        approx_ram_mb: spec.approx_ram_mb,
        backend: if cfg!(feature = "candle") {
            "candle".into()
        } else {
            "none".into()
        },
    }
}

/// Download progress callback: (bytes_done, bytes_total_if_known).
pub type Progress<'a> = &'a mut dyn FnMut(u64, Option<u64>);

/// Progress is reported roughly every this many bytes.
const PROGRESS_STEP: u64 = 1 << 20;

/// Download `spec` into `app_data`, verify SHA-256 (when the spec declares one),
/// and return the status.
///
/// Each file is streamed to `<name>.part` and renamed only after the checksum
/// matches, so a failed or interrupted download never looks installed. The
/// tokenizer (small) is fetched first, then the GGUF weights. Progress is
/// cumulative over both files. A file already in place with the right
/// checksum is not downloaded again.
pub fn download(
    app_data: &Path,
    spec: &ModelSpec,
    progress: Progress<'_>,
) -> AssistantResult<ModelStatus> {
    let dir = model_dir(app_data, spec);
    std::fs::create_dir_all(&dir)
        .map_err(|e| AssistantError::Download(format!("cannot create {}: {e}", dir.display())))?;
    let files = [
        (
            &spec.tokenizer_url,
            &spec.tokenizer_sha256,
            "tokenizer.json",
        ),
        (&spec.gguf_url, &spec.gguf_sha256, "model.gguf"),
    ];
    let mut done: u64 = 0;
    let mut total: Option<u64> = if spec.gguf_size_bytes > 0 {
        Some(spec.gguf_size_bytes)
    } else {
        None
    };
    for (i, (url, sha, name)) in files.iter().enumerate() {
        let target = dir.join(name);
        if target.is_file()
            && !sha.is_empty()
            && sha256_file(&target)
                .map(|h| h.eq_ignore_ascii_case(sha))
                .unwrap_or(false)
        {
            let len = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
            if i == 0 {
                total = total.map(|t| t + len);
            }
            done += len;
            progress(done, total);
            continue;
        }
        let resp = agent_for(url)
            .get(url)
            .call()
            .map_err(|e| AssistantError::Download(format!("{url}: {e}")))?;
        let len: Option<u64> = resp.header("Content-Length").and_then(|v| v.parse().ok());
        if i == 0 {
            // tokenizer first: total = tokenizer + declared gguf size
            total = match (total, len) {
                (Some(t), Some(l)) => Some(t + l),
                _ => None,
            };
        } else if let (Some(l), None) = (len, total) {
            total = Some(done + l);
        }
        let part = dir.join(format!("{name}.part"));
        let result = stream_to_file(resp.into_reader(), &part, &mut done, total, progress)
            .and_then(|hash| {
                if !sha.is_empty() && !hash.eq_ignore_ascii_case(sha) {
                    return Err(AssistantError::Checksum(format!(
                        "{name}: expected {sha}, got {hash}"
                    )));
                }
                std::fs::rename(&part, &target).map_err(|e| {
                    AssistantError::Download(format!("cannot move {name} into place: {e}"))
                })
            });
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    }
    progress(done, Some(done));
    Ok(status_of(app_data, spec))
}

/// HTTP agent; honours HTTPS_PROXY / HTTP_PROXY / ALL_PROXY except for loopback URLs.
fn agent_for(url: &str) -> ureq::Agent {
    let loopback = ["://127.0.0.1", "://localhost", "://[::1]"]
        .iter()
        .any(|h| url.contains(h));
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(30))
        .timeout_read(std::time::Duration::from_secs(120))
        .try_proxy_from_env(!loopback)
        .build()
}

/// Copy `reader` into `path`, reporting progress, and return the SHA-256 (hex).
fn stream_to_file(
    mut reader: impl Read,
    path: &Path,
    done: &mut u64,
    total: Option<u64>,
    progress: Progress<'_>,
) -> AssistantResult<String> {
    let mut out = std::fs::File::create(path)
        .map_err(|e| AssistantError::Download(format!("cannot create {}: {e}", path.display())))?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut last_report = *done;
    loop {
        let n = match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => {
                return Err(AssistantError::Download(format!(
                    "connection interrupted: {e}"
                )))
            }
        };
        out.write_all(&buf[..n]).map_err(|e| {
            AssistantError::Download(format!("cannot write {}: {e}", path.display()))
        })?;
        hasher.update(&buf[..n]);
        *done += n as u64;
        if *done - last_report >= PROGRESS_STEP {
            last_report = *done;
            progress(*done, total);
        }
    }
    out.flush()
        .and_then(|_| out.sync_all())
        .map_err(|e| AssistantError::Download(format!("cannot write {}: {e}", path.display())))?;
    Ok(hex::encode(hasher.finalize()))
}

/// SHA-256 of a file (hex, lower case).
pub fn sha256_file(path: &Path) -> AssistantResult<String> {
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex::encode(hasher.finalize()))
}

/// Install a model from local files (for users who downloaded the files
/// manually, and for tests). Each file is copied to `<name>.part`, verified
/// against the spec's SHA-256 (when declared) and renamed into place.
pub fn install_from_files(
    app_data: &Path,
    spec: &ModelSpec,
    gguf_path: &Path,
    tokenizer_path: &Path,
) -> AssistantResult<ModelStatus> {
    let dir = model_dir(app_data, spec);
    std::fs::create_dir_all(&dir)?;
    for (src, sha, name) in [
        (tokenizer_path, &spec.tokenizer_sha256, "tokenizer.json"),
        (gguf_path, &spec.gguf_sha256, "model.gguf"),
    ] {
        if !src.is_file() {
            return Err(AssistantError::Other(format!(
                "{} does not exist",
                src.display()
            )));
        }
        let part = dir.join(format!("{name}.part"));
        let result = std::fs::copy(src, &part)
            .map_err(|e| AssistantError::Other(format!("cannot copy {}: {e}", src.display())))
            .and_then(|_| sha256_file(&part))
            .and_then(|hash| {
                if !sha.is_empty() && !hash.eq_ignore_ascii_case(sha) {
                    return Err(AssistantError::Checksum(format!(
                        "{}: expected {sha}, got {hash}",
                        src.display()
                    )));
                }
                std::fs::rename(&part, dir.join(name)).map_err(AssistantError::from)
            });
        if let Err(e) = result {
            let _ = std::fs::remove_file(&part);
            return Err(e);
        }
    }
    Ok(status_of(app_data, spec))
}

/// Remove an installed model.
pub fn remove(app_data: &Path, spec: &ModelSpec) -> AssistantResult<()> {
    let dir = model_dir(app_data, spec);
    if dir.is_dir() {
        std::fs::remove_dir_all(&dir)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_spec(gguf_sha: &str, tok_sha: &str) -> ModelSpec {
        ModelSpec {
            id: "test-model".into(),
            gguf_sha256: gguf_sha.into(),
            tokenizer_sha256: tok_sha.into(),
            gguf_size_bytes: 0,
            ..DEFAULT_MODEL.clone()
        }
    }

    fn sha(bytes: &[u8]) -> String {
        hex::encode(Sha256::digest(bytes))
    }

    #[test]
    fn default_model_declares_checksums() {
        assert_eq!(DEFAULT_MODEL.gguf_sha256.len(), 64);
        assert_eq!(DEFAULT_MODEL.tokenizer_sha256.len(), 64);
    }

    #[test]
    fn status_and_install_from_files() {
        let app = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let g = src.path().join("x.gguf");
        let t = src.path().join("tok.json");
        std::fs::write(&g, b"weights").unwrap();
        std::fs::write(&t, b"{\"tok\":1}").unwrap();
        let spec = test_spec(&sha(b"weights"), &sha(b"{\"tok\":1}"));

        let before = status_of(app.path(), &spec);
        assert!(!before.installed);
        assert_eq!(
            before.model_dir,
            app.path().join("assistant").join("test-model")
        );

        let after = install_from_files(app.path(), &spec, &g, &t).unwrap();
        assert!(after.installed);
        assert_eq!(std::fs::read(&after.gguf_path).unwrap(), b"weights");
        assert!(!after.model_dir.join("model.gguf.part").exists());
        assert_eq!(status_of(app.path(), &spec), after);

        remove(app.path(), &spec).unwrap();
        assert!(!status_of(app.path(), &spec).installed);
    }

    #[test]
    fn install_rejects_checksum_mismatch_and_leaves_nothing() {
        let app = tempfile::tempdir().unwrap();
        let src = tempfile::tempdir().unwrap();
        let g = src.path().join("x.gguf");
        let t = src.path().join("tok.json");
        std::fs::write(&g, b"weights").unwrap();
        std::fs::write(&t, b"tok").unwrap();
        let spec = test_spec(&"0".repeat(64), "");
        let err = install_from_files(app.path(), &spec, &g, &t).unwrap_err();
        assert!(matches!(err, AssistantError::Checksum(_)), "{err:?}");
        let st = status_of(app.path(), &spec);
        assert!(!st.installed);
        assert!(!st.gguf_path.exists());
        assert!(!st.model_dir.join("model.gguf.part").exists());
    }

    /// Serve fixed bodies over plain HTTP on localhost (one request per connection).
    fn serve(bodies: Vec<(&'static str, Vec<u8>)>) -> String {
        use std::io::BufRead;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { break };
                let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                loop {
                    let mut h = String::new();
                    if reader.read_line(&mut h).unwrap_or(0) == 0 || h == "\r\n" {
                        break;
                    }
                }
                match bodies.iter().find(|(p, _)| *p == path) {
                    Some((_, body)) => {
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = stream.write_all(body);
                    }
                    None => {
                        let _ = write!(stream, "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
            }
        });
        format!("http://{addr}")
    }

    #[test]
    fn download_streams_verifies_and_installs() {
        let gguf: Vec<u8> = (0..3_000_000u32).map(|i| (i % 251) as u8).collect();
        let tok = b"{\"model\":{}}".to_vec();
        let base = serve(vec![("/m.gguf", gguf.clone()), ("/t.json", tok.clone())]);
        let spec = ModelSpec {
            gguf_url: format!("{base}/m.gguf"),
            tokenizer_url: format!("{base}/t.json"),
            gguf_size_bytes: gguf.len() as u64,
            ..test_spec(&sha(&gguf), &sha(&tok))
        };
        let app = tempfile::tempdir().unwrap();
        let mut calls = Vec::new();
        let st = download(app.path(), &spec, &mut |d, t| calls.push((d, t))).unwrap();
        assert!(st.installed);
        assert_eq!(std::fs::read(&st.gguf_path).unwrap(), gguf);
        assert!(calls.len() >= 3, "{calls:?}");
        let full = (gguf.len() + tok.len()) as u64;
        assert_eq!(*calls.last().unwrap(), (full, Some(full)));
        assert!(calls.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn download_checksum_mismatch_is_not_installed() {
        let base = serve(vec![
            ("/m.gguf", b"bad weights".to_vec()),
            ("/t.json", b"tok".to_vec()),
        ]);
        let spec = ModelSpec {
            gguf_url: format!("{base}/m.gguf"),
            tokenizer_url: format!("{base}/t.json"),
            ..test_spec(&"a".repeat(64), "")
        };
        let app = tempfile::tempdir().unwrap();
        let err = download(app.path(), &spec, &mut |_, _| {}).unwrap_err();
        assert!(matches!(err, AssistantError::Checksum(_)), "{err:?}");
        let st = status_of(app.path(), &spec);
        assert!(!st.installed);
        assert!(!st.model_dir.join("model.gguf.part").exists());
        assert!(!st.gguf_path.exists());
    }

    #[test]
    fn download_http_error_is_download_error() {
        let base = serve(vec![]);
        let spec = ModelSpec {
            gguf_url: format!("{base}/missing.gguf"),
            tokenizer_url: format!("{base}/missing.json"),
            ..test_spec("", "")
        };
        let app = tempfile::tempdir().unwrap();
        let err = download(app.path(), &spec, &mut |_, _| {}).unwrap_err();
        assert!(matches!(err, AssistantError::Download(_)), "{err:?}");
        assert!(!status_of(app.path(), &spec).installed);
    }
}
