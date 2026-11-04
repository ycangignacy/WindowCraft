// Made by ycangignacy
//! Streaming downloads over rustls with SHA-256 verification.

use crate::error::{EngineError, Result};
use crate::jobs::CancelToken;
use sha2::{Digest, Sha256};
use std::future::Future;
use std::io::{BufWriter, Read, Write};
use std::path::Path;
use std::sync::Once;
use std::time::Duration;

const USER_AGENT: &str = concat!("WindowCraft/", env!("CARGO_PKG_VERSION"));

fn install_crypto_provider() {
    static INSTALL: Once = Once::new();
    INSTALL.call_once(|| {
        // Only fails when a provider is already installed, which is fine.
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

pub fn http_client() -> Result<reqwest::Client> {
    install_crypto_provider();
    reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .connect_timeout(Duration::from_secs(15))
        .build()
        .map_err(|e| EngineError::Network(e.to_string()))
}

/// Runs async network work from a blocking job thread.
pub fn block_on<F: Future>(future: F) -> Result<F::Output> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| EngineError::io("tokio runtime", e))?;
    Ok(runtime.block_on(future))
}

pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file = std::fs::File::open(path).map_err(|e| EngineError::io(path.display().to_string(), e))?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 1 << 20];
    loop {
        let read = file.read(&mut buffer).map_err(|e| EngineError::io(path.display().to_string(), e))?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hex(&hasher.finalize()))
}

/// Downloads `url` to `destination`, reporting `(received, total)`.
///
/// Data lands in a `.part` file that only replaces the destination after the
/// checksum passes, so a failed or cancelled download leaves nothing usable.
pub async fn download(
    url: &str,
    destination: &Path,
    expected_sha256: Option<&str>,
    cancel: &CancelToken,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        std::fs::create_dir_all(parent).map_err(|e| EngineError::io(parent.display().to_string(), e))?;
    }
    let mut response = http_client()?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| EngineError::Network(e.to_string()))?;
    let total = response.content_length();

    let partial = destination.with_extension("part");
    let result = async {
        let file =
            std::fs::File::create(&partial).map_err(|e| EngineError::io(partial.display().to_string(), e))?;
        let mut writer = BufWriter::with_capacity(1 << 20, file);
        let mut hasher = Sha256::new();
        let mut received = 0u64;
        while let Some(chunk) = response.chunk().await.map_err(|e| EngineError::Network(e.to_string()))? {
            cancel.check()?;
            hasher.update(&chunk);
            writer.write_all(&chunk).map_err(|e| EngineError::io(partial.display().to_string(), e))?;
            received += chunk.len() as u64;
            progress(received, total);
        }
        let file = writer.into_inner().map_err(|e| EngineError::io(partial.display().to_string(), e.into_error()))?;
        file.sync_all().map_err(|e| EngineError::io(partial.display().to_string(), e))?;
        drop(file);

        if let Some(expected) = expected_sha256 {
            let actual = hex(&hasher.finalize());
            if !actual.eq_ignore_ascii_case(expected) {
                return Err(EngineError::Checksum {
                    file: destination.display().to_string(),
                    expected: expected.to_owned(),
                    actual,
                });
            }
        }
        std::fs::rename(&partial, destination).map_err(|e| EngineError::io(destination.display().to_string(), e))
    }
    .await;

    if result.is_err() {
        // The partial file is worthless; a leftover would only waste space.
        let _ = std::fs::remove_file(&partial);
    }
    result
}

pub async fn fetch_text(url: &str) -> Result<String> {
    http_client()?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| EngineError::Network(e.to_string()))?
        .text()
        .await
        .map_err(|e| EngineError::Network(e.to_string()))
}

/// Fetches a text file and checks its SHA-256 before returning it.
pub async fn fetch_text_pinned(url: &str, expected_sha256: &str) -> Result<String> {
    let bytes = http_client()?
        .get(url)
        .send()
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| EngineError::Network(e.to_string()))?
        .bytes()
        .await
        .map_err(|e| EngineError::Network(e.to_string()))?;
    let actual = hex(&Sha256::digest(&bytes));
    if !actual.eq_ignore_ascii_case(expected_sha256) {
        return Err(EngineError::Checksum { file: url.to_owned(), expected: expected_sha256.to_owned(), actual });
    }
    String::from_utf8(bytes.to_vec()).map_err(|_| EngineError::Invalid(format!("{url} is not UTF-8 text")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    async fn serve(body: &'static [u8]) -> String {
        serve_response("200 OK", body).await
    }

    async fn serve_response(status: &'static str, body: &'static [u8]) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).await.unwrap();
        let address = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0u8; 2048];
                let _ = stream.read(&mut request).await;
                let header = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(header.as_bytes()).await;
                let _ = stream.write_all(body).await;
            }
        });
        format!("http://{address}/file")
    }

    const SHA_OF_HELLO: &str = "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824";
    const SHA_OF_EMPTY: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

    #[test]
    fn hex_formats_lowercase_bytes_with_leading_zeros() {
        assert_eq!(hex(&[]), "");
        assert_eq!(hex(&[0x00, 0x0a, 0x10, 0xff]), "000a10ff");
    }

    #[test]
    fn hashes_a_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("f");
        std::fs::write(&path, b"hello").unwrap();
        assert_eq!(sha256_file(&path).unwrap(), SHA_OF_HELLO);
    }

    #[test]
    fn sha256_file_handles_empty_multi_chunk_and_missing_files() {
        let dir = tempfile::tempdir().unwrap();
        let empty = dir.path().join("empty.bin");
        std::fs::write(&empty, b"").unwrap();
        assert_eq!(sha256_file(&empty).unwrap(), SHA_OF_EMPTY);

        let large = dir.path().join("large.bin");
        let payload = vec![b'x'; (1 << 20) + 1024];
        std::fs::write(&large, &payload).unwrap();
        let expected = hex(&Sha256::digest(&payload));
        assert_eq!(sha256_file(&large).unwrap(), expected);

        let missing = dir.path().join("missing.bin");
        assert!(sha256_file(&missing).unwrap_err().is_not_found());
    }

    #[test]
    fn block_on_drives_futures_and_http_client_is_reusable() {
        assert!(http_client().is_ok());
        assert!(http_client().is_ok());
        let output = block_on(async { 21 + 21 }).unwrap();
        assert_eq!(output, 42);
    }

    /// Reaches the public internet, so it only runs with `--ignored`.
    #[tokio::test]
    #[ignore = "needs network access"]
    async fn tls_works_against_a_real_endpoint() {
        let license = fetch_text("https://raw.githubusercontent.com/ChrisTitusTech/winutil/main/LICENSE").await.unwrap();
        assert!(license.contains("MIT License"));
    }

    #[tokio::test]
    async fn downloads_and_verifies() {
        let url = serve(b"hello").await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let mut seen = 0;
        download(&url, &path, Some(SHA_OF_HELLO), &CancelToken::new(), |received, _| seen = received)
            .await
            .unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
        assert_eq!(seen, 5);
        assert!(!path.with_extension("part").exists());
    }

    #[tokio::test]
    async fn download_creates_parent_directories_and_accepts_uppercase_checksum() {
        let url = serve(b"hello").await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("deeper").join("out.bin");
        let upper_sha = SHA_OF_HELLO.to_ascii_uppercase();
        download(&url, &path, Some(&upper_sha), &CancelToken::new(), |_, _| {}).await.unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"hello");
    }

    #[tokio::test]
    async fn http_error_status_maps_to_network_error_in_download_and_fetch_text() {
        let not_found_url = serve_response("404 Not Found", b"missing").await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let err = download(&not_found_url, &path, None, &CancelToken::new(), |_, _| {}).await.unwrap_err();
        assert!(matches!(err, EngineError::Network(_)));
        assert!(!path.exists());
        assert!(!path.with_extension("part").exists());

        let fetch_err = fetch_text(&not_found_url).await.unwrap_err();
        assert!(matches!(fetch_err, EngineError::Network(_)));

        let ok_url = serve(b"manifest-body").await;
        assert_eq!(fetch_text(&ok_url).await.unwrap(), "manifest-body");
    }

    #[tokio::test]
    async fn checksum_failure_leaves_nothing_behind() {
        let url = serve(b"tampered").await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let error = download(&url, &path, Some(SHA_OF_HELLO), &CancelToken::new(), |_, _| {}).await.unwrap_err();
        assert!(matches!(error, EngineError::Checksum { .. }));
        assert!(!path.exists());
        assert!(!path.with_extension("part").exists());
    }

    #[tokio::test]
    async fn cancellation_leaves_nothing_behind() {
        let url = serve(b"hello").await;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("out.bin");
        let cancel = CancelToken::new();
        cancel.cancel();
        let error = download(&url, &path, None, &cancel, |_, _| {}).await.unwrap_err();
        assert!(matches!(error, EngineError::Cancelled));
        assert!(!path.exists());
        assert!(!path.with_extension("part").exists());
    }
}
