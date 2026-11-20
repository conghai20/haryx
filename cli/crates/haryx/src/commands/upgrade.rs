//! `haryx upgrade`: upgrade through the package manager that installed this binary
//! (`run_self_upgrade`). Upstream knows uv and pipx; haryx ships through Homebrew, cargo, pip/uv
//! wrappers and npm, so detection looks at where the running executable lives.

use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Command;

use haryx_core::{PROG, printer};

use super::CommandError;

/// Exit code after Ctrl-C.
const INTERRUPTED_EXIT: u8 = 130;

/// How this binary was installed, with the command that upgrades it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    Homebrew,
    Cargo,
    Pipx,
    Uv,
    Npm,
}

impl InstallMethod {
    pub fn command(self) -> &'static [&'static str] {
        match self {
            InstallMethod::Homebrew => &["brew", "upgrade", "haryx"],
            InstallMethod::Cargo => &["cargo", "install", "--locked", "haryx"],
            InstallMethod::Pipx => &["pipx", "upgrade", "haryx"],
            InstallMethod::Uv => &["uv", "tool", "upgrade", "haryx"],
            InstallMethod::Npm => &["npm", "install", "--global", "haryx@latest"],
        }
    }

    fn name(self) -> &'static str {
        self.command()[0]
    }
}

/// Detect the install method from the executable's (symlink-resolved) path.
pub fn detect_install_method(exe: &Path) -> Option<InstallMethod> {
    let path = exe.to_string_lossy();
    if path.contains("/Cellar/") || path.contains("/linuxbrew/") || path.contains("/homebrew/") {
        Some(InstallMethod::Homebrew)
    } else if path.contains("/.cargo/bin/") {
        Some(InstallMethod::Cargo)
    } else if path.contains("/pipx/venvs/") {
        Some(InstallMethod::Pipx)
    } else if path.contains("/uv/tools/") {
        Some(InstallMethod::Uv)
    } else if path.contains("/node_modules/") {
        Some(InstallMethod::Npm)
    } else {
        None
    }
}

/// The version the binary at `exe` reports (`haryx 0.1.0 (...)` → `0.1.0`).
fn version_on_disk(exe: &Path) -> Option<String> {
    let out = Command::new(exe).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    text.split_whitespace().nth(1).map(str::to_owned)
}

pub fn run() -> Result<(), CommandError> {
    let before = env!("CARGO_PKG_VERSION");
    let exe = std::env::current_exe()
        .and_then(|p| p.canonicalize())
        .unwrap_or_default();
    let Some(method) = detect_install_method(&exe) else {
        printer::error(&format!(
            "Could not detect install method (looked for Homebrew, cargo, pipx, uv tool, npm).\n  \
             executable: {}\nTo upgrade manually, run one of:\n  brew upgrade {PROG}\n  cargo \
             install --locked {PROG}\n  pipx upgrade {PROG}\n  uv tool upgrade {PROG}\n  npm install \
             --global {PROG}@latest\nIf you built from source, use `git pull` and rebuild instead.",
            exe.display()
        ));
        return Err(CommandError::silent(1));
    };
    let cmd = method.command();
    let status = match Command::new(cmd[0]).args(&cmd[1..]).status() {
        Ok(status) => status,
        Err(_) => {
            printer::error(&format!(
                "Detected {} install but `{}` is not on PATH. Run the upgrade manually from a \
                 shell where it is available.",
                method.name(),
                cmd[0]
            ));
            return Err(CommandError::silent(1));
        }
    };
    if status.signal() == Some(libc::SIGINT) {
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
    match version_on_disk(&exe) {
        Some(after) if after != before => {
            println!(
                "{}",
                printer::accent(&format!("{PROG} upgraded: {before} -> {after}"))
            );
        }
        Some(after) => println!("{PROG} is still at {after}."),
        None => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_each_package_manager_from_the_path() {
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
            ("/Users/u/src/haryx/cli/target/release/haryx", None),
        ] {
            assert_eq!(detect_install_method(Path::new(path)), method, "{path}");
        }
    }

    #[test]
    fn commands_name_the_haryx_package() {
        assert_eq!(
            InstallMethod::Uv.command(),
            ["uv", "tool", "upgrade", "haryx"]
        );
        assert_eq!(InstallMethod::Homebrew.name(), "brew");
    }
}
