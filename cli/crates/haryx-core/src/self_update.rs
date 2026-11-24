//! `haryx upgrade` for a standalone install (a release tarball unpacked by hand): download this
//! platform's tarball from the latest release, check its SHA-256, check the new binary runs and
//! reports the release's version, then swap it in beside the running one.
//!
//! The staging folder sits next to the binary so the final step is a rename on one file system.
//! Unix replaces the file in place (a running binary keeps its old inode). Windows cannot overwrite
//! a running `.exe`, so the old one is renamed to `<name>.old` first and removed on a later run
//! ([`remove_leftovers`]). `haryxw.exe` (the windowless wake runner) is replaced alongside when it
//! is installed and the tarball carries it.

use std::path::{Path, PathBuf};
use std::process::Command;

use flate2::read::GzDecoder;

use crate::release::{Downloader, Release, download_checked};
use crate::update_check::is_newer;

/// What the upgrade did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Already on the latest release.
    UpToDate,
    /// Replaced; the version now installed.
    Upgraded(String),
}

/// The executables a release tarball can carry, besides `LICENSE` and `README.md`.
fn executable_names(windows: bool) -> &'static [&'static str] {
    if windows {
        &["haryx.exe", "haryxw.exe"]
    } else {
        &["haryx"]
    }
}

/// Upgrade the standalone binary at `exe` (its resolved path) to `release`.
pub fn upgrade_standalone(
    exe: &Path,
    current: &str,
    release: &Release,
    get: Downloader,
) -> Result<Outcome, String> {
    if !is_newer(&release.version, current).unwrap_or(false) {
        return Ok(Outcome::UpToDate);
    }
    let (asset, checksum) = release.cli_asset().ok_or_else(|| {
        format!(
            "release {} has no checked haryx build for this platform",
            release.version
        )
    })?;
    let dir = exe
        .parent()
        .ok_or_else(|| format!("{} has no folder", exe.display()))?;
    let stage = Staging::create(dir)?;
    let tarball = download_checked(asset, checksum, stage.path(), get)?;
    let windows = cfg!(windows);
    let unpacked = unpack(
        &tarball,
        &stage.path().join("new"),
        executable_names(windows),
    )?;
    let main_name = executable_names(windows)[0];
    let new_main = unpacked
        .iter()
        .find(|p| p.file_name().is_some_and(|n| n == main_name))
        .ok_or_else(|| format!("{} does not contain {main_name}", asset.name))?;
    verify_version(new_main, &release.version)?;
    // The windowless runner first (only where it is installed), the main binary last: a failure
    // part-way leaves the running haryx as it was.
    for new in unpacked.iter().filter(|p| *p != new_main) {
        let dest = dir.join(new.file_name().unwrap_or_default());
        if dest.exists() {
            swap_in(new, &dest, windows)?;
        }
    }
    swap_in(new_main, exe, windows)?;
    Ok(Outcome::Upgraded(release.version.clone()))
}

/// A folder beside the binary, removed when dropped.
struct Staging(PathBuf);

impl Staging {
    fn create(dir: &Path) -> Result<Staging, String> {
        let path = dir.join(format!(".haryx-upgrade-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir(&path).map_err(|e| {
            format!(
                "cannot write to {} ({e}); nothing was changed. Re-run with permission to \
                 write there, or reinstall haryx elsewhere.",
                dir.display()
            )
        })?;
        Ok(Staging(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Staging {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Extract the root-level files named in `wanted` from a `.tar.gz` into `dest`. Nothing else is
/// written, so an entry with a path cannot land outside `dest`.
pub fn unpack(tarball: &Path, dest: &Path, wanted: &[&str]) -> Result<Vec<PathBuf>, String> {
    let file = std::fs::File::open(tarball)
        .map_err(|e| format!("cannot open {}: {e}", tarball.display()))?;
    std::fs::create_dir_all(dest).map_err(|e| format!("cannot create {}: {e}", dest.display()))?;
    let mut archive = tar::Archive::new(GzDecoder::new(file));
    let entries = archive
        .entries()
        .map_err(|e| format!("cannot read {}: {e}", tarball.display()))?;
    let mut out = Vec::new();
    for entry in entries {
        let mut entry = entry.map_err(|e| format!("cannot read {}: {e}", tarball.display()))?;
        let path = entry
            .path()
            .map_err(|e| format!("bad entry in {}: {e}", tarball.display()))?
            .into_owned();
        let Some(name) = path.to_str().filter(|n| wanted.contains(n)) else {
            continue;
        };
        let target = dest.join(name);
        let mut file = std::fs::File::create(&target)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        std::io::copy(&mut entry, &mut file)
            .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        out.push(target);
    }
    Ok(out)
}

/// Run `exe --version` and require it to name `expected` (`haryx 0.2.0 (...)`).
fn verify_version(exe: &Path, expected: &str) -> Result<(), String> {
    make_executable(exe)?;
    let output = Command::new(exe)
        .arg("--version")
        .output()
        .map_err(|e| format!("the downloaded haryx does not run here: {e}; nothing was changed"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let reported = text.split_whitespace().nth(1).unwrap_or_default();
    if output.status.success() && reported == expected {
        Ok(())
    } else {
        Err(format!(
            "the downloaded haryx reports {:?}, not {expected}; nothing was changed",
            text.trim()
        ))
    }
}

#[cfg(unix)]
fn make_executable(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .map_err(|e| format!("cannot mark {} executable: {e}", path.display()))
}

#[cfg(not(unix))]
fn make_executable(_path: &Path) -> Result<(), String> {
    Ok(())
}

/// `<dest>.old`: where Windows parks a binary that is still running.
pub fn parked(dest: &Path) -> PathBuf {
    let mut name = dest.file_name().unwrap_or_default().to_os_string();
    name.push(".old");
    dest.with_file_name(name)
}

/// Move `new` over `dest`. `park` (Windows) moves the running `dest` aside to `<dest>.old` first,
/// and puts it back if the second move fails.
pub fn swap_in(new: &Path, dest: &Path, park: bool) -> Result<(), String> {
    let fail = |e: std::io::Error| format!("cannot replace {}: {e}", dest.display());
    if !park {
        return std::fs::rename(new, dest).map_err(fail);
    }
    let old = parked(dest);
    let _ = std::fs::remove_file(&old);
    if dest.exists() {
        std::fs::rename(dest, &old).map_err(fail)?;
    }
    std::fs::rename(new, dest).map_err(|e| {
        let _ = std::fs::rename(&old, dest);
        fail(e)
    })
}

/// Delete `<name>.old` files a Windows upgrade left beside `exe`. Best effort: one still in use
/// stays until a later run.
pub fn remove_leftovers(exe: &Path) {
    let Some(dir) = exe.parent() else { return };
    for name in executable_names(true) {
        let _ = std::fs::remove_file(parked(&dir.join(name)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::release::Asset;
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    fn tarball(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(flate2::write::GzEncoder::new(
            Vec::new(),
            flate2::Compression::fast(),
        ));
        for (name, body) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(body.len() as u64);
            header.set_mode(0o755);
            header.set_cksum();
            builder.append_data(&mut header, name, *body).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// A release for this host whose tarball is `tgz`, and a downloader serving it.
    fn release_serving(
        version: &str,
        tgz: Vec<u8>,
        checksum: Option<String>,
    ) -> (Release, impl Fn(&str, &Path) -> Result<(), String>) {
        let name = crate::release::cli_asset_name(version, crate::release::host_target().unwrap());
        let sum = checksum.unwrap_or_else(|| format!("{:x}", Sha256::digest(&tgz)));
        let release = Release {
            version: version.into(),
            published_at: None,
            notes: String::new(),
            html_url: None,
            assets: vec![
                Asset {
                    name: name.clone(),
                    url: "tgz".into(),
                    size: tgz.len() as u64,
                },
                Asset {
                    name: format!("{name}.sha256"),
                    url: "sum".into(),
                    size: 64,
                },
            ],
        };
        let get = move |url: &str, dest: &Path| {
            let body = if url == "tgz" {
                tgz.clone()
            } else {
                format!("{sum}  x\n").into_bytes()
            };
            std::fs::write(dest, body).map_err(|e| e.to_string())
        };
        (release, get)
    }

    #[test]
    fn up_to_date_downloads_nothing() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("haryx");
        let release = Release {
            version: "0.1.4".into(),
            published_at: None,
            notes: String::new(),
            html_url: None,
            assets: vec![],
        };
        let get = |_: &str, _: &Path| -> Result<(), String> { panic!("no download expected") };
        assert_eq!(
            upgrade_standalone(&exe, "0.1.4", &release, &get),
            Ok(Outcome::UpToDate)
        );
    }

    #[cfg(unix)]
    #[test]
    fn replaces_the_binary_after_checking_it() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("haryx");
        std::fs::write(&exe, "#!/bin/sh\necho 'haryx 0.1.0 (old)'\n").unwrap();
        let script: &[u8] = b"#!/bin/sh\necho 'haryx 0.2.0 (new)'\n";
        let (release, get) = release_serving(
            "0.2.0",
            tarball(&[("haryx", script), ("LICENSE", b"MIT")]),
            None,
        );
        let outcome = upgrade_standalone(&exe, "0.1.0", &release, &get).unwrap();
        assert_eq!(outcome, Outcome::Upgraded("0.2.0".into()));
        assert_eq!(std::fs::read(&exe).unwrap(), script);
        assert!(
            !tmp.path().join("LICENSE").exists(),
            "only executables are installed"
        );
        let leftovers: Vec<_> = std::fs::read_dir(tmp.path()).unwrap().collect();
        assert_eq!(leftovers.len(), 1, "staging folder removed");
    }

    #[cfg(unix)]
    #[test]
    fn a_binary_reporting_the_wrong_version_is_refused() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("haryx");
        std::fs::write(&exe, "old").unwrap();
        let script: &[u8] = b"#!/bin/sh\necho 'haryx 0.1.9 (stale)'\n";
        let (release, get) = release_serving("0.2.0", tarball(&[("haryx", script)]), None);
        let err = upgrade_standalone(&exe, "0.1.0", &release, &get).unwrap_err();
        assert!(err.contains("not 0.2.0"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn a_checksum_mismatch_changes_nothing() {
        let tmp = TempDir::new().unwrap();
        let exe = tmp.path().join("haryx");
        std::fs::write(&exe, "old").unwrap();
        let (release, get) =
            release_serving("0.2.0", tarball(&[("haryx", b"x")]), Some("0".repeat(64)));
        let err = upgrade_standalone(&exe, "0.1.0", &release, &get).unwrap_err();
        assert!(err.contains("does not match its checksum"), "{err}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old");
    }

    #[test]
    fn a_release_without_this_platform_is_an_error() {
        let tmp = TempDir::new().unwrap();
        let release = Release {
            version: "0.2.0".into(),
            published_at: None,
            notes: String::new(),
            html_url: None,
            assets: vec![],
        };
        let get = |_: &str, _: &Path| -> Result<(), String> { Ok(()) };
        let err =
            upgrade_standalone(&tmp.path().join("haryx"), "0.1.0", &release, &get).unwrap_err();
        assert!(err.contains("no checked haryx build"), "{err}");
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_folder_is_reported() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("ro");
        std::fs::create_dir(&dir).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let (release, get) = release_serving("0.2.0", tarball(&[("haryx", b"x")]), None);
        let result = upgrade_standalone(&dir.join("haryx"), "0.1.0", &release, &get);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        if nix_is_root() {
            return; // root writes anywhere
        }
        assert!(result.unwrap_err().contains("cannot write to"));
    }

    #[cfg(unix)]
    fn nix_is_root() -> bool {
        // SAFETY: geteuid has no preconditions.
        unsafe { libc::geteuid() == 0 }
    }

    #[test]
    fn unpack_takes_only_named_root_files() {
        let tmp = TempDir::new().unwrap();
        let tgz = tmp.path().join("a.tar.gz");
        std::fs::write(
            &tgz,
            tarball(&[
                ("haryx", b"bin"),
                ("nested/haryx", b"no"),
                ("README.md", b"no"),
            ]),
        )
        .unwrap();
        let out = unpack(&tgz, &tmp.path().join("out"), &["haryx"]).unwrap();
        assert_eq!(out, vec![tmp.path().join("out").join("haryx")]);
        assert_eq!(std::fs::read(&out[0]).unwrap(), b"bin");
    }

    #[test]
    fn parked_swap_keeps_the_old_binary_aside_and_cleans_up_later() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("haryx.exe");
        let new = tmp.path().join("new.exe");
        std::fs::write(&dest, "old").unwrap();
        std::fs::write(&new, "new").unwrap();
        swap_in(&new, &dest, true).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"new");
        assert_eq!(std::fs::read(parked(&dest)).unwrap(), b"old");
        remove_leftovers(&dest);
        assert!(!parked(&dest).exists());
    }

    #[test]
    fn parked_swap_restores_the_old_binary_when_the_move_fails() {
        let tmp = TempDir::new().unwrap();
        let dest = tmp.path().join("haryx.exe");
        std::fs::write(&dest, "old").unwrap();
        let err = swap_in(&tmp.path().join("missing.exe"), &dest, true).unwrap_err();
        assert!(err.contains("cannot replace"), "{err}");
        assert_eq!(std::fs::read(&dest).unwrap(), b"old");
    }
}
