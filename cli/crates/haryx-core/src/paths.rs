//! Port of `claude_swap/paths.py`: path resolution for Claude Code's config and credential files, and the
//! haryx store root.
//!
//! Mirrors Claude Code's own resolution so haryx reads and writes the same files Claude Code does. Key rules
//! (from the Claude Code source):
//!
//! - Config home: `CLAUDE_CONFIG_DIR` if set (and non-empty), else `~/.claude`.
//! - Global config: `<config_home>/.config.json` (legacy) if it is a regular file that is non-empty, or empty
//!   and writable with no `.claude.json` (the rule of Claude Code 2.1.299), otherwise
//!   `(CLAUDE_CONFIG_DIR || $HOME)/.claude.json`. Note the asymmetry: `.claude.json` sits at the home dir by
//!   default, not inside `.claude/`.
//! - Credentials: `.credentials.json` in the secure-storage directory, which is
//!   `CLAUDE_SECURESTORAGE_CONFIG_DIR` when that is defined (empty means `~/.claude`) and the config home
//!   otherwise.
//!
//! The store root is haryx's own (not shared with cswap): macOS `~/Library/Application Support/HaryX`, Linux
//! `$XDG_DATA_HOME/haryx` (default `~/.local/share/haryx`), Windows `%LOCALAPPDATA%\haryx`, and `HARYX_HOME`
//! overrides it on any OS. Upstream's
//! legacy-to-XDG migration is dropped (haryx has no legacy layout); [`cswap_store_root`] and
//! [`cswap_store_candidates`] keep upstream's resolution for `import --from-cswap`.
//!
//! Every resolver has a pure `*_in(&PathEnv)` form so tests never read the process environment or the real
//! home; the argument-free forms snapshot the process environment with [`PathEnv::from_process`].

use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use crate::error::{Error, Result};

/// Upstream's legacy (pre-XDG) cswap backup directory name, under the home directory.
pub const CSWAP_LEGACY_BACKUP_DIRNAME: &str = ".claude-swap-backup";
/// Upstream's directory name under `$XDG_DATA_HOME`.
pub const CSWAP_XDG_DIRNAME: &str = "claude-swap";
/// haryx's directory name under `$XDG_DATA_HOME` (Linux).
pub const HARYX_XDG_DIRNAME: &str = "haryx";
/// haryx's directory name under `~/Library/Application Support` (macOS).
pub const HARYX_MACOS_DIRNAME: &str = "HaryX";
/// haryx's directory name under `%LOCALAPPDATA%` (Windows).
pub const HARYX_WINDOWS_DIRNAME: &str = "haryx";

/// The operating system family the path rules branch on (upstream `models.Platform`, reduced to what the
/// path rules distinguish: WSL resolves exactly like Linux).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostOs {
    MacOs,
    Linux,
    Windows,
    Other,
}

impl HostOs {
    /// The OS this binary was built for.
    pub fn current() -> Self {
        if cfg!(target_os = "macos") {
            HostOs::MacOs
        } else if cfg!(target_os = "linux") {
            HostOs::Linux
        } else if cfg!(windows) {
            HostOs::Windows
        } else {
            HostOs::Other
        }
    }
}

/// A snapshot of everything the path rules read from the environment.
///
/// `None` means "unset". Upstream treats an empty `CLAUDE_CONFIG_DIR`, `XDG_DATA_HOME` (and here
/// `HARYX_HOME`) like an unset one, but an empty `CLAUDE_SECURESTORAGE_CONFIG_DIR` is meaningful (it selects
/// the default profile), so the raw values are kept and each rule applies its own reading.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PathEnv {
    pub home: PathBuf,
    pub claude_config_dir: Option<OsString>,
    pub secure_storage_dir: Option<OsString>,
    pub xdg_data_home: Option<OsString>,
    pub haryx_home: Option<OsString>,
    /// `LOCALAPPDATA` (Windows): the per-machine, per-user data folder that holds the store.
    pub local_app_data: Option<OsString>,
    pub os: HostOs,
}

impl PathEnv {
    /// An environment with only a home directory set, for the current OS.
    pub fn with_home(home: impl Into<PathBuf>) -> Self {
        PathEnv {
            home: home.into(),
            claude_config_dir: None,
            secure_storage_dir: None,
            xdg_data_home: None,
            haryx_home: None,
            local_app_data: None,
            os: HostOs::current(),
        }
    }

    /// Snapshot the process environment. The home directory honours `HOME` first (as Python's
    /// `Path.home()` does), then the OS account database.
    pub fn from_process() -> Result<Self> {
        Ok(PathEnv {
            home: home_dir()?,
            claude_config_dir: std::env::var_os("CLAUDE_CONFIG_DIR"),
            secure_storage_dir: std::env::var_os("CLAUDE_SECURESTORAGE_CONFIG_DIR"),
            xdg_data_home: std::env::var_os("XDG_DATA_HOME"),
            haryx_home: std::env::var_os("HARYX_HOME"),
            local_app_data: std::env::var_os("LOCALAPPDATA"),
            os: HostOs::current(),
        })
    }

    /// `CLAUDE_CONFIG_DIR` when set and non-empty (upstream's `if env:`).
    fn config_dir_override(&self) -> Option<PathBuf> {
        non_empty(self.claude_config_dir.as_ref()).map(PathBuf::from)
    }

    /// Expand a leading `~` (alone or followed by `/`) against this environment's home.
    fn expand_tilde(&self, raw: &OsString) -> PathBuf {
        let path = Path::new(raw);
        match path.strip_prefix("~") {
            Ok(rest) if rest.as_os_str().is_empty() => self.home.clone(),
            Ok(rest) => self.home.join(rest),
            Err(_) => path.to_path_buf(),
        }
    }
}

fn non_empty(value: Option<&OsString>) -> Option<&OsString> {
    value.filter(|v| !v.is_empty())
}

/// The user's home directory: `HOME` when set and non-empty, else the OS account database.
pub fn home_dir() -> Result<PathBuf> {
    if let Some(home) = non_empty(std::env::var_os("HOME").as_ref()) {
        return Ok(PathBuf::from(home));
    }
    dirs::home_dir().ok_or_else(|| Error::General("Could not determine the home directory".into()))
}

// ---------------------------------------------------------------------------------------------------------
// Claude Code paths
// ---------------------------------------------------------------------------------------------------------

/// The Claude config home directory (`CLAUDE_CONFIG_DIR` or `~/.claude`).
pub fn claude_config_home_in(env: &PathEnv) -> PathBuf {
    env.config_dir_override()
        .unwrap_or_else(|| env.home.join(".claude"))
}

/// Claude Code 2.1.299's choice between `.config.json` and `.claude.json`.
fn pick_global_config(legacy: PathBuf, global_cfg: PathBuf) -> PathBuf {
    // Follows symlinks; an error makes the global file win.
    let Ok(meta) = std::fs::metadata(&legacy) else {
        return global_cfg;
    };
    if meta.is_file() && (meta.len() > 0 || (is_writable_by_anyone(&meta) && !global_cfg.exists()))
    {
        return legacy;
    }
    global_cfg
}

#[cfg(unix)]
fn is_writable_by_anyone(meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    meta.permissions().mode() & 0o222 != 0
}

#[cfg(not(unix))]
fn is_writable_by_anyone(meta: &std::fs::Metadata) -> bool {
    !meta.permissions().readonly()
}

/// The global Claude config file: the legacy `<config_home>/.config.json` if Claude Code would use it, else
/// `(CLAUDE_CONFIG_DIR || $HOME)/.claude.json`.
pub fn global_config_path_in(env: &PathEnv) -> PathBuf {
    let base = env
        .config_dir_override()
        .unwrap_or_else(|| env.home.clone());
    pick_global_config(
        claude_config_home_in(env).join(".config.json"),
        base.join(".claude.json"),
    )
}

/// The *default* profile's config home, ignoring `CLAUDE_CONFIG_DIR`.
///
/// Credential capture has to tell an env var that names the default profile from one that names another,
/// since only the former's credential is the active store's.
pub fn default_claude_config_home_in(env: &PathEnv) -> PathBuf {
    env.home.join(".claude")
}

/// The global config path of the *default* profile.
///
/// Same legacy fallback as [`global_config_path_in`], but deliberately ignores `CLAUDE_CONFIG_DIR`: callers
/// that mirror the user's real profile (session sharing) must not source from another session when invoked
/// from inside one.
pub fn default_global_config_path_in(env: &PathEnv) -> PathBuf {
    pick_global_config(
        default_claude_config_home_in(env).join(".config.json"),
        env.home.join(".claude.json"),
    )
}

/// The Claude credentials file.
///
/// Claude Code (2.1.322) keeps the file in its secure-storage directory: `CLAUDE_SECURESTORAGE_CONFIG_DIR`
/// when that is defined, where an empty value selects the default profile's `~/.claude`, and the config home
/// otherwise. Its Keychain item is named from the same variable, so the file and the item always belong to
/// one profile.
pub fn credentials_path_in(env: &PathEnv) -> PathBuf {
    let secure_home = match &env.secure_storage_dir {
        None => claude_config_home_in(env),
        Some(dir) if dir.is_empty() => default_claude_config_home_in(env),
        Some(dir) => PathBuf::from(dir),
    };
    secure_home.join(".credentials.json")
}

// ---------------------------------------------------------------------------------------------------------
// Stores
// ---------------------------------------------------------------------------------------------------------

/// `$XDG_DATA_HOME/<name>`, per the XDG spec: `$XDG_DATA_HOME` is ignored when unset, empty or non-absolute,
/// falling back to `~/.local/share/<name>`. A leading `~` is expanded so values set via systemd unit files or
/// Dockerfiles (which get no shell expansion) still work.
fn xdg_data_dir(env: &PathEnv, name: &str) -> PathBuf {
    if let Some(xdg) = non_empty(env.xdg_data_home.as_ref()) {
        let path = env.expand_tilde(xdg);
        if path.is_absolute() {
            return path.join(name);
        }
    }
    env.home.join(".local").join("share").join(name)
}

/// `%LOCALAPPDATA%` when set to an absolute path, else its usual place, `~\AppData\Local`.
fn local_app_data_dir(env: &PathEnv) -> PathBuf {
    non_empty(env.local_app_data.as_ref())
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| env.home.join("AppData").join("Local"))
}

/// haryx's own store root.
///
/// `HARYX_HOME` (non-empty, `~` expanded) on any OS; else macOS `~/Library/Application Support/HaryX`, Linux
/// (and other Unix) `$XDG_DATA_HOME/haryx`, Windows `%LOCALAPPDATA%\haryx`. The Windows store holds
/// per-machine credentials, so it lives in the local, not the roaming, profile.
pub fn store_root_in(env: &PathEnv) -> Result<PathBuf> {
    if let Some(home) = non_empty(env.haryx_home.as_ref()) {
        return Ok(env.expand_tilde(home));
    }
    match env.os {
        HostOs::MacOs => Ok(env
            .home
            .join("Library")
            .join("Application Support")
            .join(HARYX_MACOS_DIRNAME)),
        HostOs::Linux | HostOs::Other => Ok(xdg_data_dir(env, HARYX_XDG_DIRNAME)),
        HostOs::Windows => Ok(local_app_data_dir(env).join(HARYX_WINDOWS_DIRNAME)),
    }
}

/// Upstream's legacy cswap backup root: `~/.claude-swap-backup`.
pub fn cswap_legacy_store_root_in(env: &PathEnv) -> PathBuf {
    env.home.join(CSWAP_LEGACY_BACKUP_DIRNAME)
}

/// Where cswap keeps its store on this OS (upstream `get_backup_root`): Linux/WSL
/// `$XDG_DATA_HOME/claude-swap` (default `~/.local/share/claude-swap`), elsewhere `~/.claude-swap-backup`.
pub fn cswap_store_root_in(env: &PathEnv) -> PathBuf {
    match env.os {
        HostOs::Linux => xdg_data_dir(env, CSWAP_XDG_DIRNAME),
        HostOs::MacOs | HostOs::Windows | HostOs::Other => cswap_legacy_store_root_in(env),
    }
}

/// Every place a cswap store may be, most likely first: [`cswap_store_root_in`], then the legacy root when it
/// differs (a Linux cswap that never ran its legacy-to-XDG migration still keeps its data there).
pub fn cswap_store_candidates_in(env: &PathEnv) -> Vec<PathBuf> {
    let primary = cswap_store_root_in(env);
    let legacy = cswap_legacy_store_root_in(env);
    if primary == legacy {
        vec![primary]
    } else {
        vec![primary, legacy]
    }
}

// ---------------------------------------------------------------------------------------------------------
// Process-environment forms
// ---------------------------------------------------------------------------------------------------------

/// [`claude_config_home_in`] for the process environment.
pub fn claude_config_home() -> Result<PathBuf> {
    Ok(claude_config_home_in(&PathEnv::from_process()?))
}

/// [`global_config_path_in`] for the process environment.
pub fn global_config_path() -> Result<PathBuf> {
    Ok(global_config_path_in(&PathEnv::from_process()?))
}

/// [`default_claude_config_home_in`] for the process environment.
pub fn default_claude_config_home() -> Result<PathBuf> {
    Ok(default_claude_config_home_in(&PathEnv::from_process()?))
}

/// [`default_global_config_path_in`] for the process environment.
pub fn default_global_config_path() -> Result<PathBuf> {
    Ok(default_global_config_path_in(&PathEnv::from_process()?))
}

/// [`credentials_path_in`] for the process environment.
pub fn credentials_path() -> Result<PathBuf> {
    Ok(credentials_path_in(&PathEnv::from_process()?))
}

/// [`store_root_in`] for the process environment.
pub fn store_root() -> Result<PathBuf> {
    store_root_in(&PathEnv::from_process()?)
}

/// [`cswap_store_root_in`] for the process environment.
pub fn cswap_store_root() -> Result<PathBuf> {
    Ok(cswap_store_root_in(&PathEnv::from_process()?))
}

/// [`cswap_store_candidates_in`] for the process environment.
pub fn cswap_store_candidates() -> Result<Vec<PathBuf>> {
    Ok(cswap_store_candidates_in(&PathEnv::from_process()?))
}

/// Python's non-strict `Path.resolve()`: walk the components, resolving symlinks in every prefix that
/// exists and applying `..` to the resolved prefix; a missing tail is kept as written.
pub fn resolve_lenient(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => {
                out.push(other);
                if let Ok(canonical) = crate::fsutil::canonicalize(&out) {
                    out = canonical;
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn env_at(home: &Path) -> PathEnv {
        PathEnv::with_home(home)
    }

    #[test]
    fn default_config_home_is_dot_claude_in_home() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            claude_config_home_in(&env_at(tmp.path())),
            tmp.path().join(".claude")
        );
    }

    #[test]
    fn config_home_respects_env_var_and_ignores_empty() {
        let tmp = TempDir::new().unwrap();
        let mut env = env_at(tmp.path());
        env.claude_config_dir = Some("/custom/dir".into());
        assert_eq!(claude_config_home_in(&env), PathBuf::from("/custom/dir"));
        env.claude_config_dir = Some("".into());
        assert_eq!(claude_config_home_in(&env), tmp.path().join(".claude"));
    }

    #[test]
    fn default_global_config_is_homedir_claude_json() {
        let tmp = TempDir::new().unwrap();
        assert_eq!(
            global_config_path_in(&env_at(tmp.path())),
            tmp.path().join(".claude.json")
        );
    }

    #[test]
    fn ccd_set_returns_ccd_claude_json() {
        let tmp = TempDir::new().unwrap();
        let ccd = tmp.path().join("ccd");
        let mut env = env_at(tmp.path());
        env.claude_config_dir = Some(ccd.clone().into());
        assert_eq!(global_config_path_in(&env), ccd.join(".claude.json"));
    }

    #[test]
    fn legacy_config_json_takes_precedence() {
        let tmp = TempDir::new().unwrap();
        let legacy = tmp.path().join(".claude").join(".config.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();
        std::fs::write(&legacy, "{}").unwrap();
        std::fs::write(tmp.path().join(".claude.json"), "{}").unwrap();
        assert_eq!(global_config_path_in(&env_at(tmp.path())), legacy);
    }

    #[test]
    fn legacy_config_json_in_ccd_takes_precedence() {
        let tmp = TempDir::new().unwrap();
        let ccd = tmp.path().join("ccd");
        std::fs::create_dir_all(&ccd).unwrap();
        std::fs::write(ccd.join(".config.json"), "{\"a\":1}").unwrap();
        let mut env = env_at(tmp.path());
        env.claude_config_dir = Some(ccd.clone().into());
        assert_eq!(global_config_path_in(&env), ccd.join(".config.json"));
    }

    /// Claude Code 2.1.299: an empty legacy file wins only if writable and no `.claude.json` exists; a
    /// directory never wins.
    #[cfg(unix)]
    #[test]
    fn legacy_config_rule_matches_claude_code_2_1_299() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = TempDir::new().unwrap();
        let env = env_at(tmp.path());
        let legacy = tmp.path().join(".claude").join(".config.json");
        let global = tmp.path().join(".claude.json");
        std::fs::create_dir_all(legacy.parent().unwrap()).unwrap();

        std::fs::write(&legacy, "").unwrap();
        assert_eq!(
            global_config_path_in(&env),
            legacy,
            "empty, writable, no global"
        );

        std::fs::write(&global, "{}").unwrap();
        assert_eq!(
            global_config_path_in(&env),
            global,
            "empty legacy loses to an existing global"
        );
        std::fs::remove_file(&global).unwrap();

        std::fs::set_permissions(&legacy, std::fs::Permissions::from_mode(0o444)).unwrap();
        assert_eq!(
            global_config_path_in(&env),
            global,
            "empty read-only legacy loses"
        );

        std::fs::remove_file(&legacy).unwrap();
        std::fs::create_dir(&legacy).unwrap();
        assert_eq!(
            global_config_path_in(&env),
            global,
            "a directory never wins"
        );
    }

    #[test]
    fn default_global_config_ignores_ccd() {
        let tmp = TempDir::new().unwrap();
        let mut env = env_at(tmp.path());
        env.claude_config_dir = Some("/elsewhere".into());
        assert_eq!(
            default_global_config_path_in(&env),
            tmp.path().join(".claude.json")
        );
        assert_eq!(
            default_claude_config_home_in(&env),
            tmp.path().join(".claude")
        );
    }

    #[test]
    fn credentials_path_rules() {
        let tmp = TempDir::new().unwrap();
        let mut env = env_at(tmp.path());
        assert_eq!(
            credentials_path_in(&env),
            tmp.path().join(".claude/.credentials.json")
        );

        env.claude_config_dir = Some("/ccd".into());
        assert_eq!(
            credentials_path_in(&env),
            PathBuf::from("/ccd/.credentials.json")
        );

        env.secure_storage_dir = Some("/secure".into());
        assert_eq!(
            credentials_path_in(&env),
            PathBuf::from("/secure/.credentials.json")
        );

        // Empty selects the default profile, even with CLAUDE_CONFIG_DIR set.
        env.secure_storage_dir = Some("".into());
        assert_eq!(
            credentials_path_in(&env),
            tmp.path().join(".claude/.credentials.json")
        );
    }

    fn linux_env(home: &Path) -> PathEnv {
        PathEnv {
            os: HostOs::Linux,
            ..env_at(home)
        }
    }

    #[test]
    fn linux_store_default_is_xdg_data_home() {
        let env = linux_env(Path::new("/home/u"));
        assert_eq!(
            store_root_in(&env).unwrap(),
            PathBuf::from("/home/u/.local/share/haryx")
        );
        assert_eq!(
            cswap_store_root_in(&env),
            PathBuf::from("/home/u/.local/share/claude-swap")
        );
    }

    // `/data` is absolute only on Unix.
    #[cfg(unix)]
    #[test]
    fn linux_store_respects_xdg_data_home() {
        let mut env = linux_env(Path::new("/home/u"));
        env.xdg_data_home = Some("/data".into());
        assert_eq!(store_root_in(&env).unwrap(), PathBuf::from("/data/haryx"));
        assert_eq!(
            cswap_store_root_in(&env),
            PathBuf::from("/data/claude-swap")
        );
    }

    #[test]
    fn linux_store_ignores_empty_and_relative_xdg_data_home() {
        let mut env = linux_env(Path::new("/home/u"));
        for bad in ["", "relative/data"] {
            env.xdg_data_home = Some(bad.into());
            assert_eq!(
                store_root_in(&env).unwrap(),
                PathBuf::from("/home/u/.local/share/haryx")
            );
        }
    }

    #[cfg(unix)]
    #[test]
    fn linux_store_expands_tilde_in_xdg_data_home() {
        let mut env = linux_env(Path::new("/home/u"));
        env.xdg_data_home = Some("~/data".into());
        assert_eq!(
            store_root_in(&env).unwrap(),
            PathBuf::from("/home/u/data/haryx")
        );
    }

    #[test]
    fn macos_store_is_application_support() {
        let env = PathEnv {
            os: HostOs::MacOs,
            ..env_at(Path::new("/Users/u"))
        };
        assert_eq!(
            store_root_in(&env).unwrap(),
            PathBuf::from("/Users/u/Library/Application Support/HaryX")
        );
        assert_eq!(
            cswap_store_root_in(&env),
            PathBuf::from("/Users/u/.claude-swap-backup")
        );
        assert_eq!(
            cswap_store_candidates_in(&env),
            vec![PathBuf::from("/Users/u/.claude-swap-backup")]
        );
    }

    #[test]
    fn haryx_home_overrides_on_every_os() {
        for os in [HostOs::MacOs, HostOs::Linux, HostOs::Windows, HostOs::Other] {
            let mut env = PathEnv {
                os,
                ..env_at(Path::new("/h"))
            };
            env.haryx_home = Some("/portable/store".into());
            assert_eq!(
                store_root_in(&env).unwrap(),
                PathBuf::from("/portable/store")
            );
            env.haryx_home = Some("~/s".into());
            assert_eq!(store_root_in(&env).unwrap(), PathBuf::from("/h/s"));
            env.haryx_home = Some("".into());
            assert!(store_root_in(&env).is_ok());
        }
    }

    #[test]
    fn windows_store_root_is_under_local_app_data() {
        let env = PathEnv {
            os: HostOs::Windows,
            local_app_data: Some("/h/AppData/Local".into()),
            ..env_at(Path::new("/h"))
        };
        assert_eq!(
            store_root_in(&env).unwrap(),
            PathBuf::from("/h/AppData/Local/haryx")
        );
    }

    #[test]
    fn windows_store_root_falls_back_when_local_app_data_is_missing_or_relative() {
        for value in [None, Some(""), Some("relative/dir")] {
            let env = PathEnv {
                os: HostOs::Windows,
                local_app_data: value.map(Into::into),
                ..env_at(Path::new("/h"))
            };
            assert_eq!(
                store_root_in(&env).unwrap(),
                PathBuf::from("/h")
                    .join("AppData")
                    .join("Local")
                    .join("haryx"),
                "LOCALAPPDATA = {value:?}"
            );
        }
    }

    #[test]
    fn linux_cswap_candidates_include_legacy() {
        let env = linux_env(Path::new("/home/u"));
        assert_eq!(
            cswap_store_candidates_in(&env),
            vec![
                PathBuf::from("/home/u/.local/share/claude-swap"),
                PathBuf::from("/home/u/.claude-swap-backup"),
            ]
        );
    }
}
