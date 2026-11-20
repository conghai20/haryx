//! The root guard every store-touching command runs first (upstream `cli._guard_root` and
//! `ClaudeAccountSwitcher._is_running_in_container`).
//!
//! Running as root would leave root-owned files in the user's store; inside a container root is the
//! normal user, so it is allowed there.

use std::path::Path;

use super::CommandError;

const ROOT_MESSAGE: &str = "Error: Do not run this script as root (unless running in a container)";

/// Refuse to run as root outside a container (exit 1).
// TODO(switcher): share the container check with the switcher once it is ported.
pub fn refuse_root() -> Result<(), CommandError> {
    if is_root() && !is_running_in_container() {
        return Err(CommandError {
            message: ROOT_MESSAGE.to_owned(),
            exit_code: 1,
        });
    }
    Ok(())
}

#[cfg(unix)]
fn is_root() -> bool {
    // SAFETY: geteuid takes no arguments, cannot fail and touches no memory.
    unsafe { libc::geteuid() == 0 }
}

#[cfg(not(unix))]
fn is_root() -> bool {
    false
}

/// `CONTAINER`/`container` set, `/.dockerenv` present, or PID 1's cgroup naming a container runtime.
fn is_running_in_container() -> bool {
    let env_set = |name: &str| std::env::var_os(name).is_some_and(|v| !v.is_empty());
    if env_set("CONTAINER") || env_set("container") {
        return true;
    }
    if Path::new("/.dockerenv").exists() {
        return true;
    }
    std::fs::read_to_string("/proc/1/cgroup").is_ok_and(|content| {
        ["docker", "lxc", "containerd", "kubepods"]
            .iter()
            .any(|marker| content.contains(marker))
    })
}
