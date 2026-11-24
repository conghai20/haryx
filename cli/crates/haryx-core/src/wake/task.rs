//! The per-user Task Scheduler task that runs `haryx ai wake run --once` every minute (Windows).
//!
//! The Windows form of the macOS LaunchAgent. "Run as soon as possible after a scheduled start is
//! missed" (`StartWhenAvailable`) gives the same "fire late after sleep" behaviour, and the task runs as
//! the signed-in user with no stored password and no admin rights.
//!
//! Task Scheduler has no per-task environment, so the variables the shell had (`HARYX_HOME`, ...) ride
//! along as leading `--env NAME=VALUE` arguments, which `haryxw` (the windowless build of haryx) applies
//! before dispatching. Running `haryxw.exe` rather than the console `haryx.exe` keeps a console window
//! from flashing up every minute.

use std::path::Path;

/// The task's path in Task Scheduler.
pub const TASK_NAME: &str = r"\HaryX\Wake";
/// The windowless build that runs the tick, next to `haryx.exe`.
pub const WINDOWLESS_EXE: &str = "haryxw.exe";

/// The arguments the task passes: carried variables first, then the tick.
pub fn arguments(env: &[(String, String)]) -> Vec<String> {
    env.iter()
        .flat_map(|(k, v)| ["--env".to_string(), format!("{k}={v}")])
        .chain(["ai", "wake", "run", "--once"].map(String::from))
        .collect()
}

/// One argument quoted for a Windows command line, so `CommandLineToArgvW` (and Rust's own parser)
/// read it back unchanged: backslashes double only where they precede a quote.
pub fn quote_arg(arg: &str) -> String {
    if !arg.is_empty() && !arg.contains([' ', '\t', '\n', '"']) {
        return arg.to_string();
    }
    let mut out = String::from("\"");
    let mut backslashes = 0;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// XML-escape text, dropping characters XML 1.0 cannot carry.
fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r'))
        .collect::<String>()
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// The task definition for `exe` ticking every `interval_s` seconds. `start` is the first trigger time,
/// local, as `YYYY-MM-DDTHH:MM:SS`.
pub fn task_xml(exe: &Path, env: &[(String, String)], interval_s: u32, start: &str) -> String {
    let command = escape(&exe.to_string_lossy());
    let args: Vec<String> = arguments(env).iter().map(|a| quote_arg(a)).collect();
    let args = escape(&args.join(" "));
    let minutes = interval_s.div_ceil(60).max(1);
    format!(
        r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.2" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>HaryX: restart the 5-hour window of opted-in accounts when it resets.</Description>
    <URI>{TASK_NAME}</URI>
  </RegistrationInfo>
  <Triggers>
    <TimeTrigger>
      <StartBoundary>{start}</StartBoundary>
      <Enabled>true</Enabled>
      <Repetition>
        <Interval>PT{minutes}M</Interval>
        <StopAtDurationEnd>false</StopAtDurationEnd>
      </Repetition>
    </TimeTrigger>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <StartWhenAvailable>true</StartWhenAvailable>
    <RunOnlyIfNetworkAvailable>false</RunOnlyIfNetworkAvailable>
    <IdleSettings>
      <StopOnIdleEnd>false</StopOnIdleEnd>
      <RestartOnIdle>false</RestartOnIdle>
    </IdleSettings>
    <AllowStartOnDemand>true</AllowStartOnDemand>
    <Enabled>true</Enabled>
    <Hidden>true</Hidden>
    <ExecutionTimeLimit>PT5M</ExecutionTimeLimit>
    <Priority>7</Priority>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{command}</Command>
      <Arguments>{args}</Arguments>
    </Exec>
  </Actions>
</Task>
"#
    )
}

/// UTF-16LE with a byte-order mark: the encoding `schtasks /XML` reads reliably.
pub fn utf16_bytes(text: &str) -> Vec<u8> {
    [0xFF, 0xFE]
        .into_iter()
        .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arguments_carry_env_before_the_tick() {
        let env = vec![("HARYX_HOME".to_string(), r"D:\store".to_string())];
        assert_eq!(
            arguments(&env),
            [
                "--env",
                r"HARYX_HOME=D:\store",
                "ai",
                "wake",
                "run",
                "--once"
            ]
        );
    }

    #[test]
    fn quote_arg_round_trips_windows_rules() {
        assert_eq!(quote_arg("plain"), "plain");
        assert_eq!(quote_arg(""), "\"\"");
        assert_eq!(quote_arg(r"C:\Program Files\x"), r#""C:\Program Files\x""#);
        assert_eq!(quote_arg(r"ends\ in\"), r#""ends\ in\\""#);
        assert_eq!(quote_arg(r#"say "hi""#), r#""say \"hi\"""#);
        assert_eq!(quote_arg(r#"a\"b"#), r#""a\\\"b""#);
    }

    #[test]
    fn task_runs_every_minute_and_catches_up_after_sleep() {
        let xml = task_xml(
            Path::new(r"C:\Users\u\bin\haryxw.exe"),
            &[],
            60,
            "2026-11-24T00:00:00",
        );
        assert!(xml.contains(r"<Command>C:\Users\u\bin\haryxw.exe</Command>"));
        assert!(xml.contains("<Arguments>ai wake run --once</Arguments>"));
        assert!(xml.contains("<Interval>PT1M</Interval>"));
        assert!(xml.contains("<StartWhenAvailable>true</StartWhenAvailable>"));
        assert!(xml.contains("<RunLevel>LeastPrivilege</RunLevel>"));
        assert!(xml.contains("<StartBoundary>2026-11-24T00:00:00</StartBoundary>"));
    }

    #[test]
    fn task_xml_escapes_paths_and_env() {
        let env = vec![("HARYX_HOME".to_string(), r"C:\a & b".to_string())];
        let xml = task_xml(Path::new(r"C:\x<y>.exe"), &env, 60, "2026-01-01T00:00:00");
        assert!(xml.contains(r"<Command>C:\x&lt;y&gt;.exe</Command>"));
        assert!(
            xml.contains(r#"--env &quot;HARYX_HOME=C:\a &amp; b&quot; ai"#),
            "{xml}"
        );
    }

    #[test]
    fn utf16_has_bom_and_little_endian_units() {
        assert_eq!(utf16_bytes("A"), vec![0xFF, 0xFE, 0x41, 0x00]);
    }
}
