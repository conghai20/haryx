//! The latest published HaryX release and the files in it: which asset this machine's app or CLI
//! updates from, and downloading one with its `.sha256` checked.
//!
//! The release comes from GitHub's latest-release API ([`crate::update_check::RELEASES_URL`]),
//! which never names a draft or a pre-release. Only the fields used here are kept ([`trim`]), so the
//! cached copy stays small and keeps GitHub's key names: [`Release::from_json`] reads both.
//!
//! Asset names follow `scripts/release/`: `HaryX-<v>.dmg`, `HaryX-<v>-<x64|arm64>-setup.exe`,
//! `haryx-v<v>-<target>.tar.gz`, each with a `<name>.sha256` beside it.

use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::{PROG, VERSION};

/// Bound on a download (a DMG is tens of megabytes).
pub const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(600);

/// One file attached to a release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Asset {
    pub name: String,
    pub url: String,
    pub size: u64,
}

/// A published release.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    /// The tag without its leading `v`.
    pub version: String,
    pub published_at: Option<String>,
    /// The release notes (Markdown, rendered from CHANGELOG.md).
    pub notes: String,
    pub html_url: Option<String>,
    pub assets: Vec<Asset>,
}

/// The fields of a GitHub release object that [`Release::from_json`] reads; the rest is dropped.
pub fn trim(data: &Value) -> Value {
    let assets: Vec<Value> = data
        .get("assets")
        .and_then(Value::as_array)
        .map(|assets| {
            assets
                .iter()
                .map(|a| {
                    json!({
                        "name": a.get("name"),
                        "browser_download_url": a.get("browser_download_url"),
                        "size": a.get("size"),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    json!({
        "tag_name": data.get("tag_name"),
        "published_at": data.get("published_at"),
        "body": data.get("body"),
        "html_url": data.get("html_url"),
        "assets": assets,
    })
}

impl Release {
    /// Read a GitHub release object (whole or [`trim`]med). `None` without a `tag_name`.
    pub fn from_json(data: &Value) -> Option<Release> {
        let tag = data.get("tag_name").and_then(Value::as_str)?;
        let text = |key: &str| data.get(key).and_then(Value::as_str).map(str::to_owned);
        let assets = data
            .get("assets")
            .and_then(Value::as_array)
            .map(|assets| assets.iter().filter_map(asset_of).collect())
            .unwrap_or_default();
        Some(Release {
            version: tag.strip_prefix('v').unwrap_or(tag).to_owned(),
            published_at: text("published_at"),
            notes: text("body").unwrap_or_default(),
            html_url: text("html_url"),
            assets,
        })
    }

    pub fn asset(&self, name: &str) -> Option<&Asset> {
        self.assets.iter().find(|a| a.name == name)
    }

    /// The asset named `name` with the URL of its `.sha256`; `None` unless both are attached, since
    /// an update is never installed unchecked.
    pub fn checked_asset(&self, name: &str) -> Option<(&Asset, &Asset)> {
        Some((self.asset(name)?, self.asset(&format!("{name}.sha256"))?))
    }

    /// This machine's app installer, with its checksum.
    pub fn app_asset(&self) -> Option<(&Asset, &Asset)> {
        self.checked_asset(&app_asset_name(
            &self.version,
            std::env::consts::OS,
            std::env::consts::ARCH,
        )?)
    }

    /// This machine's CLI tarball, with its checksum.
    pub fn cli_asset(&self) -> Option<(&Asset, &Asset)> {
        self.checked_asset(&cli_asset_name(&self.version, host_target()?))
    }
}

fn asset_of(a: &Value) -> Option<Asset> {
    Some(Asset {
        name: a.get("name")?.as_str()?.to_owned(),
        url: a.get("browser_download_url")?.as_str()?.to_owned(),
        size: a.get("size").and_then(Value::as_u64).unwrap_or(0),
    })
}

/// The app installer for `os`/`arch` (`std::env::consts` spellings). Linux has no app.
pub fn app_asset_name(version: &str, os: &str, arch: &str) -> Option<String> {
    match (os, arch) {
        ("macos", _) => Some(format!("HaryX-{version}.dmg")),
        ("windows", "x86_64") => Some(format!("HaryX-{version}-x64-setup.exe")),
        ("windows", "aarch64") => Some(format!("HaryX-{version}-arm64-setup.exe")),
        _ => None,
    }
}

/// The CLI tarball for a Rust target triple.
pub fn cli_asset_name(version: &str, target: &str) -> String {
    format!("{PROG}-v{version}-{target}.tar.gz")
}

/// The release target this binary was built for, as the release workflow names it.
pub fn host_target() -> Option<&'static str> {
    target_for(std::env::consts::OS, std::env::consts::ARCH)
}

/// The release target for `os`/`arch`; `None` for a platform with no release build.
pub fn target_for(os: &str, arch: &str) -> Option<&'static str> {
    Some(match (os, arch) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        ("windows", "aarch64") => "aarch64-pc-windows-msvc",
        ("windows", "x86_64") => "x86_64-pc-windows-msvc",
        _ => return None,
    })
}

// ------------------------------------------------------------------------------------- download

/// Fetches a URL into a file. The real one is [`http_download`]; tests serve files from disk.
pub type Downloader<'a> = &'a dyn Fn(&str, &Path) -> Result<(), String>;

/// Download `url` to `dest` over HTTPS.
pub fn http_download(url: &str, dest: &Path) -> Result<(), String> {
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .timeout_global(Some(DOWNLOAD_TIMEOUT))
        .tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        )
        .build()
        .into();
    let resp = agent
        .get(url)
        .header("User-Agent", &format!("{PROG}/{VERSION}"))
        .call()
        .map_err(|e| format!("download failed: {url}: {e}"))?;
    let mut reader = resp.into_body().into_reader();
    let mut file =
        std::fs::File::create(dest).map_err(|e| format!("cannot write {}: {e}", dest.display()))?;
    std::io::copy(&mut reader, &mut file)
        .map_err(|e| format!("download interrupted: {url}: {e}"))?;
    file.flush()
        .map_err(|e| format!("cannot write {}: {e}", dest.display()))
}

/// The hex digest a `.sha256` file names (`<hex>  <name>`, as `shasum -a 256` writes it).
pub fn parse_checksum(text: &str) -> Option<String> {
    let hex = text.split_whitespace().next()?.to_ascii_lowercase();
    (hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit())).then_some(hex)
}

/// The SHA-256 of a file, as lowercase hex.
pub fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut file = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// Download `asset` and its checksum into `dir`, returning the file once its SHA-256 matches.
/// A mismatch deletes the file.
pub fn download_checked(
    asset: &Asset,
    checksum: &Asset,
    dir: &Path,
    get: Downloader,
) -> Result<std::path::PathBuf, String> {
    let dest = dir.join(&asset.name);
    let sum_path = dir.join(&checksum.name);
    get(&checksum.url, &sum_path)?;
    let sum_text = std::fs::read_to_string(&sum_path)
        .map_err(|e| format!("cannot read {}: {e}", sum_path.display()))?;
    let expected = parse_checksum(&sum_text)
        .ok_or_else(|| format!("{} is not a SHA-256 file", checksum.name))?;
    get(&asset.url, &dest)?;
    let actual = sha256_file(&dest).map_err(|e| format!("cannot read {}: {e}", dest.display()))?;
    if actual != expected {
        let _ = std::fs::remove_file(&dest);
        return Err(format!(
            "{} does not match its checksum (expected {expected}, got {actual}); nothing was changed",
            asset.name
        ));
    }
    Ok(dest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn github_release() -> Value {
        json!({
            "tag_name": "v0.2.0",
            "published_at": "2026-11-25T08:00:00Z",
            "body": "### Added\n- Updates",
            "html_url": "https://github.com/conghai20/haryx/releases/tag/v0.2.0",
            "draft": false,
            "author": {"login": "someone"},
            "assets": [
                {"name": "HaryX-0.2.0.dmg", "browser_download_url": "https://x/HaryX-0.2.0.dmg", "size": 100, "uploader": {}},
                {"name": "HaryX-0.2.0.dmg.sha256", "browser_download_url": "https://x/HaryX-0.2.0.dmg.sha256", "size": 1},
                {"name": "HaryX-0.2.0-x64-setup.exe", "browser_download_url": "https://x/setup.exe", "size": 50},
                {"name": "haryx-v0.2.0-aarch64-apple-darwin.tar.gz", "browser_download_url": "https://x/a.tgz", "size": 9},
                {"name": "haryx-v0.2.0-aarch64-apple-darwin.tar.gz.sha256", "browser_download_url": "https://x/a.tgz.sha256", "size": 1},
            ]
        })
    }

    #[test]
    fn reads_a_github_release_whole_or_trimmed() {
        let whole = Release::from_json(&github_release()).unwrap();
        let trimmed = trim(&github_release());
        assert!(trimmed.get("author").is_none());
        assert!(trimmed["assets"][0].get("uploader").is_none());
        assert_eq!(Release::from_json(&trimmed).unwrap(), whole);
        assert_eq!(whole.version, "0.2.0");
        assert_eq!(whole.notes, "### Added\n- Updates");
        assert_eq!(whole.assets.len(), 5);
        assert_eq!(whole.asset("HaryX-0.2.0.dmg").unwrap().size, 100);
    }

    #[test]
    fn no_tag_is_no_release() {
        assert_eq!(Release::from_json(&json!({"message": "Not Found"})), None);
    }

    #[test]
    fn an_asset_without_its_checksum_is_not_offered() {
        let release = Release::from_json(&github_release()).unwrap();
        assert!(release.checked_asset("HaryX-0.2.0.dmg").is_some());
        // The setup.exe has no .sha256 attached.
        assert!(release.checked_asset("HaryX-0.2.0-x64-setup.exe").is_none());
    }

    #[test]
    fn asset_names_per_platform() {
        assert_eq!(
            app_asset_name("1.0.0", "macos", "aarch64").unwrap(),
            "HaryX-1.0.0.dmg"
        );
        assert_eq!(
            app_asset_name("1.0.0", "macos", "x86_64").unwrap(),
            "HaryX-1.0.0.dmg"
        );
        assert_eq!(
            app_asset_name("1.0.0", "windows", "x86_64").unwrap(),
            "HaryX-1.0.0-x64-setup.exe"
        );
        assert_eq!(
            app_asset_name("1.0.0", "windows", "aarch64").unwrap(),
            "HaryX-1.0.0-arm64-setup.exe"
        );
        assert_eq!(app_asset_name("1.0.0", "linux", "x86_64"), None);
        assert_eq!(
            cli_asset_name("1.0.0", "x86_64-pc-windows-msvc"),
            "haryx-v1.0.0-x86_64-pc-windows-msvc.tar.gz"
        );
        assert_eq!(
            target_for("linux", "x86_64"),
            Some("x86_64-unknown-linux-musl")
        );
        assert_eq!(target_for("freebsd", "x86_64"), None);
        assert!(host_target().is_some());
    }

    #[test]
    fn checksum_files() {
        let hex = "a".repeat(64);
        assert_eq!(
            parse_checksum(&format!("{hex}  HaryX.dmg\n")),
            Some(hex.clone())
        );
        assert_eq!(parse_checksum(&hex.to_uppercase()), Some(hex));
        assert_eq!(parse_checksum("abc  x"), None);
        assert_eq!(parse_checksum(""), None);
    }

    /// A downloader serving `files` (url → bytes).
    fn serve(files: Vec<(String, Vec<u8>)>) -> impl Fn(&str, &Path) -> Result<(), String> {
        move |url, dest| {
            let (_, body) = files
                .iter()
                .find(|(u, _)| u == url)
                .ok_or_else(|| format!("404 {url}"))?;
            std::fs::write(dest, body).map_err(|e| e.to_string())
        }
    }

    fn pair() -> (Asset, Asset) {
        let asset = Asset {
            name: "f.tgz".into(),
            url: "u/f.tgz".into(),
            size: 5,
        };
        let sum = Asset {
            name: "f.tgz.sha256".into(),
            url: "u/f.tgz.sha256".into(),
            size: 1,
        };
        (asset, sum)
    }

    #[test]
    fn download_checked_accepts_a_matching_file() {
        let tmp = TempDir::new().unwrap();
        let (asset, sum) = pair();
        let digest = format!("{:x}", Sha256::digest(b"hello"));
        let get = serve(vec![
            ("u/f.tgz".into(), b"hello".to_vec()),
            (
                "u/f.tgz.sha256".into(),
                format!("{digest}  f.tgz\n").into_bytes(),
            ),
        ]);
        let path = download_checked(&asset, &sum, tmp.path(), &get).unwrap();
        assert_eq!(std::fs::read(path).unwrap(), b"hello");
    }

    #[test]
    fn download_checked_refuses_and_deletes_a_mismatch() {
        let tmp = TempDir::new().unwrap();
        let (asset, sum) = pair();
        let get = serve(vec![
            ("u/f.tgz".into(), b"tampered".to_vec()),
            (
                "u/f.tgz.sha256".into(),
                format!("{}  f.tgz\n", "0".repeat(64)).into_bytes(),
            ),
        ]);
        let err = download_checked(&asset, &sum, tmp.path(), &get).unwrap_err();
        assert!(err.contains("does not match its checksum"), "{err}");
        assert!(!tmp.path().join("f.tgz").exists());
    }

    #[test]
    fn download_checked_refuses_a_bad_checksum_file() {
        let tmp = TempDir::new().unwrap();
        let (asset, sum) = pair();
        let get = serve(vec![(
            "u/f.tgz.sha256".into(),
            b"<html>404</html>".to_vec(),
        )]);
        let err = download_checked(&asset, &sum, tmp.path(), &get).unwrap_err();
        assert!(err.contains("not a SHA-256 file"), "{err}");
    }
}
