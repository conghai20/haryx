//! Whether Claude Code is set up for haryx: the `claude` command can be found, and Claude Code has
//! signed in at least once on the default profile. Every `ai` command that signs in, runs or reads
//! an account depends on both, so `haryx ai doctor` and the apps report them before anything fails.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::spawn;

/// Anthropic's installer for Claude Code on this platform.
pub const INSTALL_COMMAND: &str = if cfg!(windows) {
    "irm https://claude.ai/install.ps1 | iex"
} else {
    "curl -fsSL https://claude.ai/install.sh | bash"
};

/// What stands between this machine and a working `haryx ai`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    /// No `claude` on `PATH` or in the usual install folders.
    ClaudeMissing,
    /// `claude` exists, but Claude Code has no signed-in account on the default profile.
    NotSignedIn,
}

impl Problem {
    /// Stable identifier for scripts and the apps.
    pub fn code(self) -> &'static str {
        match self {
            Problem::ClaudeMissing => "claude_missing",
            Problem::NotSignedIn => "not_signed_in",
        }
    }

    pub fn message(self) -> &'static str {
        match self {
            Problem::ClaudeMissing => "Claude Code is not installed, or haryx cannot find it.",
            Problem::NotSignedIn => "Claude Code has not signed in yet.",
        }
    }

    /// The one thing to do about it.
    pub fn fix(self) -> String {
        match self {
            Problem::ClaudeMissing => format!(
                "Install it with `{INSTALL_COMMAND}`. If it is already installed, make sure \
                 `claude` runs in a new terminal."
            ),
            Problem::NotSignedIn => "Run `claude` in a terminal and sign in once.".into(),
        }
    }
}

/// The two checks, as found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readiness {
    pub claude: Option<PathBuf>,
    pub signed_in_email: Option<String>,
}

impl Readiness {
    /// Problems in the order they must be fixed (signing in needs `claude`).
    pub fn problems(&self) -> Vec<Problem> {
        [
            self.claude.is_none().then_some(Problem::ClaudeMissing),
            self.signed_in_email
                .is_none()
                .then_some(Problem::NotSignedIn),
        ]
        .into_iter()
        .flatten()
        .collect()
    }

    pub fn is_ready(&self) -> bool {
        self.problems().is_empty()
    }
}

/// Find `claude`: on `path_var`, then in the folders installers use. Apps started from the desktop
/// often get a shorter `PATH` than a terminal, so the fallbacks matter there.
pub fn find_claude(
    path_var: Option<&OsStr>,
    home: &Path,
    app_data: Option<&Path>,
) -> Option<PathBuf> {
    let on_path = spawn::resolve(OsStr::new("claude"), path_var).filter(|p| p.is_file());
    on_path.or_else(|| {
        install_dirs(home, app_data).into_iter().find_map(|dir| {
            spawn::resolve(OsStr::new("claude"), Some(dir.as_os_str())).filter(|p| p.is_file())
        })
    })
}

/// `claude` for this process: its `PATH`, its home, and on Windows `%APPDATA%`.
pub fn claude_bin() -> crate::Result<PathBuf> {
    let home = crate::paths::home_dir()?;
    let app_data = std::env::var_os("APPDATA").map(PathBuf::from);
    find_claude(
        std::env::var_os("PATH").as_deref(),
        &home,
        app_data.as_deref(),
    )
    .ok_or_else(|| {
        crate::Error::Session(format!(
            "{} {}",
            Problem::ClaudeMissing.message(),
            Problem::ClaudeMissing.fix()
        ))
    })
}

/// Where Claude Code's installers put `claude`, beyond `PATH`.
fn install_dirs(home: &Path, app_data: Option<&Path>) -> Vec<PathBuf> {
    // The native installer (every platform), and the older per-user "local" install.
    let mut dirs = vec![
        home.join(".local").join("bin"),
        home.join(".claude").join("local"),
    ];
    if cfg!(windows) {
        // `npm install -g` puts a `claude.cmd` shim in the npm prefix.
        let roaming =
            app_data.map_or_else(|| home.join("AppData").join("Roaming"), Path::to_path_buf);
        dirs.push(roaming.join("npm"));
    } else {
        dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    }
    dirs
}

#[cfg(test)]
mod tests {
    use super::*;

    fn executable(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    fn exe_name() -> &'static str {
        if cfg!(windows) {
            "claude.exe"
        } else {
            "claude"
        }
    }

    #[test]
    fn finds_claude_on_path_first() {
        let tmp = tempfile::tempdir().unwrap();
        let on_path = tmp.path().join("bin").join(exe_name());
        let native = tmp.path().join("home/.local/bin").join(exe_name());
        executable(&on_path);
        executable(&native);
        let found = find_claude(
            Some(on_path.parent().unwrap().as_os_str()),
            &tmp.path().join("home"),
            None,
        );
        assert_eq!(
            found
                .as_deref()
                .map(std::fs::canonicalize)
                .transpose()
                .unwrap(),
            Some(std::fs::canonicalize(&on_path).unwrap())
        );
    }

    #[test]
    fn falls_back_to_the_native_installer_folder_when_path_misses_it() {
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        let native = home.join(".local/bin").join(exe_name());
        executable(&native);
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let found = find_claude(Some(empty.as_os_str()), &home, None).unwrap();
        assert_eq!(
            std::fs::canonicalize(found).unwrap(),
            std::fs::canonicalize(native).unwrap()
        );
    }

    #[test]
    fn reports_missing_claude_when_nowhere() {
        let tmp = tempfile::tempdir().unwrap();
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        // The home has no installs; the system folders are checked too, so only assert on a
        // machine without a system-wide claude.
        let found = find_claude(Some(empty.as_os_str()), &tmp.path().join("home"), None);
        let system_wide = ["/opt/homebrew/bin/claude", "/usr/local/bin/claude"]
            .iter()
            .any(|p| Path::new(p).is_file());
        if !system_wide {
            assert_eq!(found, None);
        }
    }

    #[test]
    fn problems_come_in_fix_order() {
        let none = Readiness {
            claude: None,
            signed_in_email: None,
        };
        assert_eq!(
            none.problems(),
            [Problem::ClaudeMissing, Problem::NotSignedIn]
        );
        assert!(!none.is_ready());
        let unsigned = Readiness {
            claude: Some("/x/claude".into()),
            signed_in_email: None,
        };
        assert_eq!(unsigned.problems(), [Problem::NotSignedIn]);
        let ready = Readiness {
            claude: Some("/x/claude".into()),
            signed_in_email: Some("a@x.com".into()),
        };
        assert!(ready.is_ready());
    }

    #[test]
    fn every_problem_names_its_fix() {
        assert!(Problem::ClaudeMissing.fix().contains(INSTALL_COMMAND));
        assert!(Problem::NotSignedIn.fix().contains("`claude`"));
        assert_ne!(Problem::ClaudeMissing.code(), Problem::NotSignedIn.code());
    }
}
