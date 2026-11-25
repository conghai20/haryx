//! The page the browser shows after it returns to the loopback listener.
//!
//! One template (`assets/callback.html`) in the site's "Ink & tide" design, two states: `done` (the
//! Wing H's halves join and the tide line draws) and `failed` (the halves stay parted, the line lies
//! flat). It is answered only after the code exchange, so it never claims a sign-in that then failed.
//! Fonts are subsets embedded as data, so the page makes no request (`assets/fonts`, OFL).

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;

const TEMPLATE: &str = include_str!("../../assets/callback.html");
const FONT_DISPLAY: &[u8] = include_bytes!("../../assets/fonts/space-grotesk-500.subset.woff2");
const FONT_TEXT: &[u8] = include_bytes!("../../assets/fonts/geist-400.subset.woff2");
const FONT_MONO: &[u8] = include_bytes!("../../assets/fonts/geist-mono-400.subset.woff2");

/// The headlines are set in a Space Grotesk subset holding exactly these glyphs.
const DONE_HEADLINE: &str = "Signed in.";
const FAILED_HEADLINE: &str = "<span class=\"nowrap\">Sign-in</span> didn’t finish.";

const TIDE_WAVE: &str = r#"<div class="tide" aria-hidden="true"><svg class="tideline" viewBox="0 0 1440 80" preserveAspectRatio="none"><path class="tideline-path" pathLength="1" d="M0 50 C 180 50, 300 18, 520 20 S 860 70, 1080 62 S 1340 30, 1440 34"/></svg></div>"#;
const TIDE_STILL: &str = r#"<div class="tide tide--still" aria-hidden="true"><svg class="tideline" viewBox="0 0 1440 80" preserveAspectRatio="none"><path class="tideline-path" d="M0 40 H1440"/></svg></div>"#;

/// The page for a finished sign-in as `email` (empty when the provider gave none).
pub fn signed_in(email: &str) -> String {
    let who = if email.is_empty() {
        "This machine is signed in to HaryX.".to_owned()
    } else {
        format!(
            "This machine is signed in to HaryX as <span class=\"who\">{}</span>.",
            escape(email)
        )
    };
    let content = format!(
        "<h1 class=\"title\">{DONE_HEADLINE}</h1>\n{TIDE_WAVE}\n<p class=\"lede\">{who}</p>\n\
         <p class=\"note\">You can close this tab and go back to HaryX.</p>"
    );
    render("Signed in to HaryX", "done", &content)
}

/// The page for a sign-in that did not finish, with the reason as HaryX reports it.
pub fn not_finished(reason: &str) -> String {
    let content = format!(
        "<h1 class=\"title\">{FAILED_HEADLINE}</h1>\n{TIDE_STILL}\n<p class=\"lede\">{}</p>\n\
         <p class=\"note\">Go back to HaryX and sign in again, or run</p>\n\
         <p class=\"cmd\"><code><span class=\"prompt\" aria-hidden=\"true\">$ </span>haryx login</code></p>",
        escape(reason)
    );
    render("Sign-in didn’t finish · HaryX", "failed", &content)
}

fn render(title: &str, state: &str, content: &str) -> String {
    TEMPLATE
        .replace("{{title}}", title)
        .replace("{{state}}", state)
        .replace("{{font_display}}", &STANDARD.encode(FONT_DISPLAY))
        .replace("{{font_text}}", &STANDARD.encode(FONT_TEXT))
        .replace("{{font_mono}}", &STANDARD.encode(FONT_MONO))
        .replace("{{content}}", content)
}

fn escape(text: &str) -> String {
    text.chars()
        .filter(|c| !c.is_control())
        .map(|c| match c {
            '&' => "&amp;".to_owned(),
            '<' => "&lt;".to_owned(),
            '>' => "&gt;".to_owned(),
            '"' => "&quot;".to_owned(),
            '\'' => "&#39;".to_owned(),
            other => other.to_string(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn done_page_names_the_account_and_fills_every_slot() {
        let page = signed_in("you@example.com");
        assert!(page.contains("data-state=\"done\""));
        assert!(page.contains("<title>Signed in to HaryX</title>"));
        assert!(page.contains("as <span class=\"who\">you@example.com</span>."));
        assert!(!page.contains("{{"), "a placeholder was left unfilled");
        assert!(signed_in("").contains("This machine is signed in to HaryX."));
    }

    #[test]
    fn failed_page_escapes_the_reason() {
        let page = not_finished("<script>alert(1)</script> & \u{1b}]52;x");
        assert!(page.contains("data-state=\"failed\""));
        assert!(page.contains("&lt;script&gt;alert(1)&lt;/script&gt; &amp; ]52;x"));
        assert!(!page.contains("<script>"));
        assert!(!page.contains('\u{1b}'));
        assert!(!page.contains("{{"));
    }

    #[test]
    fn the_page_makes_no_request() {
        for page in [signed_in("a@b.c"), not_finished("x")] {
            for needle in [
                "src=\"http",
                "href=\"http",
                "url(http",
                "@import",
                "<script",
            ] {
                assert!(!page.contains(needle), "found {needle}");
            }
        }
    }

    #[test]
    fn the_headlines_stay_inside_the_display_subset() {
        // The embedded Space Grotesk holds only these glyphs; a new headline needs a new subset.
        let glyphs: std::collections::BTreeSet<char> =
            "Signed in.Sign-in didn’t finish.".chars().collect();
        for headline in [DONE_HEADLINE, FAILED_HEADLINE] {
            let text = headline
                .replace("<span class=\"nowrap\">", "")
                .replace("</span>", "");
            assert!(text.chars().all(|c| glyphs.contains(&c)), "{headline}");
        }
    }
}
