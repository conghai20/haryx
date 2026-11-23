//! Home-path portability: the absolute home becomes [`HOME_TOKEN`] on the way out and the target home on
//! the way in. Claude Code names project folders after the project path (`/Users/me/app` becomes
//! `-Users-me-app`), so folder names use [`SLUG_TOKEN`] for the home part.

use std::borrow::Cow;

pub const HOME_TOKEN: &str = "{{HARYX_HOME}}";
pub const SLUG_TOKEN: &str = "{{HARYX_HOME_SLUG}}";

/// The home without a trailing slash (`/` stays `/`).
pub fn normalize_home(home: &str) -> String {
    let trimmed = home.trim_end_matches('/');
    if trimmed.is_empty() {
        "/".to_owned()
    } else {
        trimmed.to_owned()
    }
}

/// Claude Code's project folder name for a path: every character but ASCII letters and digits becomes `-`.
pub fn project_slug(path: &str) -> String {
    path.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// A home too short to rewrite safely (`/`, empty).
fn rewritable(home: &str) -> bool {
    home.len() > 1
}

/// Replace the home path in text with [`HOME_TOKEN`].
pub fn tokenize_text<'a>(text: &'a str, home: &str) -> Cow<'a, str> {
    if !rewritable(home) || !text.contains(home) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace(home, HOME_TOKEN))
}

/// Replace [`HOME_TOKEN`] in text with the home path.
pub fn expand_text<'a>(text: &'a str, home: &str) -> Cow<'a, str> {
    if !text.contains(HOME_TOKEN) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(text.replace(HOME_TOKEN, home))
}

/// A project folder name with its home part replaced by [`SLUG_TOKEN`].
pub fn tokenize_name(name: &str, home: &str) -> String {
    if !rewritable(home) {
        return name.to_owned();
    }
    let slug = project_slug(home);
    match name.strip_prefix(&slug) {
        Some(rest) if rest.is_empty() || rest.starts_with('-') => format!("{SLUG_TOKEN}{rest}"),
        _ => name.to_owned(),
    }
}

/// A project folder name with [`SLUG_TOKEN`] replaced by the home's slug.
pub fn expand_name(name: &str, home: &str) -> String {
    match name.strip_prefix(SLUG_TOKEN) {
        Some(rest) => format!("{}{rest}", project_slug(home)),
        None => name.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_home_in_text() {
        let text = "bash /Users/me/.claude/hooks/a.sh";
        let tokenized = tokenize_text(text, "/Users/me");
        assert_eq!(tokenized, "bash {{HARYX_HOME}}/.claude/hooks/a.sh");
        assert_eq!(
            expand_text(&tokenized, "/home/me"),
            "bash /home/me/.claude/hooks/a.sh"
        );
    }

    #[test]
    fn leaves_root_home_alone() {
        assert_eq!(tokenize_text("/usr/bin", "/"), "/usr/bin");
    }

    #[test]
    fn rewrites_project_folder_names() {
        let name = tokenize_name("-Users-me-code-app", "/Users/me");
        assert_eq!(name, "{{HARYX_HOME_SLUG}}-code-app");
        assert_eq!(expand_name(&name, "/home/me.x"), "-home-me-x-code-app");
        assert_eq!(
            tokenize_name("-Users-meow-app", "/Users/me"),
            "-Users-meow-app"
        );
        assert_eq!(tokenize_name("-opt-app", "/Users/me"), "-opt-app");
    }

    #[test]
    fn normalizes_trailing_slash() {
        assert_eq!(normalize_home("/Users/me/"), "/Users/me");
        assert_eq!(normalize_home("/"), "/");
    }
}
