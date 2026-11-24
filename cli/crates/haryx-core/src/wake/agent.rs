//! What runs `haryx ai wake run --once` every minute: a user LaunchAgent on macOS, a per-user Task
//! Scheduler task on Windows ([`super::task`]).

use std::path::{Path, PathBuf};
#[cfg(not(windows))]
use std::process::Command;

#[cfg(not(windows))]
use crate::error::Error;
use crate::error::Result;
#[cfg(not(windows))]
use crate::fsutil::{WriteOptions, atomic_write_text};

pub const LABEL: &str = "dev.haryx.wake";
/// Seconds between ticks. launchd folds intervals missed during sleep into one run after wake.
pub const INTERVAL_S: u32 = 60;
/// Environment variables the agent carries over so it reads the same store as the shell.
const CARRIED_ENV: &[&str] = &["HARYX_HOME", "CLAUDE_CONFIG_DIR", "XDG_DATA_HOME"];

pub fn plist_path(home: &Path) -> PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

/// XML-escape element text, dropping characters XML 1.0 cannot carry (they would make launchd
/// reject the plist).
fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// The plist text for `exe`, carrying `env` pairs.
pub fn plist(exe: &Path, log: &Path, env: &[(String, String)]) -> String {
    let env_block = if env.is_empty() {
        String::new()
    } else {
        let pairs: String = env
            .iter()
            .map(|(k, v)| {
                format!(
                    "    <key>{}</key><string>{}</string>\n",
                    escape(k),
                    escape(v)
                )
            })
            .collect();
        format!("  <key>EnvironmentVariables</key>\n  <dict>\n{pairs}  </dict>\n")
    };
    let exe = escape(&exe.to_string_lossy());
    let log = escape(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
    <string>{exe}</string>
    <string>ai</string>
    <string>wake</string>
    <string>run</string>
    <string>--once</string>
  </array>
  <key>StartInterval</key><integer>{INTERVAL_S}</integer>
  <key>RunAtLoad</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>LowPriorityIO</key><true/>
  <key>StandardErrorPath</key><string>{log}</string>
{env_block}</dict>
</plist>
"#
    )
}

fn carried_env() -> Vec<(String, String)> {
    CARRIED_ENV
        .iter()
        .filter_map(|k| std::env::var(k).ok().map(|v| ((*k).to_string(), v)))
        .collect()
}

#[cfg(unix)]
fn domain() -> String {
    // SAFETY: getuid has no preconditions and cannot fail.
    let uid = unsafe { libc::getuid() };
    format!("gui/{uid}")
}

/// Never reached: every caller runs `require_macos` first.
#[cfg(not(any(unix, windows)))]
fn domain() -> String {
    String::new()
}

#[cfg(not(windows))]
fn launchctl(args: &[&str]) -> Result<std::process::Output> {
    Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| Error::General(format!("cannot run launchctl: {e}")))
}

#[cfg(not(windows))]
fn require_macos() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(Error::General(
            "the wake agent runs on macOS and Windows; elsewhere run `haryx ai wake run --once` from a timer"
                .into(),
        ))
    }
}

pub fn is_installed(home: &Path) -> bool {
    #[cfg(windows)]
    {
        let _ = home;
        windows::is_installed()
    }
    #[cfg(not(windows))]
    plist_path(home).is_file()
}

/// Write the plist and (re)load it (macOS), or register the scheduled task (Windows). Returns the file
/// that defines it.
pub fn install(home: &Path, exe: &Path, log: &Path) -> Result<PathBuf> {
    #[cfg(windows)]
    {
        let _ = home;
        windows::install(exe, log)
    }
    #[cfg(not(windows))]
    launchd_install(home, exe, log)
}

#[cfg(not(windows))]
fn launchd_install(home: &Path, exe: &Path, log: &Path) -> Result<PathBuf> {
    require_macos()?;
    let path = plist_path(home);
    let dir = path.parent().unwrap_or(home);
    std::fs::create_dir_all(dir)
        .map_err(|e| Error::General(format!("cannot create {}: {e}", dir.display())))?;
    atomic_write_text(
        &path,
        &plist(exe, log, &carried_env()),
        WriteOptions {
            mode: 0o644,
            ..WriteOptions::default()
        },
    )
    .map_err(|e| Error::General(format!("cannot write {}: {e}", path.display())))?;
    let target = format!("{}/{LABEL}", domain());
    let _ = launchctl(&["bootout", &target]);
    let out = launchctl(&["bootstrap", &domain(), &path.to_string_lossy()])?;
    if !out.status.success() {
        return Err(Error::General(format!(
            "launchctl bootstrap failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(path)
}

/// Unload and remove the agent. Returns whether a plist was there.
pub fn uninstall(home: &Path) -> Result<bool> {
    #[cfg(windows)]
    {
        let _ = home;
        windows::uninstall()
    }
    #[cfg(not(windows))]
    launchd_uninstall(home)
}

#[cfg(not(windows))]
fn launchd_uninstall(home: &Path) -> Result<bool> {
    require_macos()?;
    let path = plist_path(home);
    let target = format!("{}/{LABEL}", domain());
    let _ = launchctl(&["bootout", &target]);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(Error::General(format!(
            "cannot remove {}: {e}",
            path.display()
        ))),
    }
}

#[cfg(windows)]
mod windows {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use super::super::task::{TASK_NAME, WINDOWLESS_EXE, task_xml, utf16_bytes};
    use super::{INTERVAL_S, carried_env};
    use crate::error::{Error, Result};
    use crate::fsutil::{WriteOptions, atomic_write_bytes};

    fn schtasks(args: &[&str]) -> Result<std::process::Output> {
        Command::new("schtasks.exe")
            .args(args)
            .output()
            .map_err(|e| Error::General(format!("cannot run schtasks: {e}")))
    }

    pub fn is_installed() -> bool {
        schtasks(&["/Query", "/TN", TASK_NAME]).is_ok_and(|o| o.status.success())
    }

    /// `haryxw.exe` beside `exe` when it is there, so no console window opens each minute.
    fn tick_exe(exe: &Path) -> PathBuf {
        let windowless = exe.with_file_name(WINDOWLESS_EXE);
        if windowless.is_file() {
            windowless
        } else {
            log::warn!(
                "{} not found next to {}; the wake task runs the console build",
                WINDOWLESS_EXE,
                exe.display()
            );
            exe.to_path_buf()
        }
    }

    pub fn install(exe: &Path, log: &Path) -> Result<PathBuf> {
        let dir = log.parent().unwrap_or(Path::new("."));
        std::fs::create_dir_all(dir)
            .map_err(|e| Error::General(format!("cannot create {}: {e}", dir.display())))?;
        let start = jiff::Zoned::now().strftime("%Y-%m-%dT%H:%M:00").to_string();
        let xml = task_xml(&tick_exe(exe), &carried_env(), INTERVAL_S, &start);
        let path = dir.join("wake-task.xml");
        atomic_write_bytes(&path, &utf16_bytes(&xml), WriteOptions::default())
            .map_err(|e| Error::General(format!("cannot write {}: {e}", path.display())))?;
        let xml_arg = path.to_string_lossy();
        let out = schtasks(&["/Create", "/TN", TASK_NAME, "/XML", &xml_arg, "/F"])?;
        if !out.status.success() {
            return Err(Error::General(format!(
                "schtasks /Create failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(path)
    }

    pub fn uninstall() -> Result<bool> {
        if !is_installed() {
            return Ok(false);
        }
        let out = schtasks(&["/Delete", "/TN", TASK_NAME, "/F"])?;
        if !out.status.success() {
            return Err(Error::General(format!(
                "schtasks /Delete failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            )));
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plist_runs_once_every_minute() {
        let text = plist(Path::new("/opt/haryx"), Path::new("/tmp/w.log"), &[]);
        assert!(text.contains("<string>/opt/haryx</string>"));
        assert!(text.contains("<string>--once</string>"));
        assert!(text.contains("<integer>60</integer>"));
        assert!(!text.contains("EnvironmentVariables"));
    }

    #[test]
    fn plist_escapes_and_carries_env() {
        let env = vec![("HARYX_HOME".to_string(), "/a&b".to_string())];
        let text = plist(Path::new("/x<y>\u{1}"), Path::new("/l"), &env);
        assert!(text.contains("/x&lt;y&gt;"));
        assert!(text.contains("<key>HARYX_HOME</key><string>/a&amp;b</string>"));
    }

    #[test]
    fn plist_path_is_under_launch_agents() {
        assert_eq!(
            plist_path(Path::new("/Users/u")),
            PathBuf::from("/Users/u/Library/LaunchAgents/dev.haryx.wake.plist")
        );
    }
}
