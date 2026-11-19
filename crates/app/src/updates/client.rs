//! The updater's requests: `latest.json`, then the update itself, over
//! HTTPS with rustls (the core's crypto provider). GitHub answers both with
//! a redirect to its download host, which is followed. Run on the core's
//! tokio runtime.

use std::time::Duration;

use reqwest::header::{ACCEPT, HeaderValue};

/// The manifest is small; a check that takes longer than this failed.
const CHECK_TIMEOUT: Duration = Duration::from_secs(60);
/// A download that stalls this long failed; a slow one goes on.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
/// Progress is told every this many bytes, and at the end.
const PROGRESS_STEP: u64 = 256 * 1024;

fn client() -> Result<reqwest::Client, String> {
    dbforge_core::install_crypto();
    reqwest::Client::builder()
        .user_agent(concat!("DBForge/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(CONNECT_TIMEOUT)
        .read_timeout(READ_TIMEOUT)
        .build()
        .map_err(|error| format!("The update check couldn't start: {error}"))
}

/// The manifest at `endpoint`, as bytes.
pub async fn fetch_manifest(endpoint: &str) -> Result<Vec<u8>, String> {
    let response = client()?
        .get(endpoint)
        .header(ACCEPT, HeaderValue::from_static("application/json"))
        .timeout(CHECK_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("DBForge couldn't reach the update server: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("The update server answered {status}"));
    }
    let body = response.bytes().await.map_err(|error| format!("The update manifest couldn't be read: {error}"))?;
    Ok(body.to_vec())
}

/// Downloads `url`, telling `progress` the bytes so far and the total when known.
pub async fn download(url: &str, mut progress: impl FnMut(u64, Option<u64>)) -> Result<Vec<u8>, String> {
    let mut response = client()?
        .get(url)
        .header(ACCEPT, HeaderValue::from_static("application/octet-stream"))
        .send()
        .await
        .map_err(|error| format!("The update couldn't be downloaded: {error}"))?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!("The download server answered {status}"));
    }
    let total = response.content_length();
    let mut bytes = Vec::with_capacity(total.unwrap_or(0).min(512 * 1024 * 1024) as usize);
    let mut told = 0;
    progress(0, total);
    while let Some(chunk) = response.chunk().await.map_err(|error| format!("The download stopped: {error}"))? {
        bytes.extend_from_slice(&chunk);
        let downloaded = bytes.len() as u64;
        if downloaded - told >= PROGRESS_STEP {
            told = downloaded;
            progress(downloaded, total);
        }
    }
    progress(bytes.len() as u64, total);
    if let Some(total) = total
        && bytes.len() as u64 != total
    {
        return Err(format!("The download ended after {} of {total} bytes", bytes.len()));
    }
    Ok(bytes)
}
