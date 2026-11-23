//! Which part of a Claude Code setup each entry of `~/.claude` belongs to.

use std::fmt;

/// A part of the setup. Default-on parts export unless excluded; optional parts only when asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Settings,
    Hooks,
    Skills,
    Agents,
    Commands,
    Rules,
    OutputStyles,
    Plugins,
    Mcps,
    Memory,
    Other,
    History,
    Credentials,
    Signed,
    ClaudeJson,
    Cache,
}

impl Category {
    pub const ALL: [Category; 16] = [
        Category::Settings,
        Category::Hooks,
        Category::Skills,
        Category::Agents,
        Category::Commands,
        Category::Rules,
        Category::OutputStyles,
        Category::Plugins,
        Category::Mcps,
        Category::Memory,
        Category::Other,
        Category::History,
        Category::Credentials,
        Category::Signed,
        Category::ClaudeJson,
        Category::Cache,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Category::Settings => "settings",
            Category::Hooks => "hooks",
            Category::Skills => "skills",
            Category::Agents => "agents",
            Category::Commands => "commands",
            Category::Rules => "rules",
            Category::OutputStyles => "output-styles",
            Category::Plugins => "plugins",
            Category::Mcps => "mcps",
            Category::Memory => "memory",
            Category::Other => "other",
            Category::History => "history",
            Category::Credentials => "credentials",
            Category::Signed => "signed",
            Category::ClaudeJson => "claude-json",
            Category::Cache => "cache",
        }
    }

    pub fn parse(text: &str) -> Option<Category> {
        Category::ALL.into_iter().find(|c| c.name() == text)
    }

    /// Parse a comma list (`skills,mcps`). The error names the unknown entry.
    pub fn parse_list(text: &str) -> Result<Vec<Category>, String> {
        text.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                Category::parse(s).ok_or_else(|| {
                    let known: Vec<&str> = Category::ALL.iter().map(|c| c.name()).collect();
                    format!("unknown category '{s}' (known: {})", known.join(", "))
                })
            })
            .collect()
    }

    /// Exported unless excluded.
    pub fn default_on(self) -> bool {
        !matches!(
            self,
            Category::Credentials | Category::Signed | Category::ClaudeJson | Category::Cache
        )
    }

    /// `--prune` may remove local items of this category.
    pub fn prunable(self) -> bool {
        !matches!(
            self,
            Category::History | Category::Credentials | Category::Plugins | Category::Cache
        )
    }
}

impl fmt::Display for Category {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

/// Entries of `~/.claude` that never leave the machine: credentials and runtime state that points at local
/// processes, editors and sockets.
const NEVER: &[&str] = &[
    ".credentials.json",
    ".claude.json",
    ".DS_Store",
    ".last-cleanup",
    ".last-update-result.json",
    "sessions",
    "session-env",
    "shell-snapshots",
    "ide",
    "debug",
    "backups",
    "telemetry",
    "tmp",
    "downloads",
    "daemon",
    "chrome",
    "jobs",
];

/// JSON files of `~/.claude` handled key by key, not as files.
pub const SETTINGS_FILES: &[&str] = &["settings.json", "settings.local.json"];

/// The category of a top-level entry of `~/.claude`, or `None` when it is never exported.
pub fn classify(name: &str) -> Option<Category> {
    if NEVER.contains(&name) || name.ends_with(".lock") {
        return None;
    }
    let category = match name {
        n if SETTINGS_FILES.contains(&n) => Category::Settings,
        "hooks" | "scripts" => Category::Hooks,
        "skills" => Category::Skills,
        "agents" => Category::Agents,
        "commands" => Category::Commands,
        "rules" => Category::Rules,
        "output-styles" => Category::OutputStyles,
        "plugins" => Category::Plugins,
        "CLAUDE.md" | "AGENTS.md" | "keybindings.json" => Category::Memory,
        "projects" | "history.jsonl" | "file-history" | "todos" | "plans" => Category::History,
        "cache" | "paste-cache" | "statsig" | "stats-cache.json" | "mcp-needs-auth-cache.json" => {
            Category::Cache
        }
        n if n.starts_with("policy-limits") || n.starts_with("remote-settings") => Category::Signed,
        _ => Category::Other,
    };
    Some(category)
}

/// Directories whose children are separate items (one skill, one project, ...). Other entries are one
/// item each.
pub fn splits_children(name: &str) -> bool {
    matches!(
        name,
        "hooks"
            | "scripts"
            | "skills"
            | "agents"
            | "commands"
            | "rules"
            | "output-styles"
            | "projects"
            | "file-history"
    )
}

/// Child entries never copied (OS litter, plugin housekeeping).
pub fn ignored_child(name: &str) -> bool {
    matches!(name, ".DS_Store" | ".trash" | ".last_inuse_sweep") || name.ends_with(".lock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_known_entries() {
        assert_eq!(classify("skills"), Some(Category::Skills));
        assert_eq!(classify("settings.json"), Some(Category::Settings));
        assert_eq!(classify("projects"), Some(Category::History));
        assert_eq!(
            classify("policy-limits.json.signature.json"),
            Some(Category::Signed)
        );
        assert_eq!(classify("ecc"), Some(Category::Other));
        assert_eq!(classify(".credentials.json"), None);
        assert_eq!(classify("sessions"), None);
        assert_eq!(classify("daemon"), None);
    }

    #[test]
    fn parses_category_lists() {
        assert_eq!(
            Category::parse_list("skills, mcps").unwrap(),
            vec![Category::Skills, Category::Mcps]
        );
        assert!(
            Category::parse_list("skills,nope")
                .unwrap_err()
                .contains("nope")
        );
    }
}
