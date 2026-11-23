//! Plugins by reference: read what is installed, reinstall through the `claude` CLI.

use std::collections::BTreeSet;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use serde_json::Value;

use super::ProfileEnv;
use super::items::{JsonDoc, read_live_json};
use super::manifest::PluginRef;
use crate::error::{Error, Result};

/// Installed plugins with their marketplace source, sorted by id.
pub fn live_refs(env: &ProfileEnv) -> Result<Vec<PluginRef>> {
    let installed = read_live_json(env, JsonDoc::InstalledPlugins)?.unwrap_or(Value::Null);
    let known = read_live_json(env, JsonDoc::KnownMarketplaces)?.unwrap_or(Value::Null);
    let Some(plugins) = installed.get("plugins").and_then(Value::as_object) else {
        return Ok(Vec::new());
    };
    let mut refs: Vec<PluginRef> = plugins
        .iter()
        .map(|(id, entries)| {
            let marketplace = id.rsplit_once('@').map(|(_, m)| m).unwrap_or("").to_owned();
            let version = entries
                .as_array()
                .and_then(|a| a.first())
                .and_then(|e| e.get("version"))
                .and_then(Value::as_str)
                .map(str::to_owned);
            let source = known
                .get(&marketplace)
                .and_then(|m| m.get("source"))
                .cloned()
                .unwrap_or(Value::Null);
            PluginRef {
                id: id.clone(),
                marketplace,
                version,
                source,
            }
        })
        .collect();
    refs.sort_by(|a, b| a.id.cmp(&b.id));
    Ok(refs)
}

/// Ids of the plugins installed here.
pub fn installed_ids(env: &ProfileEnv) -> Result<BTreeSet<String>> {
    let installed = read_live_json(env, JsonDoc::InstalledPlugins)?.unwrap_or(Value::Null);
    Ok(installed
        .get("plugins")
        .and_then(Value::as_object)
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default())
}

/// Names of the marketplaces known here.
pub fn known_marketplaces(env: &ProfileEnv) -> Result<BTreeSet<String>> {
    let known = read_live_json(env, JsonDoc::KnownMarketplaces)?.unwrap_or(Value::Null);
    Ok(known
        .as_object()
        .map(|m| m.keys().cloned().collect())
        .unwrap_or_default())
}

/// The argument of `claude plugin marketplace add` for a marketplace source.
pub fn marketplace_arg(source: &Value) -> Option<String> {
    let kind = source.get("source")?.as_str()?;
    let field = match kind {
        "github" => "repo",
        "git" | "url" => "url",
        "directory" | "file" => "path",
        _ => return None,
    };
    source.get(field)?.as_str().map(str::to_owned)
}

/// Installs plugins. The real one shells out to `claude`; tests use a fake.
pub trait PluginInstaller {
    fn add_marketplace(&self, source: &str) -> Result<()>;
    fn install(&self, id: &str) -> Result<()>;
}

/// A value from a profile must never be read by `claude` as an option.
fn no_option(value: &str) -> Result<&str> {
    if value.starts_with('-') || value.trim().is_empty() {
        return Err(Error::Validation(format!(
            "Refusing plugin value '{value}'"
        )));
    }
    Ok(value)
}

/// `claude plugin marketplace add` / `claude plugin install`.
pub struct ClaudeCli {
    pub bin: PathBuf,
}

impl ClaudeCli {
    /// `claude` from `PATH`.
    pub fn from_path() -> Result<Self> {
        crate::spawn::resolve(
            std::ffi::OsStr::new("claude"),
            std::env::var_os("PATH").as_deref(),
        )
        .filter(|p| p.is_file())
        .map(|bin| ClaudeCli { bin })
        .ok_or_else(|| {
            Error::General("'claude' was not found on PATH; plugins were not installed".into())
        })
    }

    fn run(&self, args: &[&str]) -> Result<()> {
        let output = Command::new(&self.bin)
            .args(args)
            .stdin(Stdio::null())
            .output()?;
        if output.status.success() {
            return Ok(());
        }
        let err = String::from_utf8_lossy(&output.stderr);
        let out = String::from_utf8_lossy(&output.stdout);
        let detail = err
            .lines()
            .chain(out.lines())
            .find(|l| !l.trim().is_empty());
        Err(Error::General(format!(
            "claude {} failed{}",
            args.join(" "),
            detail
                .map(|d| format!(": {}", d.trim()))
                .unwrap_or_default()
        )))
    }
}

impl PluginInstaller for ClaudeCli {
    fn add_marketplace(&self, source: &str) -> Result<()> {
        self.run(&["plugin", "marketplace", "add", no_option(source)?])
    }

    fn install(&self, id: &str) -> Result<()> {
        self.run(&["plugin", "install", no_option(id)?])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn marketplace_args_by_source_kind() {
        assert_eq!(
            marketplace_arg(&json!({"source": "github", "repo": "o/r"})).as_deref(),
            Some("o/r")
        );
        assert_eq!(
            marketplace_arg(&json!({"source": "git", "url": "https://x/y.git"})).as_deref(),
            Some("https://x/y.git")
        );
        assert_eq!(marketplace_arg(&json!({"source": "npm"})), None);
    }

    #[test]
    fn reads_live_refs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let plugins = home.join(".claude/plugins");
        std::fs::create_dir_all(&plugins).unwrap();
        std::fs::write(
            plugins.join("installed_plugins.json"),
            json!({"version": 2, "plugins": {"p@m": [{"version": "1.0"}]}}).to_string(),
        )
        .unwrap();
        std::fs::write(
            plugins.join("known_marketplaces.json"),
            json!({"m": {"source": {"source": "github", "repo": "o/m"}}}).to_string(),
        )
        .unwrap();
        let env = ProfileEnv::from_path_env(&crate::paths::PathEnv::with_home(home)).unwrap();
        let refs = live_refs(&env).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].marketplace, "m");
        assert_eq!(refs[0].version.as_deref(), Some("1.0"));
        assert_eq!(marketplace_arg(&refs[0].source).as_deref(), Some("o/m"));
    }
}
