//! `purge`: upstream `purge` and `_quarantined_history_files`. Removes every piece of haryx data: the
//! store directory, the slot backups (Keychain items and files) and session profiles with their Keychain
//! entries. The live Claude Code login is never touched.

use std::path::Path;

use serde_json::Value;

use super::Switcher;
use crate::credentials::delete_macos_keychain_entry;
use crate::error::{Error, Result};
use crate::paths::HostOs;
use crate::printer::{accent, dimmed, warning};
use crate::session;

/// Where history migration parks colliding transcripts (upstream `session.QUARANTINE_DIRNAME`).
pub const QUARANTINE_DIRNAME: &str = "history-conflicts";

fn count_files(root: &Path) -> usize {
    let Ok(entries) = std::fs::read_dir(root) else {
        return 0;
    };
    entries
        .flatten()
        .map(|e| {
            let path = e.path();
            if path.is_dir() {
                count_files(&path)
            } else {
                usize::from(path.is_file())
            }
        })
        .sum()
}

impl Switcher {
    fn quarantined_history_files(&self) -> usize {
        let root = self.backup_dir.join(QUARANTINE_DIRNAME);
        if root.is_dir() { count_files(&root) } else { 0 }
    }

    /// `purge`: confirm, then remove all haryx data. Refuses while a session-mode instance is live.
    pub fn purge(&self) -> Result<()> {
        self.refuse_session_shell()?;
        let sessions_root = self.backup_dir.join("sessions");
        let session_dirs: Vec<std::path::PathBuf> = std::fs::read_dir(&sessions_root)
            .map(|rd| {
                rd.flatten()
                    .map(|e| e.path())
                    .filter(|p| p.is_dir())
                    .collect()
            })
            .unwrap_or_default();
        let name = |d: &Path| {
            d.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default()
        };
        let mut live = Vec::new();
        let mut unreadable = Vec::new();
        for d in &session_dirs {
            let (sessions, bad) = session::scan_live_sessions(d);
            if !sessions.is_empty() {
                let pids: Vec<String> = sessions.iter().map(|s| s.pid.to_string()).collect();
                live.push(format!("{} (PID {})", name(d), pids.join(", ")));
            } else if bad > 0 {
                unreadable.push(format!("{} ({bad} record(s))", name(d)));
            }
        }
        if !live.is_empty() {
            return Err(Error::Session(format!(
                "Live session-mode Claude instance(s) found: {}. Exit them first, then retry --purge.",
                live.join("; ")
            )));
        }
        if !unreadable.is_empty() {
            return Err(Error::Session(format!(
                "Session records that could not be read: {}. Whether a Claude instance is live cannot be \
                 determined, and purging would pull a live profile out from under it. Repair or remove them, \
                 then retry --purge.",
                unreadable.join("; ")
            )));
        }

        warning("This will remove ALL haryx data from your system:");
        println!("  - Store directory: {}", self.backup_dir.display());
        if self.env.os == HostOs::MacOs {
            println!("  - All stored account credentials (macOS Keychain and/or files)");
        } else {
            println!("  - All stored account credential files");
        }
        if !session_dirs.is_empty() {
            println!("  - All session profiles and their Keychain entries");
        }
        let quarantined = self.quarantined_history_files();
        if quarantined > 0 {
            warning(&format!(
                "  - {quarantined} quarantined conversation transcript(s) in {}",
                self.backup_dir.join(QUARANTINE_DIRNAME).display()
            ));
            println!(
                "{}",
                dimmed("    These are the ONLY remaining copy of the losing half of each collided")
            );
            println!(
                "{}",
                dimmed(
                    "    session — history migration parked them here rather than deleting them."
                )
            );
            println!(
                "{}",
                dimmed("    Copy them somewhere else first if you want them.")
            );
        }
        println!();
        println!(
            "{}",
            dimmed("Note: This does NOT affect your current Claude Code login.")
        );
        println!();
        let answer = super::add::prompt("Are you sure you want to purge all data? [y/N] ")
            .unwrap_or_default();
        if answer.to_lowercase() != "y" {
            println!("{}", dimmed("Cancelled"));
            return Ok(());
        }

        let mut removed = Vec::new();
        if let Some(data) = self.get_sequence_data().ok().flatten()
            && let Some(accounts) = data.get("accounts").and_then(Value::as_object)
        {
            for (num, info) in accounts {
                let email = info
                    .get("email")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let file = self.store.backup_enc_path(num, email);
                let had_file = file.exists();
                self.store.delete_account_credentials(num, email);
                if had_file {
                    removed.push(format!(
                        "Credential file: {}",
                        file.file_name()
                            .map(|n| n.to_string_lossy().into_owned())
                            .unwrap_or_default()
                    ));
                }
                if self.env.os == HostOs::MacOs {
                    removed.push(format!("Credential: account-{num}-{email}"));
                }
            }
        }
        if !session_dirs.is_empty() {
            for d in &session_dirs {
                delete_macos_keychain_entry(
                    self.keychain.as_ref(),
                    self.env.os,
                    d,
                    &self.keychain_account,
                );
            }
            let names: Vec<String> = session_dirs.iter().map(|d| name(d)).collect();
            removed.push(format!("Session profiles: {}", names.join(", ")));
        }
        if self.backup_dir.exists() {
            std::fs::remove_dir_all(&self.backup_dir)?;
            removed.push(format!("Directory: {}", self.backup_dir.display()));
        }
        if removed.is_empty() {
            println!("\n{}", dimmed("No haryx data found to remove."));
        } else {
            println!("\n{}", accent("Removed:"));
            for item in removed {
                println!("  {} {item}", dimmed("-"));
            }
        }
        println!("\n{}", accent("Purge complete."));
        Ok(())
    }
}
