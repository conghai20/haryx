//! Opening a terminal on Windows: Windows Terminal, PowerShell 7, Windows PowerShell, Command Prompt or
//! Git Bash, in a folder, optionally running `haryx run <slot>` (Claude as that account, in that
//! terminal only) or plain `claude`.
//!
//! The command text a shell parses only ever holds the haryx path (quoted for that shell) and a slot
//! number, never free text, so nothing the user typed elsewhere can be read as shell syntax. The folder is
//! the process's working directory, not part of the command.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
#[allow(clippy::enum_variant_names)] // "Windows Terminal" is the product's name.
pub enum Terminal {
    WindowsTerminal,
    Pwsh,
    Powershell,
    Cmd,
    GitBash,
}

impl Terminal {
    pub const ALL: [Terminal; 5] = [
        Terminal::WindowsTerminal,
        Terminal::Pwsh,
        Terminal::Powershell,
        Terminal::Cmd,
        Terminal::GitBash,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Terminal::WindowsTerminal => "Windows Terminal",
            Terminal::Pwsh => "PowerShell 7",
            Terminal::Powershell => "Windows PowerShell",
            Terminal::Cmd => "Command Prompt",
            Terminal::GitBash => "Git Bash",
        }
    }
}

/// What to run in the new terminal.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Job {
    /// Just a shell.
    Shell,
    /// `claude` as whichever account is active.
    Claude,
    /// `haryx run <slot>`: Claude as that account in this terminal only.
    RunAs { slot: u32 },
    /// `haryx run`: the account a mapped folder points at.
    RunMapped,
    /// `haryx ai login [slot]`: sign in (again) in the browser Claude Code opens.
    Login { slot: Option<u32> },
    /// `haryx ai sessions continue <pid> --to <slot|best>`: end a session that ran out and resume
    /// its conversation on another account.
    Continue { pid: u32, to: Option<u32> },
    /// Anthropic's Claude Code installer for Windows (what `haryx ai doctor` reports). Fixed here:
    /// the UI names the job, never the command.
    InstallClaude,
}

/// `haryx ai doctor`'s Windows install command, run by PowerShell from whichever shell opens.
const INSTALL_CLAUDE: &str = "irm https://claude.ai/install.ps1 | iex";

/// The program files each terminal is started through.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Installed {
    pub wt: Option<PathBuf>,
    pub pwsh: Option<PathBuf>,
    pub powershell: Option<PathBuf>,
    pub cmd: Option<PathBuf>,
    pub git_bash: Option<PathBuf>,
}

impl Installed {
    pub fn path(&self, t: Terminal) -> Option<&Path> {
        match t {
            Terminal::WindowsTerminal => self.wt.as_deref(),
            Terminal::Pwsh => self.pwsh.as_deref(),
            Terminal::Powershell => self.powershell.as_deref(),
            Terminal::Cmd => self.cmd.as_deref(),
            Terminal::GitBash => self.git_bash.as_deref(),
        }
    }

    pub fn available(&self) -> Vec<Terminal> {
        Terminal::ALL
            .into_iter()
            .filter(|t| self.path(*t).is_some())
            .collect()
    }

    /// The shell a Windows Terminal tab runs: PowerShell 7 when present.
    fn tab_shell(&self) -> Option<(Terminal, &Path)> {
        [Terminal::Pwsh, Terminal::Powershell, Terminal::Cmd]
            .into_iter()
            .find_map(|t| self.path(t).map(|p| (t, p)))
    }
}

/// `root` joined with each part, so the path is built the same way on every OS (tests run on all).
fn under(root: &Path, parts: &[&str]) -> PathBuf {
    parts
        .iter()
        .fold(root.to_path_buf(), |p, part| p.join(part))
}

/// Find the terminals on this machine. `path_var` is `PATH`; the other roots are the usual install
/// folders (`%LOCALAPPDATA%`, `%ProgramFiles%`, `%SystemRoot%`).
pub fn detect(
    path_var: Option<&std::ffi::OsStr>,
    local_app_data: Option<&Path>,
    program_files: Option<&Path>,
    system_root: Option<&Path>,
) -> Installed {
    let on_path = |name: &str| -> Option<PathBuf> {
        path_var.and_then(|p| {
            std::env::split_paths(p)
                .map(|d| d.join(name))
                .find(|f| f.is_file())
        })
    };
    let first = |paths: Vec<Option<PathBuf>>| paths.into_iter().flatten().find(|p| p.is_file());
    Installed {
        wt: first(vec![
            on_path("wt.exe"),
            local_app_data.map(|d| under(d, &["Microsoft", "WindowsApps", "wt.exe"])),
        ]),
        pwsh: first(vec![
            on_path("pwsh.exe"),
            program_files.map(|d| under(d, &["PowerShell", "7", "pwsh.exe"])),
        ]),
        powershell: first(vec![
            system_root.map(|d| {
                under(
                    d,
                    &["System32", "WindowsPowerShell", "v1.0", "powershell.exe"],
                )
            }),
            on_path("powershell.exe"),
        ]),
        cmd: first(vec![
            system_root.map(|d| under(d, &["System32", "cmd.exe"])),
            on_path("cmd.exe"),
        ]),
        git_bash: first(vec![
            program_files.map(|d| under(d, &["Git", "git-bash.exe"])),
            on_path("git-bash.exe"),
        ]),
    }
}

/// The argv words of the job (program first), or `None` for a plain shell.
fn job_words(job: &Job, haryx: &Path) -> Option<Vec<String>> {
    let exe = haryx.to_string_lossy().into_owned();
    match job {
        Job::Shell => None,
        Job::Claude => Some(vec!["claude".into()]),
        Job::RunAs { slot } => Some(vec![exe, "run".into(), slot.to_string()]),
        Job::RunMapped => Some(vec![exe, "run".into()]),
        Job::Login { slot } => {
            let mut words = vec![exe, "ai".into(), "login".into()];
            words.extend(slot.map(|n| n.to_string()));
            Some(words)
        }
        Job::InstallClaude => Some(vec![
            "powershell.exe".into(),
            "-NoProfile".into(),
            "-Command".into(),
            INSTALL_CLAUDE.into(),
        ]),
        Job::Continue { pid, to } => Some(vec![
            exe,
            "ai".into(),
            "sessions".into(),
            "continue".into(),
            pid.to_string(),
            "--to".into(),
            to.map_or_else(|| "best".to_owned(), |n| n.to_string()),
        ]),
    }
}

/// PowerShell: single quotes, `'` doubled; `&` calls the quoted program.
fn powershell_command(words: &[String]) -> String {
    let quoted: Vec<String> = words
        .iter()
        .map(|w| format!("'{}'", w.replace('\'', "''")))
        .collect();
    format!("& {}", quoted.join(" "))
}

/// cmd: double quotes around each word. Words are a path (which cannot hold `"`) and plain tokens.
fn cmd_command(words: &[String]) -> String {
    words
        .iter()
        .map(|w| format!("\"{w}\""))
        .collect::<Vec<_>>()
        .join(" ")
}

/// bash: single quotes, `'` closed and escaped; Windows paths become forward-slash paths bash accepts.
fn bash_command(words: &[String]) -> String {
    words
        .iter()
        .map(|w| format!("'{}'", w.replace('\\', "/").replace('\'', r"'\''")))
        .collect::<Vec<_>>()
        .join(" ")
}

/// The program and arguments that open `terminal` running `job`.
pub fn launch_argv(
    terminal: Terminal,
    job: &Job,
    haryx: &Path,
    folder: &Path,
    installed: &Installed,
) -> Result<Vec<String>, String> {
    let program = installed
        .path(terminal)
        .ok_or_else(|| format!("{} is not installed", terminal.title()))?
        .to_string_lossy()
        .into_owned();
    let words = job_words(job, haryx);
    let shell_args = |shell: Terminal| -> Vec<String> {
        match (shell, &words) {
            (Terminal::Pwsh | Terminal::Powershell, None) => vec!["-NoLogo".into()],
            (Terminal::Pwsh | Terminal::Powershell, Some(w)) => vec![
                "-NoLogo".into(),
                "-NoExit".into(),
                "-Command".into(),
                powershell_command(w),
            ],
            (Terminal::Cmd, None) => vec![],
            (Terminal::Cmd, Some(w)) => vec!["/K".into(), cmd_command(w)],
            (_, None) => vec![],
            (_, Some(w)) => vec!["-c".into(), format!("{}; exec bash", bash_command(w))],
        }
    };
    let mut argv = vec![program];
    match terminal {
        Terminal::WindowsTerminal => {
            let (shell, shell_path) = installed
                .tab_shell()
                .ok_or("no shell found for Windows Terminal")?;
            argv.extend([
                "new-tab".into(),
                "--startingDirectory".into(),
                folder.to_string_lossy().into_owned(),
                "--".into(),
                shell_path.to_string_lossy().into_owned(),
            ]);
            argv.extend(shell_args(shell));
        }
        Terminal::GitBash => {
            argv.push(format!("--cd={}", folder.to_string_lossy()));
            argv.extend(shell_args(Terminal::GitBash));
        }
        shell => argv.extend(shell_args(shell)),
    }
    Ok(argv)
}

/// Start the terminal in its own console window.
pub fn launch(argv: &[String], folder: &Path) -> Result<(), String> {
    let (program, args) = argv.split_first().ok_or("nothing to launch")?;
    let mut command = Command::new(program);
    command.args(args).current_dir(folder);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;
        command.creation_flags(CREATE_NEW_CONSOLE);
    }
    command
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("could not open {program}: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_installed() -> Installed {
        Installed {
            wt: Some(r"C:\wt.exe".into()),
            pwsh: Some(r"C:\pwsh.exe".into()),
            powershell: Some(r"C:\ps.exe".into()),
            cmd: Some(r"C:\cmd.exe".into()),
            git_bash: Some(r"C:\git-bash.exe".into()),
        }
    }

    const HARYX: &str = r"C:\Users\o'neil\bin\haryx.exe";

    fn argv(t: Terminal, job: Job) -> Vec<String> {
        launch_argv(
            t,
            &job,
            Path::new(HARYX),
            Path::new(r"D:\code"),
            &all_installed(),
        )
        .unwrap()
    }

    #[test]
    fn windows_terminal_opens_a_pwsh_tab_in_the_folder() {
        assert_eq!(
            argv(Terminal::WindowsTerminal, Job::RunAs { slot: 2 }),
            [
                r"C:\wt.exe",
                "new-tab",
                "--startingDirectory",
                r"D:\code",
                "--",
                r"C:\pwsh.exe",
                "-NoLogo",
                "-NoExit",
                "-Command",
                r"& 'C:\Users\o''neil\bin\haryx.exe' 'run' '2'",
            ]
        );
    }

    #[test]
    fn cmd_keeps_the_window_after_the_command() {
        assert_eq!(
            argv(Terminal::Cmd, Job::RunMapped),
            [
                r"C:\cmd.exe",
                "/K",
                r#""C:\Users\o'neil\bin\haryx.exe" "run""#
            ]
        );
        assert_eq!(argv(Terminal::Cmd, Job::Shell), [r"C:\cmd.exe"]);
    }

    #[test]
    fn git_bash_gets_a_posix_path_and_stays_open() {
        assert_eq!(
            argv(Terminal::GitBash, Job::Claude),
            [
                r"C:\git-bash.exe",
                r"--cd=D:\code",
                "-c",
                "'claude'; exec bash"
            ]
        );
        let run = argv(Terminal::GitBash, Job::RunAs { slot: 3 });
        assert_eq!(
            run[3],
            r"'C:/Users/o'\''neil/bin/haryx.exe' 'run' '3'; exec bash"
        );
    }

    #[test]
    fn continue_runs_the_handoff_in_the_terminal() {
        let words = argv(
            Terminal::Powershell,
            Job::Continue {
                pid: 4242,
                to: Some(3),
            },
        );
        assert_eq!(
            words.last().unwrap(),
            r"& 'C:\Users\o''neil\bin\haryx.exe' 'ai' 'sessions' 'continue' '4242' '--to' '3'"
        );
        let best = argv(Terminal::Powershell, Job::Continue { pid: 7, to: None });
        assert!(best.last().unwrap().ends_with("'--to' 'best'"));
    }

    #[test]
    fn login_runs_in_the_terminal() {
        let words = argv(Terminal::Powershell, Job::Login { slot: Some(4) });
        assert_eq!(
            words.last().unwrap(),
            r"& 'C:\Users\o''neil\bin\haryx.exe' 'ai' 'login' '4'"
        );
    }

    #[test]
    fn install_claude_runs_the_powershell_installer_from_any_shell() {
        assert_eq!(
            argv(Terminal::Cmd, Job::InstallClaude).last().unwrap(),
            r#""powershell.exe" "-NoProfile" "-Command" "irm https://claude.ai/install.ps1 | iex""#
        );
        assert_eq!(
            argv(Terminal::Powershell, Job::InstallClaude)
                .last()
                .unwrap(),
            "& 'powershell.exe' '-NoProfile' '-Command' 'irm https://claude.ai/install.ps1 | iex'"
        );
    }

    #[test]
    fn a_plain_shell_runs_nothing() {
        assert_eq!(
            argv(Terminal::Pwsh, Job::Shell),
            [r"C:\pwsh.exe", "-NoLogo"]
        );
    }

    #[test]
    fn missing_terminal_is_an_error() {
        let none = Installed::default();
        let err = launch_argv(
            Terminal::WindowsTerminal,
            &Job::Shell,
            Path::new(HARYX),
            Path::new("."),
            &none,
        )
        .unwrap_err();
        assert_eq!(err, "Windows Terminal is not installed");
    }

    #[test]
    fn detect_finds_files_in_the_usual_folders() {
        let tmp = tempfile::tempdir().unwrap();
        let sys = tmp.path().join("Windows");
        let cmd = sys.join("System32").join("cmd.exe");
        std::fs::create_dir_all(cmd.parent().unwrap()).unwrap();
        std::fs::write(&cmd, "").unwrap();
        let found = detect(None, None, None, Some(&sys));
        assert_eq!(found.cmd, Some(cmd));
        assert_eq!(found.available(), vec![Terminal::Cmd]);
    }
}
