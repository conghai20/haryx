//! The user LaunchAgent that runs `haryx ai wake run --once` every minute (macOS).

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::error::{Error, Result};
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

/// Never reached: every caller runs `require_macos` first. Kept so the crate builds on Windows.
#[cfg(not(unix))]
fn domain() -> String {
    String::new()
}

fn launchctl(args: &[&str]) -> Result<std::process::Output> {
    Command::new("/bin/launchctl")
        .args(args)
        .output()
        .map_err(|e| Error::General(format!("cannot run launchctl: {e}")))
}

fn require_macos() -> Result<()> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(Error::General(
            "the wake agent is macOS-only for now; run `haryx ai wake run --once` from a timer"
                .into(),
        ))
    }
}

pub fn is_installed(home: &Path) -> bool {
    plist_path(home).is_file()
}

/// Write the plist and (re)load it. Returns the plist path.
pub fn install(home: &Path, exe: &Path, log: &Path) -> Result<PathBuf> {
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
