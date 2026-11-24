//! `haryx upgrade`: bring this haryx to the latest release, the way it was installed.
//!
//! - A package manager install (Homebrew, cargo, pipx, uv, npm) runs that manager's upgrade.
//! - The copy inside the HaryX app (macOS `HaryX.app`, the Windows install folder) is updated with
//!   the app, so this only says so.
//! - A standalone binary (a release tarball unpacked by hand) replaces itself
//!   ([`haryx_core::self_update`]).
//! - A source build (`target/`) and a pip install get the manual routes.
//!
//! `haryx upgrade --check [--json] [--refresh]` only reports the latest release; both apps read its
//! JSON (`packages/contracts/haryx/upgrade-check.json`) to drive their own updates.

use std::path::Path;
use std::process::{Command, ExitStatus};

use haryx_core::release::{Asset, Release, download_checked, http_download};
use haryx_core::self_update::{self, Outcome};
use haryx_core::update_check::{is_newer, latest_release};
use haryx_core::{PROG, VERSION, paths, printer};
use serde_json::{Value, json};

use super::CommandError;
use super::profile_view::print_json;

/// Exit code after Ctrl-C.
const INTERRUPTED_EXIT: u8 = 130;
/// The Windows app's executable, beside the `bin\` folder holding its haryx.
const WINDOWS_APP_EXE: &str = "HaryX.exe";

/// How this binary was installed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Homebrew,
    Cargo,
    Pipx,
    Uv,
    Npm,
    /// pip into a Python environment (the wheel's script folder).
    Pip,
    /// Inside the HaryX app.
    App,
    /// A release tarball unpacked by hand.
    Standalone,
}

impl InstallMethod {
    /// The package manager command that upgrades it; `None` when haryx does it (or cannot).
    pub fn command(self) -> Option<&'static [&'static str]> {
        Some(match self {
            InstallMethod::Homebrew => &["brew", "upgrade", "haryx"],
            InstallMethod::Cargo => &["cargo", "install", "--locked", "haryx"],
            InstallMethod::Pipx => &["pipx", "upgrade", "haryx"],
            InstallMethod::Uv => &["uv", "tool", "upgrade", "haryx"],
            InstallMethod::Npm => &["npm", "install", "--global", "haryx@latest"],
            InstallMethod::Pip | InstallMethod::App | InstallMethod::Standalone => return None,
        })
    }

    /// The name `--check --json` reports.
    pub fn key(self) -> &'static str {
        match self {
            InstallMethod::Homebrew => "homebrew",
            InstallMethod::Cargo => "cargo",
            InstallMethod::Pipx => "pipx",
            InstallMethod::Uv => "uv",
            InstallMethod::Npm => "npm",
            InstallMethod::Pip => "pip",
            InstallMethod::App => "app",
            InstallMethod::Standalone => "standalone",
        }
    }

    /// What to tell someone who wants the latest release.
    pub fn hint(self) -> String {
        match self {
            InstallMethod::App => {
                "Update the HaryX app (HaryX › About & Updates); its haryx updates with it.".into()
            }
            InstallMethod::Pip => {
                format!("Run `pip install --upgrade {PROG}` in the environment that installed it.")
            }
            InstallMethod::Standalone => format!("Run `{PROG} upgrade`."),
            other => format!(
                "Run `{PROG} upgrade` ({}).",
                other.command().unwrap_or(&[]).join(" ")
            ),
        }
    }
}

/// Detect the install method from the executable's (symlink-resolved) path. `None` for a build
/// from source. Backslashes are read as slashes, so Windows paths match too.
pub fn detect_install_method(exe: &Path) -> Option<InstallMethod> {
    let path = exe.to_string_lossy().replace('\\', "/");
    let has = |part: &str| path.contains(part);
    if has(".app/Contents/Resources/bin/") {
        Some(InstallMethod::App)
    } else if has("/Cellar/") || has("/linuxbrew/") || has("/homebrew/") {
        Some(InstallMethod::Homebrew)
    } else if has("/.cargo/bin/") {
        Some(InstallMethod::Cargo)
    } else if has("/pipx/venvs/") {
        Some(InstallMethod::Pipx)
    } else if has("/uv/tools/") {
        Some(InstallMethod::Uv)
    } else if has("/node_modules/") {
        Some(InstallMethod::Npm)
    } else if has("/target/debug/") || has("/target/release/") || is_cross_target_dir(&path) {
        None
    } else {
        Some(InstallMethod::Standalone)
    }
}

/// `.../target/<triple>/release/haryx`: a cross build.
fn is_cross_target_dir(path: &str) -> bool {
    path.split("/target/").nth(1).is_some_and(|rest| {
        rest.split('/')
            .nth(1)
            .is_some_and(|d| d == "release" || d == "debug")
    })
}

/// [`detect_install_method`] plus what only the disk can tell: the Windows app's `bin\` folder
/// (`HaryX.exe` one level up) and a pip environment (a `python` beside the script).
pub fn detect_on_disk(exe: &Path) -> Option<InstallMethod> {
    let dir = exe.parent()?;
    if dir
        .file_name()
        .is_some_and(|n| n.eq_ignore_ascii_case("bin"))
        && dir
            .parent()
            .is_some_and(|app| app.join(WINDOWS_APP_EXE).is_file())
    {
        return Some(InstallMethod::App);
    }
    let method = detect_install_method(exe)?;
    let in_python_env = ["python", "python3", "python.exe"]
        .iter()
        .any(|py| dir.join(py).exists());
    if method == InstallMethod::Standalone && in_python_env {
        return Some(InstallMethod::Pip);
    }
    Some(method)
}

/// The version the binary at `exe` reports (`haryx 0.1.0 (...)` → `0.1.0`).
fn version_on_disk(exe: &Path) -> Option<String> {
    let out = Command::new(exe).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1).map(str::to_owned)
}

fn current_exe() -> std::path::PathBuf {
    std::env::current_exe()
        .and_then(|p| haryx_core::fsutil::canonicalize(&p))
        .unwrap_or_default()
}

pub fn run(
    check: bool,
    json: bool,
    refresh: bool,
    download_app: Option<&str>,
) -> Result<(), CommandError> {
    let exe = current_exe();
    self_update::remove_leftovers(&exe);
    if let Some(dir) = download_app {
        return run_download_app(Path::new(dir), json);
    }
    let method = detect_on_disk(&exe);
    if check {
        return run_check(method, json, refresh);
    }
    match method {
        None => {
            printer::error(&manual_routes(&exe));
            Err(CommandError::silent(1))
        }
        Some(InstallMethod::App) => {
            println!(
                "This {PROG} comes with the HaryX app ({}).\n{}",
                exe.display(),
                InstallMethod::App.hint()
            );
            Ok(())
        }
        Some(InstallMethod::Pip) => {
            println!("{}", InstallMethod::Pip.hint());
            Ok(())
        }
        Some(InstallMethod::Standalone) => run_standalone(&exe),
        Some(method) => run_package_manager(&exe, method),
    }
}

fn manual_routes(exe: &Path) -> String {
    format!(
        "This {PROG} looks like a build from source ({}).\nUse `git pull` and rebuild, or install \
         a release with one of:\n  brew upgrade {PROG}\n  cargo install --locked {PROG}\n  pipx \
         upgrade {PROG}\n  uv tool upgrade {PROG}\n  npm install --global {PROG}@latest",
        exe.display()
    )
}

fn store_root() -> std::path::PathBuf {
    paths::store_root().unwrap_or_else(|_| std::env::temp_dir().join(PROG))
}

// ------------------------------------------------------------------------------------- --check

fn run_check(method: Option<InstallMethod>, json: bool, refresh: bool) -> Result<(), CommandError> {
    let result = latest_release(&store_root(), refresh);
    let report = check_report(VERSION, method, &result);
    if json {
        print_json(&report);
        return Ok(());
    }
    match (&result, report["available"].as_bool()) {
        (Err(err), _) => {
            printer::error(&format!("Could not check for updates: {err}"));
            Err(CommandError::silent(1))
        }
        (Ok(None), _) => {
            println!("No release found; try again later.");
            Ok(())
        }
        (Ok(Some(release)), Some(true)) => {
            println!(
                "{PROG} {} is available (you have {VERSION}).",
                release.version
            );
            if let Some(m) = method {
                println!("{}", m.hint());
            }
            Ok(())
        }
        (Ok(Some(_)), _) => {
            println!("{PROG} {VERSION} is up to date.");
            Ok(())
        }
    }
}

fn asset_json(pair: Option<(&Asset, &Asset)>) -> Value {
    pair.map_or(Value::Null, |(asset, sum)| {
        json!({
            "name": asset.name,
            "url": asset.url,
            "sha256_url": sum.url,
            "size": asset.size,
        })
    })
}

/// The `upgrade --check --json` payload. A failed lookup still answers, with `error` set.
pub fn check_report(
    current: &str,
    method: Option<InstallMethod>,
    result: &Result<Option<Release>, String>,
) -> Value {
    let release = result.as_ref().ok().and_then(Option::as_ref);
    json!({
        "current": current,
        "latest": release.map(|r| r.version.clone()),
        "available": release.is_some_and(|r| is_newer(&r.version, current).unwrap_or(false)),
        "published_at": release.and_then(|r| r.published_at.clone()),
        "notes": release.map(|r| r.notes.clone()),
        "html_url": release.and_then(|r| r.html_url.clone()),
        "install": method.map_or("source", InstallMethod::key),
        "assets": {
            "app": asset_json(release.and_then(Release::app_asset)),
            "cli": asset_json(release.and_then(Release::cli_asset)),
        },
        "error": result.as_ref().err(),
    })
}

// -------------------------------------------------------------------------------- --download-app

/// Download this machine's app installer from the latest release into `dir`, checked against its
/// SHA-256. Prints the path (or `{"path", "version", "name"}` with `--json`).
fn run_download_app(dir: &Path, json: bool) -> Result<(), CommandError> {
    let result = latest_release(&store_root(), true).and_then(|release| {
        let release = release.ok_or("no release found; try again later")?;
        let (asset, sum) = release.app_asset().ok_or_else(|| {
            format!(
                "release {} has no checked app for this machine",
                release.version
            )
        })?;
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        let path = download_checked(asset, sum, dir, &http_download)?;
        Ok((release.version.clone(), asset.name.clone(), path))
    });
    match (result, json) {
        (Ok((version, name, path)), true) => {
            print_json(&json!({"path": path, "version": version, "name": name, "error": null}));
            Ok(())
        }
        (Ok((_, _, path)), false) => {
            println!("{}", path.display());
            Ok(())
        }
        (Err(err), true) => {
            print_json(&json!({"path": null, "version": null, "name": null, "error": err}));
            Err(CommandError::silent(1))
        }
        (Err(err), false) => {
            printer::error(&err);
            Err(CommandError::silent(1))
        }
    }
}

// ------------------------------------------------------------------------------------- install

fn run_standalone(exe: &Path) -> Result<(), CommandError> {
    let release = match latest_release(&store_root(), true) {
        Ok(Some(release)) => release,
        Ok(None) => {
            printer::error("No release found; try again later.");
            return Err(CommandError::silent(1));
        }
        Err(err) => {
            printer::error(&format!("Could not check for updates: {err}"));
            return Err(CommandError::silent(1));
        }
    };
    println!(
        "{}",
        printer::dimmed(&format!("Checking {} ...", release.version))
    );
    match self_update::upgrade_standalone(exe, VERSION, &release, &http_download) {
        Ok(Outcome::UpToDate) => {
            println!("{PROG} {VERSION} is up to date.");
            Ok(())
        }
        Ok(Outcome::Upgraded(version)) => {
            println!(
                "{}",
                printer::accent(&format!("{PROG} upgraded: {VERSION} -> {version}"))
            );
            Ok(())
        }
        Err(err) => {
            printer::error(&err);
            Err(CommandError::silent(1))
        }
    }
}

fn run_package_manager(exe: &Path, method: InstallMethod) -> Result<(), CommandError> {
    let Some(cmd) = method.command() else {
        return Ok(());
    };
    let status = match Command::new(cmd[0]).args(&cmd[1..]).status() {
        Ok(status) => status,
        Err(_) => {
            printer::error(&format!(
                "Detected {} install but `{}` is not on PATH. Run the upgrade manually from a \
                 shell where it is available.",
                cmd[0], cmd[0]
            ));
            return Err(CommandError::silent(1));
        }
    };
    if interrupted(&status) {
        println!("\n{}", printer::dimmed("Upgrade cancelled"));
        return Err(CommandError::silent(INTERRUPTED_EXIT));
    }
    if !status.success() {
        let code = status
            .code()
            .and_then(|c| u8::try_from(c).ok())
            .unwrap_or(1);
        return Err(CommandError::silent(code));
    }
    match version_on_disk(exe) {
        Some(after) if after != VERSION => {
            println!(
                "{}",
                printer::accent(&format!("{PROG} upgraded: {VERSION} -> {after}"))
            );
        }
        Some(after) => println!("{PROG} is still at {after}."),
        None => {}
    }
    Ok(())
}

/// Whether the package manager was stopped by Ctrl-C.
#[cfg(unix)]
fn interrupted(status: &ExitStatus) -> bool {
    use std::os::unix::process::ExitStatusExt;
    status.signal() == Some(libc::SIGINT)
}

/// Windows has no signals; Ctrl-C ends a console child with `STATUS_CONTROL_C_EXIT`.
#[cfg(not(unix))]
fn interrupted(status: &ExitStatus) -> bool {
    const STATUS_CONTROL_C_EXIT: i32 = 0xC000_013A_u32.cast_signed();
    status.code() == Some(STATUS_CONTROL_C_EXIT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_install_from_the_path() {
        for (path, method) in [
            (
                "/opt/homebrew/Cellar/haryx/0.1.0/bin/haryx",
                Some(InstallMethod::Homebrew),
            ),
            (
                "/home/linuxbrew/.linuxbrew/bin/haryx",
                Some(InstallMethod::Homebrew),
            ),
            ("/Users/u/.cargo/bin/haryx", Some(InstallMethod::Cargo)),
            (
                r"C:\Users\u\.cargo\bin\haryx.exe",
                Some(InstallMethod::Cargo),
            ),
            (
                "/Users/u/.local/pipx/venvs/haryx/bin/haryx",
                Some(InstallMethod::Pipx),
            ),
            (
                "/Users/u/.local/share/uv/tools/haryx/bin/haryx",
                Some(InstallMethod::Uv),
            ),
            (
                "/usr/local/lib/node_modules/haryx/bin/haryx",
                Some(InstallMethod::Npm),
            ),
            (
                "/Applications/HaryX.app/Contents/Resources/bin/haryx",
                Some(InstallMethod::App),
            ),
            ("/usr/local/bin/haryx", Some(InstallMethod::Standalone)),
            (r"D:\tools\haryx.exe", Some(InstallMethod::Standalone)),
            ("/Users/u/src/haryx/cli/target/release/haryx", None),
            ("/Users/u/src/haryx/cli/target/debug/haryx", None),
            (
                "/w/cli/target/x86_64-pc-windows-msvc/release/haryx.exe",
                None,
            ),
        ] {
            assert_eq!(detect_install_method(Path::new(path)), method, "{path}");
        }
    }

    #[test]
    fn windows_app_folder_and_pip_env_are_read_from_disk() {
        let tmp = tempfile::TempDir::new().unwrap();
        let app = tmp.path().join("HaryX");
        std::fs::create_dir_all(app.join("bin")).unwrap();
        std::fs::write(app.join(WINDOWS_APP_EXE), "").unwrap();
        assert_eq!(
            detect_on_disk(&app.join("bin").join("haryx.exe")),
            Some(InstallMethod::App)
        );
        let venv = tmp.path().join("venv").join("bin");
        std::fs::create_dir_all(&venv).unwrap();
        std::fs::write(venv.join("python3"), "").unwrap();
        assert_eq!(
            detect_on_disk(&venv.join("haryx")),
            Some(InstallMethod::Pip)
        );
        let plain = tmp.path().join("tools");
        std::fs::create_dir_all(&plain).unwrap();
        assert_eq!(
            detect_on_disk(&plain.join("haryx")),
            Some(InstallMethod::Standalone)
        );
    }

    #[test]
    fn commands_name_the_haryx_package() {
        assert_eq!(
            InstallMethod::Uv.command(),
            Some(&["uv", "tool", "upgrade", "haryx"][..])
        );
        assert_eq!(InstallMethod::App.command(), None);
        assert!(InstallMethod::App.hint().contains("HaryX app"));
        assert!(
            InstallMethod::Homebrew
                .hint()
                .contains("brew upgrade haryx")
        );
    }

    fn release() -> Release {
        let asset = |name: &str| Asset {
            name: name.into(),
            url: format!("https://x/{name}"),
            size: 7,
        };
        let cli = haryx_core::release::cli_asset_name(
            "0.2.0",
            haryx_core::release::host_target().unwrap(),
        );
        let mut assets = vec![asset(&cli), asset(&format!("{cli}.sha256"))];
        if let Some(app) = haryx_core::release::app_asset_name(
            "0.2.0",
            std::env::consts::OS,
            std::env::consts::ARCH,
        ) {
            assets.push(asset(&app));
            assets.push(asset(&format!("{app}.sha256")));
        }
        Release {
            version: "0.2.0".into(),
            published_at: Some("2026-11-25T08:00:00Z".into()),
            notes: "- New".into(),
            html_url: Some("https://x/tag".into()),
            assets,
        }
    }

    #[test]
    fn check_report_names_the_update_and_this_platforms_assets() {
        let report = check_report("0.1.4", Some(InstallMethod::App), &Ok(Some(release())));
        assert_eq!(report["current"], "0.1.4");
        assert_eq!(report["latest"], "0.2.0");
        assert_eq!(report["available"], true);
        assert_eq!(report["notes"], "- New");
        assert_eq!(report["install"], "app");
        assert_eq!(report["error"], Value::Null);
        let cli = &report["assets"]["cli"];
        assert!(cli["url"].as_str().unwrap().ends_with(".tar.gz"));
        assert!(
            cli["sha256_url"]
                .as_str()
                .unwrap()
                .ends_with(".tar.gz.sha256")
        );
        assert_eq!(cli["size"], 7);
    }

    #[test]
    fn check_report_up_to_date_and_failed() {
        let same = check_report("0.2.0", None, &Ok(Some(release())));
        assert_eq!(same["available"], false);
        assert_eq!(same["install"], "source");
        let failed = check_report("0.2.0", None, &Err("offline".into()));
        assert_eq!(failed["latest"], Value::Null);
        assert_eq!(failed["available"], false);
        assert_eq!(failed["error"], "offline");
        assert_eq!(failed["assets"]["app"], Value::Null);
    }

    #[test]
    fn check_report_matches_the_contract() {
        let contract: Value = serde_json::from_str(include_str!(
            "../../../../../packages/contracts/haryx/upgrade-check.json"
        ))
        .unwrap();
        let report = check_report("0.1.4", Some(InstallMethod::App), &Ok(Some(release())));
        let keys = |v: &Value| {
            let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
            k.sort();
            k
        };
        assert_eq!(
            keys(&report),
            keys(&contract),
            "fields drift from upgrade-check.json"
        );
        assert_eq!(
            keys(&report["assets"]["cli"]),
            keys(&contract["assets"]["cli"]),
            "asset fields drift from upgrade-check.json"
        );
    }
}
