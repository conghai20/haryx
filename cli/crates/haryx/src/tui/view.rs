//! Rendering: account cards with usage bars, the screens' frames, modals and key hints.

use haryx_core::json_output::USAGE_API_KEY;
use haryx_core::models::AccountSnapshot;
use haryx_core::oauth::FIVE_HOUR_LABEL;
use haryx_core::pace::compute_pace;
use haryx_core::poll_policy::{SERVE_TTL_S, parse_reset_ts};
use haryx_core::switcher::{error_note, last_seen_note, sentinel_note};
use haryx_core::usage_store::STALE_OK_S;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};
use serde_json::Value;

use super::app::{App, Modal, Screen};

const BAR_FILLED: &str = "━";
const BAR_HALF: &str = "╸";
const BAR_EMPTY: &str = "─";
const BAR_TICK: &str = "┃";

/// Brand palette (PRODUCT.md): Midnight Navy, Electric Cyan, Soft White.
#[derive(Clone, Copy)]
pub struct Palette {
    pub fg: Color,
    pub muted: Color,
    pub accent: Color,
    pub track: Color,
    pub ok: Color,
    pub warn: Color,
    pub bad: Color,
    pub bg: Color,
}

impl Palette {
    pub const DARK: Palette = Palette {
        fg: Color::Rgb(0xF1, 0xF5, 0xF9),
        muted: Color::Rgb(0x94, 0xA3, 0xB8),
        accent: Color::Rgb(0x22, 0xD3, 0xEE),
        track: Color::Rgb(0x33, 0x41, 0x55),
        ok: Color::Rgb(0x34, 0xD3, 0x99),
        warn: Color::Rgb(0xFB, 0xBF, 0x24),
        bad: Color::Rgb(0xF8, 0x71, 0x71),
        bg: Color::Rgb(0x10, 0x18, 0x27),
    };
    pub const LIGHT: Palette = Palette {
        fg: Color::Rgb(0x10, 0x18, 0x27),
        muted: Color::Rgb(0x64, 0x74, 0x8B),
        accent: Color::Rgb(0x08, 0x91, 0xB2),
        track: Color::Rgb(0xCB, 0xD5, 0xE1),
        ok: Color::Rgb(0x05, 0x96, 0x69),
        warn: Color::Rgb(0xB4, 0x53, 0x09),
        bad: Color::Rgb(0xDC, 0x26, 0x26),
        bg: Color::Rgb(0xF8, 0xFA, 0xFC),
    };

    fn severity(&self, pct: f64) -> Color {
        if pct >= 90.0 {
            self.bad
        } else if pct >= 70.0 {
            self.warn
        } else {
            self.ok
        }
    }
}

/// `90` → `1m 30s`-style compact duration (upstream `format_duration`).
pub fn format_duration(seconds: f64) -> String {
    let s = seconds.max(0.0) as i64;
    match s {
        0..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => {
            let (h, m) = (s / 3600, (s / 60) % 60);
            if m == 0 {
                format!("{h}h")
            } else {
                format!("{h}h {m}m")
            }
        }
        _ => {
            let (d, h) = (s / 86400, (s / 3600) % 24);
            if h == 0 {
                format!("{d}d")
            } else {
                format!("{d}d {h}h")
            }
        }
    }
}

fn reset_text(window: &Value, now: f64) -> Option<String> {
    let ts = parse_reset_ts(window.get("resets_at").and_then(Value::as_str))?;
    let remaining = ts - now;
    Some(if remaining <= 0.0 {
        "resets now".into()
    } else {
        format!("resets {}", format_duration(remaining))
    })
}

/// One bar row: label, pct, suffix.
type Row = (String, Option<f64>, String);

fn pace_marker(window: &Value, fetched_at: Option<f64>) -> Option<&'static str> {
    compute_pace(Some(window), fetched_at)
        .filter(|p| p.ahead)
        .map(|_| "(ahead of pace)")
}

fn join(parts: &[Option<String>]) -> String {
    parts
        .iter()
        .flatten()
        .cloned()
        .collect::<Vec<_>>()
        .join("  ")
}

/// Upstream `usage_rows`: spend, 5h, 7d, scoped models, a refused probe.
pub fn usage_rows(usage: Option<&Value>, now: f64, fetched_at: Option<f64>) -> Vec<Row> {
    let Some(usage) = usage.filter(|u| u.is_object()) else {
        return Vec::new();
    };
    let mut rows = Vec::new();
    if let Some(spend) = usage.get("spend").filter(|s| s.is_object()) {
        let f = |k: &str| spend.get(k).and_then(Value::as_f64).unwrap_or(0.0);
        let amounts = format!("${:.2} / ${:.2}", f("used"), f("limit"));
        rows.push((
            "$$".into(),
            spend.get("pct").and_then(Value::as_f64),
            join(&[reset_text(spend, now), Some(amounts)]),
        ));
    }
    for (key, label) in [("five_hour", FIVE_HOUR_LABEL), ("seven_day", "7d")] {
        let Some(window) = usage.get(key).filter(|w| w.is_object()) else {
            continue;
        };
        let pace = (key == "seven_day")
            .then(|| pace_marker(window, fetched_at))
            .flatten()
            .map(str::to_owned);
        rows.push((
            label.into(),
            window.get("pct").and_then(Value::as_f64),
            join(&[reset_text(window, now), pace]),
        ));
    }
    for window in usage
        .get("scoped")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let pct = window.get("pct").and_then(Value::as_f64).unwrap_or(0.0);
        let marker = if pct >= 100.0 {
            Some("(!)".to_owned())
        } else {
            pace_marker(window, fetched_at).map(str::to_owned)
        };
        let name = window
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("model");
        rows.push((
            name.into(),
            Some(pct),
            join(&[reset_text(window, now), marker]),
        ));
    }
    if let Some(rejected) = usage.get("rejected").filter(|r| r.is_object()) {
        rows.push((
            "probe".into(),
            Some(100.0),
            join(&[Some("rejected (!)".into()), reset_text(rejected, now)]),
        ));
    }
    rows
}

fn bar(
    pct: Option<f64>,
    width: usize,
    stale: bool,
    threshold: Option<f64>,
    p: &Palette,
) -> Vec<Span<'static>> {
    let track = Style::default().fg(p.track);
    let Some(pct) = pct else {
        return vec![Span::styled(BAR_EMPTY.repeat(width), track)];
    };
    let cells = pct.clamp(0.0, 100.0) / 100.0 * width as f64;
    let full = cells as usize;
    let half = cells - full as f64 >= 0.5 && full < width;
    let tick = threshold
        .map(|t| ((t / 100.0 * width as f64).round() as usize).min(width.saturating_sub(1)));
    let mut fill = Style::default().fg(p.severity(pct));
    if stale {
        fill = fill.add_modifier(Modifier::DIM);
    }
    (0..width)
        .map(|i| {
            if Some(i) == tick {
                Span::styled(BAR_TICK, Style::default().fg(p.warn))
            } else if i < full {
                Span::styled(BAR_FILLED, fill)
            } else if i == full && half {
                Span::styled(BAR_HALF, fill)
            } else {
                Span::styled(BAR_EMPTY, track)
            }
        })
        .collect()
}

/// Upstream `account_card_text`: a header line, then a sentinel note or one bar per window.
pub fn card(
    acc: &AccountSnapshot,
    width: u16,
    thresholds: (f64, Option<f64>),
    now: f64,
    p: &Palette,
) -> Vec<Line<'static>> {
    let muted = Style::default().fg(p.muted);
    let fg = Style::default().fg(p.fg);
    let accent = Style::default().fg(p.accent).add_modifier(Modifier::BOLD);
    let mut head = vec![Span::styled(
        format!("{:>2}  ", acc.number),
        fg.add_modifier(Modifier::BOLD),
    )];
    if acc.alias.is_empty() {
        head.push(Span::styled(acc.email.clone(), fg));
    } else {
        head.push(Span::styled(acc.alias.clone(), accent));
        head.push(Span::styled(format!(" ({})", acc.email), fg));
    }
    head.push(Span::styled(format!("  [{}]", acc.display_tag()), muted));
    if acc.is_active {
        head.push(Span::styled("   ● active", accent));
    }
    if acc.disabled {
        head.push(Span::styled("   (disabled)", muted));
    }
    if let Some(t) = acc.policy.threshold {
        head.push(Span::styled(format!("   switch at {t}%"), muted));
    }
    if acc.policy.priority > 0 {
        head.push(Span::styled(
            format!("   priority {}", acc.policy.priority),
            muted,
        ));
    }
    if let Some(age) = acc.usage.age_s.filter(|a| *a >= SERVE_TTL_S) {
        head.push(Span::styled(
            format!("   · {} ago", format_duration(age)),
            muted,
        ));
    }
    let mut lines = vec![Line::from(head)];
    if let Some(sentinel) = acc.usage.sentinel.as_deref() {
        let (marker, style) = if sentinel == USAGE_API_KEY {
            ("·", muted)
        } else {
            ("⚠", Style::default().fg(p.warn))
        };
        lines.push(Line::from(Span::styled(
            format!("    {marker} {}", sentinel_note(sentinel)),
            style,
        )));
        if sentinel != USAGE_API_KEY
            && let Some(seen) = last_seen_note(&acc.usage)
        {
            lines.push(Line::from(Span::styled(format!("    └ {seen}"), muted)));
        }
        return lines;
    }
    let rows = usage_rows(acc.usage.last_good.as_ref(), now, acc.usage.fetched_at);
    if rows.is_empty() {
        let mut text = "    usage unavailable".to_owned();
        if let Some(err) = &acc.usage.last_error {
            text.push_str(&format!(" · {}", error_note(err)));
        }
        lines.push(Line::from(Span::styled(text, muted)));
        return lines;
    }
    let stale = acc.usage.age_s.is_some_and(|a| a > STALE_OK_S);
    let label_width = rows
        .iter()
        .map(|(l, _, _)| l.chars().count())
        .max()
        .unwrap_or(2);
    let bar_width = (usize::from(width).saturating_sub(42 + label_width)).clamp(12, 30);
    let (threshold, weekly) = thresholds;
    let line_for = |label: &str| match weekly {
        Some(w) if label != "$$" && label != FIVE_HOUR_LABEL => w,
        _ => acc.policy.threshold.unwrap_or(threshold),
    };
    for (label, pct, suffix) in rows {
        let mut spans = vec![Span::styled(format!("    {label:<label_width$} "), muted)];
        spans.extend(bar(pct, bar_width, stale, Some(line_for(&label)), p));
        match pct {
            None => spans.push(Span::styled("  usage unknown", muted)),
            Some(pct) => {
                let mut style = Style::default().fg(p.severity(pct));
                if stale {
                    style = style.add_modifier(Modifier::DIM);
                }
                spans.push(Span::styled(format!(" {pct:3.0}%"), style));
            }
        }
        if !suffix.is_empty() {
            spans.push(Span::styled(format!("  {suffix}"), muted));
        }
        lines.push(Line::from(spans));
    }
    lines
}

fn hints(screen: Screen, app: &App) -> &'static str {
    match screen {
        Screen::Dashboard => {
            "s switch · w watch · g auto · f refresh · ↑↓ move · ctrl+t theme · q quit"
        }
        Screen::Switch => "enter switch · b best pick · ↑↓ move · esc back",
        Screen::Watch => "enter switch to highlighted · f refresh · ↑↓ move · esc back",
        Screen::Auto if app.auto.adjusting => "←/→ threshold ±1% · enter done",
        Screen::Auto => "l go live / dry-run · t threshold · esc back",
    }
}

fn title(screen: Screen) -> &'static str {
    match screen {
        Screen::Dashboard => "Dashboard",
        Screen::Switch => "Switch accounts",
        Screen::Watch => "Watch",
        Screen::Auto => "Auto-switch",
    }
}

pub fn draw(frame: &mut Frame, app: &App) {
    let p = app.palette();
    let area = frame.area();
    frame.render_widget(
        Block::default().style(Style::default().bg(p.bg).fg(p.fg)),
        area,
    );
    let [header, body, footer] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(3),
        Constraint::Length(1),
    ])
    .areas(area);
    let screen = app.screen();
    let mut head = vec![
        Span::styled(
            " HaryX ",
            Style::default()
                .fg(p.bg)
                .bg(p.accent)
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("  {}", title(screen)),
            Style::default().fg(p.fg).add_modifier(Modifier::BOLD),
        ),
    ];
    if app.busy {
        head.push(Span::styled("   working…", Style::default().fg(p.muted)));
    }
    if !app.status.is_empty() {
        head.push(Span::styled(
            format!("   {}", app.status),
            Style::default().fg(p.muted),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(head)), header);
    frame.render_widget(
        Paragraph::new(Span::styled(
            format!(" {}", hints(screen, app)),
            Style::default().fg(p.muted),
        )),
        footer,
    );
    if screen == Screen::Auto {
        draw_auto(frame, app, body, &p);
    } else {
        draw_accounts(
            frame,
            app,
            body,
            &p,
            screen != Screen::Dashboard || app.cursor_visible,
        );
    }
    if let Some(modal) = &app.modal {
        draw_modal(frame, modal, &p);
    }
}

fn draw_accounts(frame: &mut Frame, app: &App, area: Rect, p: &Palette, show_cursor: bool) {
    let Some(snapshot) = &app.snapshot else {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " Loading accounts…",
                Style::default().fg(p.muted),
            )),
            area,
        );
        return;
    };
    if snapshot.accounts.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled(
                " No managed accounts yet. Sign in to add one: haryx login",
                Style::default().fg(p.muted),
            )),
            area,
        );
        return;
    }
    let now = snapshot.taken_at;
    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_line = 0usize;
    for (i, acc) in snapshot.accounts.iter().enumerate() {
        let mut card = card(acc, area.width, app.thresholds(), now, p);
        if show_cursor && i == app.cursor {
            cursor_line = lines.len();
            if let Some(first) = card.first_mut() {
                first
                    .spans
                    .insert(0, Span::styled("▸", Style::default().fg(p.accent)));
            }
        } else if let Some(first) = card.first_mut() {
            first.spans.insert(0, Span::raw(" "));
        }
        lines.extend(card);
        lines.push(Line::raw(""));
    }
    let height = usize::from(area.height);
    let scroll = cursor_line.saturating_sub(height.saturating_sub(6));
    frame.render_widget(
        Paragraph::new(lines).scroll((u16::try_from(scroll).unwrap_or(0), 0)),
        area,
    );
}

fn draw_auto(frame: &mut Frame, app: &App, area: Rect, p: &Palette) {
    let [top, log] = Layout::vertical([Constraint::Length(3), Constraint::Min(3)]).areas(area);
    let mode = if app.auto.live {
        Span::styled(
            "LIVE",
            Style::default()
                .fg(p.bg)
                .bg(p.bad)
                .add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            "DRY-RUN",
            Style::default()
                .fg(p.bg)
                .bg(p.warn)
                .add_modifier(Modifier::BOLD),
        )
    };
    let s = &app.auto.settings;
    let weekly = s
        .weekly_threshold
        .map(|w| format!(", weekly {w}%"))
        .unwrap_or_default();
    let threshold = Span::styled(
        format!("threshold {}%{weekly}", s.threshold),
        if app.auto.adjusting {
            Style::default()
                .fg(p.accent)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::default().fg(p.fg)
        },
    );
    let info = vec![
        Line::from(vec![
            Span::raw(" "),
            mode,
            Span::raw("  "),
            threshold,
            Span::styled(
                format!(
                    "  · every {:.0}s · strategy {}",
                    s.interval_seconds, s.strategy
                ),
                Style::default().fg(p.muted),
            ),
        ]),
        Line::from(Span::styled(
            " Dry-run evaluates and reports without switching. Go live to let it switch.",
            Style::default().fg(p.muted),
        )),
    ];
    frame.render_widget(Paragraph::new(info), top);
    let height = usize::from(log.height.saturating_sub(2));
    let lines: Vec<Line> = app
        .auto
        .events
        .iter()
        .skip(app.auto.events.len().saturating_sub(height))
        .map(|(kind, text)| {
            let style = match kind.as_str() {
                "switch" => Style::default().fg(p.accent).add_modifier(Modifier::BOLD),
                "error" | "account-quarantined" => Style::default().fg(p.warn),
                "poll" | "no-switch" | "sleep" => Style::default().fg(p.muted),
                _ => Style::default().fg(p.fg),
            };
            Line::from(Span::styled(text.clone(), style))
        })
        .collect();
    frame.render_widget(
        Paragraph::new(lines).block(
            Block::bordered()
                .border_type(BorderType::Rounded)
                .border_style(Style::default().fg(p.track))
                .title(Span::styled(" Events ", Style::default().fg(p.muted))),
        ),
        log,
    );
}

fn draw_modal(frame: &mut Frame, modal: &Modal, p: &Palette) {
    let (title, body, foot) = match modal {
        Modal::Confirm { text, .. } => ("Confirm", text.as_str(), "y yes · n no"),
        Modal::Output { title, body, .. } => (title.as_str(), body.as_str(), "enter close"),
    };
    let area = frame.area();
    let width = area.width.saturating_sub(8).min(72);
    let lines = body.lines().count() as u16 + 4;
    let rect = Rect {
        x: area.x + (area.width.saturating_sub(width)) / 2,
        y: area.y + (area.height.saturating_sub(lines)) / 2,
        width,
        height: lines.min(area.height),
    };
    frame.render_widget(Clear, rect);
    let mut text: Vec<Line> = body.lines().map(|l| Line::raw(l.to_owned())).collect();
    text.push(Line::raw(""));
    text.push(Line::from(Span::styled(foot, Style::default().fg(p.muted))));
    frame.render_widget(
        Paragraph::new(text)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(p.bg).fg(p.fg))
            .block(
                Block::bordered()
                    .border_type(BorderType::Rounded)
                    .border_style(Style::default().fg(p.accent))
                    .title(Span::styled(
                        format!(" {title} "),
                        Style::default().fg(p.accent),
                    )),
            ),
        rect,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn durations_match_upstream() {
        assert_eq!(format_duration(42.0), "42s");
        assert_eq!(format_duration(125.0), "2m");
        assert_eq!(format_duration(3600.0), "1h");
        assert_eq!(format_duration(3660.0), "1h 1m");
        assert_eq!(format_duration(90000.0), "1d 1h");
        assert_eq!(format_duration(86400.0), "1d");
    }

    #[test]
    fn rows_cover_windows_scoped_and_probe() {
        let usage = json!({
            "five_hour": {"pct": 42.0, "resets_at": null},
            "seven_day": {"pct": 10.0},
            "scoped": [{"name": "Fable", "pct": 100.0}],
            "rejected": {"resets_at": null},
        });
        let rows = usage_rows(Some(&usage), 0.0, None);
        let labels: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
        assert_eq!(labels, ["5h", "7d", "Fable", "probe"]);
        assert_eq!(rows[2].2, "(!)");
        assert_eq!(rows[3].2, "rejected (!)");
    }

    #[test]
    fn bar_marks_the_threshold_and_half_cells() {
        let spans = bar(Some(45.0), 10, false, Some(90.0), &Palette::DARK);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "━━━━╸────┃");
    }

    #[test]
    fn dashboard_renders_cards_bars_and_hints() {
        use haryx_core::models::{AccountSnapshot, AccountsSnapshot};
        use haryx_core::settings::AutoSwitchSettings;
        use haryx_core::usage_store::UsageEntry;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut app = App::new(AutoSwitchSettings::default(), false, false);
        app.snapshot = Some(AccountsSnapshot {
            active_number: Some("1".into()),
            accounts: vec![AccountSnapshot {
                number: "1".into(),
                email: "a@example.com".into(),
                is_active: true,
                kind: "oauth".into(),
                usage: UsageEntry {
                    last_good: Some(
                        json!({"five_hour": {"pct": 42.0}, "seven_day": {"pct": 10.0}}),
                    ),
                    age_s: Some(1.0),
                    ..Default::default()
                },
                ..Default::default()
            }],
            taken_at: 0.0,
        });
        let mut terminal = Terminal::new(TestBackend::new(100, 12)).unwrap();
        terminal.draw(|f| draw(f, &app)).unwrap();
        let buffer = terminal.backend().buffer();
        let text: String = (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer[(x, y)].symbol().to_owned())
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("HaryX"), "{text}");
        assert!(
            text.contains("a@example.com  [personal]   ● active"),
            "{text}"
        );
        assert!(text.contains(" 42%"), "{text}");
        assert!(text.contains("s switch · w watch"), "{text}");
    }
}
