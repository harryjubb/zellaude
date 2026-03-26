use crate::state::{
    unix_now, unix_now_ms, Activity, ClickRegion, FlashMode, MenuAction, MenuClickRegion,
    NotifyMode, SessionInfo, SettingKey, State, ViewMode,
};
use std::collections::HashMap;
use std::fmt::Write;
use std::io::Write as IoWrite;
use zellij_tile::prelude::{InputMode, TabInfo};

pub struct Style {
    pub symbol: &'static str,
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

pub fn activity_priority(activity: &Activity) -> u8 {
    match activity {
        Activity::Waiting => 8,
        Activity::Tool(_) => 7,
        Activity::Thinking => 6,
        Activity::Prompting => 5,
        Activity::Notification => 4,
        Activity::Init => 3,
        Activity::Done => 2,
        Activity::AgentDone => 1,
        Activity::Idle => 0,
    }
}

pub fn activity_style(activity: &Activity) -> Style {
    match activity {
        Activity::Init => Style { symbol: "◆", r: 180, g: 175, b: 195 },
        Activity::Thinking => Style { symbol: "●", r: 180, g: 140, b: 255 },
        Activity::Tool(name) => {
            let symbol = match name.as_str() {
                "Bash" => "⚡",
                "Read" | "Glob" | "Grep" => "◉",
                "Edit" | "Write" => "✎",
                "Task" => "⊜",
                "WebSearch" | "WebFetch" => "◈",
                _ => "⚙",
            };
            Style { symbol, r: 255, g: 170, b: 50 }
        }
        Activity::Prompting => Style { symbol: "▶", r: 80, g: 200, b: 120 },
        Activity::Waiting => Style { symbol: "⚠", r: 255, g: 60, b: 60 },
        Activity::Notification => Style { symbol: "◇", r: 200, g: 200, b: 100 },
        Activity::Done => Style { symbol: "✓", r: 80, g: 200, b: 120 },
        Activity::AgentDone => Style { symbol: "✓", r: 80, g: 180, b: 100 },
        Activity::Idle => Style { symbol: "○", r: 180, g: 175, b: 195 },
    }
}

pub fn fg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[38;2;{r};{g};{b}m")
}

pub fn bg(r: u8, g: u8, b: u8) -> String {
    format!("\x1b[48;2;{r};{g};{b}m")
}

/// Write foreground color ANSI escape directly to buffer (avoids heap allocation).
pub fn write_fg(buf: &mut String, r: u8, g: u8, b: u8) {
    let _ = write!(buf, "\x1b[38;2;{r};{g};{b}m");
}

/// Write background color ANSI escape directly to buffer (avoids heap allocation).
pub fn write_bg(buf: &mut String, r: u8, g: u8, b: u8) {
    let _ = write!(buf, "\x1b[48;2;{r};{g};{b}m");
}

pub fn display_width(s: &str) -> usize {
    s.chars().count()
}

pub const RESET: &str = "\x1b[0m";
pub const BOLD: &str = "\x1b[1m";
pub const ELAPSED_THRESHOLD: u64 = 30;
pub const SEPARATOR: &str = "\u{e0b0}";

pub type Color = (u8, u8, u8);
pub const BAR_BG: Color = (30, 30, 46);
pub const PREFIX_BG: Color = (60, 50, 80);
pub const PREFIX_BG_SETTINGS: Color = (100, 70, 140);
pub const TAB_BG_ACTIVE: Color = (140, 100, 200);
pub const TAB_BG_INACTIVE: Color = (80, 75, 110);
pub const FLASH_BG_BRIGHT: Color = (80, 80, 30);

/// Write a powerline arrow: fg=from_bg, bg=to_bg, then separator char.
pub fn arrow(buf: &mut String, col: &mut usize, from: Color, to: Color) {
    write_fg(buf, from.0, from.1, from.2);
    write_bg(buf, to.0, to.1, to.2);
    buf.push_str(SEPARATOR);
    *col += 1;
}

pub fn format_elapsed(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

pub fn mode_style(mode: InputMode) -> (Color, &'static str) {
    match mode {
        InputMode::Normal => ((80, 200, 120), "NORMAL"),
        InputMode::Locked => ((255, 80, 80), "LOCKED"),
        InputMode::Pane => ((80, 180, 255), "PANE"),
        InputMode::Tab => ((180, 140, 255), "TAB"),
        InputMode::Resize => ((255, 170, 50), "RESIZE"),
        InputMode::Move => ((255, 170, 50), "MOVE"),
        InputMode::Scroll => ((200, 200, 100), "SCROLL"),
        InputMode::EnterSearch => ((200, 200, 100), "SEARCH"),
        InputMode::Search => ((200, 200, 100), "SEARCH"),
        InputMode::RenameTab => ((200, 200, 100), "RENAME"),
        InputMode::RenamePane => ((200, 200, 100), "RENAME"),
        InputMode::Session => ((180, 140, 255), "SESSION"),
        InputMode::Prompt => ((80, 200, 120), "PROMPT"),
        InputMode::Tmux => ((80, 200, 120), "TMUX"),
    }
}

pub fn render_status_bar(state: &mut State, _rows: usize, cols: usize) {
    state.click_regions.clear();
    state.menu_click_regions.clear();

    let mut buf = String::with_capacity(cols * 4);
    // Terminal setup for a 1-row status bar:
    //  \x1b[H     — cursor home (prevent scroll from cursor at end-of-line)
    //  \x1b[?7l   — disable auto-wrap (clip overflow instead of scroll)
    //  \x1b[?25l  — hide cursor
    buf.push_str("\x1b[H\x1b[?7l\x1b[?25l");
    let mut bar_bg_str = String::with_capacity(20);
    write_bg(&mut bar_bg_str, BAR_BG.0, BAR_BG.1, BAR_BG.2);

    // Bail early if terminal is too narrow
    if cols < 5 {
        let _ = write!(buf, "{bar_bg_str}{:width$}{RESET}", "", width = cols);
        print!("{buf}");
        let _ = std::io::stdout().flush();
        return;
    }

    let prefix_bg = if state.view_mode == ViewMode::Settings {
        PREFIX_BG_SETTINGS
    } else {
        PREFIX_BG
    };

    // Build prefix: " Zellaude (session) MODE "
    let (mode_bg, mode_text) = mode_style(state.input_mode);
    let session_part = match state.zellij_session_name.as_deref() {
        Some(name) => format!(" ({name})"),
        None => String::new(),
    };
    let prefix_text = format!(" Zellaude{session_part} ");
    let prefix_width = display_width(&prefix_text);
    let mode_pill_width = 1 + mode_text.len() + 1; // space + text + space
    let total_prefix_width = prefix_width + mode_pill_width;

    // Render prefix segment (truncate if wider than cols)
    let mut col;
    if total_prefix_width <= cols {
        write_bg(&mut buf, prefix_bg.0, prefix_bg.1, prefix_bg.2);
        write_fg(&mut buf, 255, 255, 255);
        let _ = write!(buf, "{BOLD}{prefix_text}{RESET}");
        write_bg(&mut buf, mode_bg.0, mode_bg.1, mode_bg.2);
        write_fg(&mut buf, 30, 30, 46);
        let _ = write!(buf, "{BOLD} {mode_text} {RESET}");
        col = total_prefix_width;
    } else if prefix_width <= cols {
        // Fit the name part but skip mode pill
        write_bg(&mut buf, prefix_bg.0, prefix_bg.1, prefix_bg.2);
        write_fg(&mut buf, 255, 255, 255);
        let _ = write!(buf, "{BOLD}{prefix_text}{RESET}");
        col = prefix_width;
    } else {
        // Even name doesn't fit — just show what we can
        let avail = cols.saturating_sub(2); // leave room for fill
        let short: String = prefix_text.chars().take(avail).collect();
        write_bg(&mut buf, prefix_bg.0, prefix_bg.1, prefix_bg.2);
        write_fg(&mut buf, 255, 255, 255);
        let _ = write!(buf, "{BOLD}{short}{RESET}");
        col = display_width(&short);
    }
    state.prefix_click_region = Some((0, col));

    let last_prefix_bg = if total_prefix_width <= cols { mode_bg } else { prefix_bg };
    let prefix_used = col;

    if col < cols {
        match state.view_mode {
            ViewMode::Normal => {
                render_tabs(state, &mut buf, &mut col, cols, last_prefix_bg, prefix_used);
            }
            ViewMode::Settings => {
                arrow(&mut buf, &mut col, last_prefix_bg, BAR_BG);
                let _ = write!(buf, "{bar_bg_str}");
                render_settings_menu(state, &mut buf, &mut col);
            }
        }
    }

    // Fill remaining width with bar background — never exceed cols
    if col < cols {
        let remaining = cols - col;
        let _ = write!(buf, "{bar_bg_str}{:width$}", "", width = remaining);
    }
    let _ = write!(buf, "{RESET}");

    print!("{buf}");
    let _ = std::io::stdout().flush();
}

pub fn render_tabs(
    state: &mut State,
    buf: &mut String,
    col: &mut usize,
    cols: usize,
    prefix_bg: Color,
    prefix_width: usize,
) {
    let now_s = unix_now();
    let now_ms = unix_now_ms();

    // Sort tabs by position
    let mut tabs: Vec<&TabInfo> = state.tabs.iter().collect();
    tabs.sort_by_key(|t| t.position);

    let count = tabs.len();
    if count == 0 {
        arrow(buf, col, prefix_bg, BAR_BG);
        return;
    }

    // Pre-group sessions by tab index — O(S) instead of 3x O(T*S) scans
    let mut sessions_by_tab: HashMap<usize, Vec<&SessionInfo>> = HashMap::new();
    for session in state.sessions.values() {
        if let Some(tab_idx) = session.tab_index {
            sessions_by_tab.entry(tab_idx).or_default().push(session);
        }
    }
    let empty_sessions: Vec<&SessionInfo> = Vec::new();

    // For each tab, find the best (highest-priority) Claude session
    let best_sessions: Vec<Option<&SessionInfo>> = tabs
        .iter()
        .map(|tab| {
            sessions_by_tab
                .get(&tab.position)
                .unwrap_or(&empty_sessions)
                .iter()
                .max_by_key(|s| activity_priority(&s.activity))
                .copied()
        })
        .collect();

    // Pre-compute elapsed strings (only for Claude tabs)
    let elapsed_strs: Vec<Option<String>> = best_sessions
        .iter()
        .map(|session: &Option<&SessionInfo>| {
            if !state.settings.elapsed_time {
                return None;
            }
            session.and_then(|s| {
                let elapsed = now_s.saturating_sub(s.last_event_ts);
                if elapsed >= ELAPSED_THRESHOLD {
                    Some(format_elapsed(elapsed))
                } else {
                    None
                }
            })
        })
        .collect();

    // Compute overhead: varies per tab type
    let total_elapsed_width: usize = elapsed_strs
        .iter()
        .map(|e: &Option<String>| e.as_ref().map_or(0, |s| s.len() + 1))
        .sum();
    let per_tab_overhead: usize = best_sessions
        .iter()
        .map(|s: &Option<&SessionInfo>| if s.is_some() { 4 } else { 2 })
        .sum();
    let overhead = prefix_width + 2 * count + per_tab_overhead + total_elapsed_width;
    let max_name_len = if overhead < cols {
        ((cols - overhead) / count).min(20)
    } else {
        0
    };

    let mut prev_bg = prefix_bg;

    for (i, tab) in tabs.iter().enumerate() {
        // Stop if we'd overflow — need room for at least arrow + closing arrow
        let arrows_needed = if prev_bg == prefix_bg { 1 } else { 2 };
        if *col + arrows_needed + 3 > cols {
            break;
        }

        let session = best_sessions[i];
        let is_claude = session.is_some();
        let tab_name = &tab.name;

        // Truncate name
        let char_count = tab_name.chars().count();
        let truncated = if max_name_len == 0 {
            String::new()
        } else if char_count > max_name_len {
            let s: String = tab_name.chars().take(max_name_len.saturating_sub(1)).collect();
            format!("{s}…")
        } else {
            tab_name.to_string()
        };

        // Check flash for any session in this tab
        let tab_sessions = sessions_by_tab.get(&tab.position).unwrap_or(&empty_sessions);
        let is_flash_bright = tab_sessions.iter().any(|s| {
            state
                .flash_deadlines
                .get(&s.pane_id)
                .map(|&deadline| now_ms < deadline && (now_ms / 250).is_multiple_of(2))
                .unwrap_or(false)
        });

        let is_active = tab.active;

        // Pick tab background color
        let tab_bg = if is_flash_bright {
            FLASH_BG_BRIGHT
        } else if is_active {
            TAB_BG_ACTIVE
        } else {
            TAB_BG_INACTIVE
        };

        // Arrow: close previous segment, then open this tab
        if prev_bg == prefix_bg {
            arrow(buf, col, prev_bg, tab_bg);
        } else {
            arrow(buf, col, prev_bg, BAR_BG);
            arrow(buf, col, BAR_BG, tab_bg);
        }

        let mut tab_bg_str = String::with_capacity(20);
        write_bg(&mut tab_bg_str, tab_bg.0, tab_bg.1, tab_bg.2);
        let region_start = *col;

        if is_claude {
            let s = session.unwrap();
            let style = activity_style(&s.activity);

            // Leading space
            let _ = write!(buf, "{tab_bg_str} ");
            *col += 1;

            // Symbol with foreground color
            if is_flash_bright {
                write_fg(buf, 255, 255, 80);
            } else {
                write_fg(buf, style.r, style.g, style.b);
            }
            let _ = write!(buf, "{}", style.symbol);
            *col += display_width(style.symbol);

            // Space + name
            if !truncated.is_empty() {
                let name_bold = is_flash_bright || is_active;
                let bold_str = if name_bold { BOLD } else { "" };
                buf.push(' ');
                buf.push_str(bold_str);
                if is_flash_bright {
                    write_fg(buf, 255, 255, 80);
                } else if is_active {
                    write_fg(buf, 255, 255, 255);
                } else {
                    write_fg(buf, 120, 220, 220);
                }
                let _ = write!(buf, "{truncated}{RESET}{tab_bg_str}");
                *col += 1 + display_width(&truncated);
            }

            // Elapsed suffix
            if let Some(ref es) = elapsed_strs[i] {
                if *col + 1 + es.len() + 1 < cols {
                    buf.push(' ');
                    write_fg(buf, 165, 160, 180);
                    buf.push_str(es);
                    *col += 1 + es.len();
                }
            }

            // Fullscreen indicator
            if tab.is_fullscreen_active && *col + 3 < cols {
                buf.push(' ');
                write_fg(buf, 255, 200, 60);
                let _ = write!(buf, "F{RESET}{tab_bg_str}");
                *col += 2;
            }

            // Trailing space
            buf.push(' ');
            *col += 1;

            // Click region: if any session is waiting, use its pane_id for focus
            let waiting_session = tab_sessions
                .iter()
                .find(|s| matches!(s.activity, Activity::Waiting))
                .copied();

            state.click_regions.push(ClickRegion {
                start_col: region_start,
                end_col: *col,
                tab_index: tab.position,
                pane_id: waiting_session.map_or(0, |s| s.pane_id),
                is_waiting: waiting_session.is_some(),
            });
        } else {
            // Non-Claude tab: dimmer, no symbol

            // Leading space
            let _ = write!(buf, "{tab_bg_str} ");
            *col += 1;

            // Name only (no symbol)
            if !truncated.is_empty() {
                let bold_str = if is_active { BOLD } else { "" };
                buf.push_str(bold_str);
                if is_active {
                    write_fg(buf, 220, 215, 230);
                } else {
                    write_fg(buf, 170, 165, 185);
                }
                let _ = write!(buf, "{truncated}{RESET}{tab_bg_str}");
                *col += display_width(&truncated);
            }

            // Fullscreen indicator
            if tab.is_fullscreen_active && *col + 3 < cols {
                buf.push(' ');
                write_fg(buf, 255, 200, 60);
                let _ = write!(buf, "F{RESET}{tab_bg_str}");
                *col += 2;
            }

            // Trailing space
            buf.push(' ');
            *col += 1;

            state.click_regions.push(ClickRegion {
                start_col: region_start,
                end_col: *col,
                tab_index: tab.position,
                pane_id: 0,
                is_waiting: false,
            });
        }

        prev_bg = tab_bg;
    }

    // Arrow from last tab → bar background (only if we rendered any tabs)
    if prev_bg != prefix_bg || count > 0 {
        arrow(buf, col, prev_bg, BAR_BG);
    }
}

pub fn notify_mode_label(mode: NotifyMode) -> (&'static str, &'static str, String, String) {
    match mode {
        NotifyMode::Always => ("●", "Notify: always", fg(80, 200, 120), fg(255, 255, 255)),
        NotifyMode::Unfocused => ("◐", "Notify: unfocused", fg(255, 200, 60), fg(255, 200, 60)),
        NotifyMode::Never => ("○", "Notify: off", fg(100, 100, 100), fg(100, 100, 100)),
    }
}

pub fn flash_mode_label(mode: FlashMode) -> (&'static str, &'static str, String, String) {
    match mode {
        FlashMode::Persist => ("●", "Flash: persist", fg(80, 200, 120), fg(255, 255, 255)),
        FlashMode::Once => ("◐", "Flash: brief", fg(255, 200, 60), fg(255, 200, 60)),
        FlashMode::Off => ("○", "Flash: off", fg(100, 100, 100), fg(100, 100, 100)),
    }
}

/// Render a three-state toggle and register its click region.
/// Assumes the caller has already set the desired background color.
#[allow(clippy::too_many_arguments)]
pub fn render_tristate(
    buf: &mut String,
    col: &mut usize,
    state_regions: &mut Vec<MenuClickRegion>,
    key: SettingKey,
    symbol: &str,
    label: &str,
    sym_color: &str,
    label_color: &str,
) {
    let region_start = *col;
    let width = display_width(symbol) + 1 + label.len();
    *col += width;

    state_regions.push(MenuClickRegion {
        start_col: region_start,
        end_col: *col,
        action: MenuAction::ToggleSetting(key),
    });

    let _ = write!(buf, "{sym_color}{symbol} {label_color}{label}");
}

pub fn render_settings_menu(state: &mut State, buf: &mut String, col: &mut usize) {
    // Leading space after arrow
    let _ = write!(buf, " ");
    *col += 1;

    // --- Notifications (three-state) ---
    {
        let (symbol, label, sym_color, label_color) =
            notify_mode_label(state.settings.notifications);
        render_tristate(
            buf, col, &mut state.menu_click_regions,
            SettingKey::Notifications, symbol, label, &sym_color, &label_color,
        );
    }

    // --- Flash (three-state) ---
    {
        let _ = write!(buf, "  ");
        *col += 2;
        let (symbol, label, sym_color, label_color) =
            flash_mode_label(state.settings.flash);
        render_tristate(
            buf, col, &mut state.menu_click_regions,
            SettingKey::Flash, symbol, label, &sym_color, &label_color,
        );
    }

    // --- Elapsed time (bool) ---
    {
        let _ = write!(buf, "  ");
        *col += 2;
        let enabled = state.settings.elapsed_time;
        let (symbol, sym_color, label_color) = if enabled {
            ("●", fg(80, 200, 120), fg(255, 255, 255))
        } else {
            ("○", fg(100, 100, 100), fg(100, 100, 100))
        };
        let label = if enabled { "Elapsed time: on" } else { "Elapsed time: off" };
        render_tristate(
            buf, col, &mut state.menu_click_regions,
            SettingKey::ElapsedTime, symbol, label, &sym_color, &label_color,
        );
    }

    // Close button
    let _ = write!(buf, "  ");
    *col += 2;
    let close_start = *col;
    write_fg(buf, 255, 60, 60);
    buf.push('×');
    *col += 1;

    state.menu_click_regions.push(MenuClickRegion {
        start_col: close_start,
        end_col: *col,
        action: MenuAction::CloseMenu,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- format_elapsed ---

    #[test]
    fn format_elapsed_seconds() {
        assert_eq!(format_elapsed(0), "0s");
        assert_eq!(format_elapsed(1), "1s");
        assert_eq!(format_elapsed(59), "59s");
    }

    #[test]
    fn format_elapsed_minutes() {
        assert_eq!(format_elapsed(60), "1m");
        assert_eq!(format_elapsed(119), "1m");
        assert_eq!(format_elapsed(120), "2m");
        assert_eq!(format_elapsed(3599), "59m");
    }

    #[test]
    fn format_elapsed_hours() {
        assert_eq!(format_elapsed(3600), "1h");
        assert_eq!(format_elapsed(7200), "2h");
        assert_eq!(format_elapsed(86400), "24h");
    }

    // --- activity_priority ---

    #[test]
    fn activity_priority_ordering() {
        assert!(activity_priority(&Activity::Waiting) > activity_priority(&Activity::Tool("x".into())));
        assert!(activity_priority(&Activity::Tool("x".into())) > activity_priority(&Activity::Thinking));
        assert!(activity_priority(&Activity::Thinking) > activity_priority(&Activity::Prompting));
        assert!(activity_priority(&Activity::Prompting) > activity_priority(&Activity::Notification));
        assert!(activity_priority(&Activity::Notification) > activity_priority(&Activity::Init));
        assert!(activity_priority(&Activity::Init) > activity_priority(&Activity::Done));
        assert!(activity_priority(&Activity::Done) > activity_priority(&Activity::AgentDone));
        assert!(activity_priority(&Activity::AgentDone) > activity_priority(&Activity::Idle));
    }

    #[test]
    fn activity_priority_idle_is_zero() {
        assert_eq!(activity_priority(&Activity::Idle), 0);
    }

    #[test]
    fn activity_priority_waiting_is_highest() {
        assert_eq!(activity_priority(&Activity::Waiting), 8);
    }

    // --- activity_style ---

    #[test]
    fn activity_style_all_variants_return_nonempty_symbol() {
        let variants = vec![
            Activity::Init,
            Activity::Thinking,
            Activity::Tool("Bash".to_string()),
            Activity::Tool("Read".to_string()),
            Activity::Tool("Edit".to_string()),
            Activity::Tool("WebSearch".to_string()),
            Activity::Tool("Task".to_string()),
            Activity::Tool("Unknown".to_string()),
            Activity::Prompting,
            Activity::Waiting,
            Activity::Notification,
            Activity::Done,
            Activity::AgentDone,
            Activity::Idle,
        ];
        for v in &variants {
            let style = activity_style(v);
            assert!(!style.symbol.is_empty(), "empty symbol for {v:?}");
        }
    }

    #[test]
    fn activity_style_tool_dispatch() {
        assert_eq!(activity_style(&Activity::Tool("Bash".into())).symbol, "⚡");
        assert_eq!(activity_style(&Activity::Tool("Read".into())).symbol, "◉");
        assert_eq!(activity_style(&Activity::Tool("Glob".into())).symbol, "◉");
        assert_eq!(activity_style(&Activity::Tool("Grep".into())).symbol, "◉");
        assert_eq!(activity_style(&Activity::Tool("Edit".into())).symbol, "✎");
        assert_eq!(activity_style(&Activity::Tool("Write".into())).symbol, "✎");
        assert_eq!(activity_style(&Activity::Tool("Task".into())).symbol, "⊜");
        assert_eq!(activity_style(&Activity::Tool("WebSearch".into())).symbol, "◈");
        assert_eq!(activity_style(&Activity::Tool("WebFetch".into())).symbol, "◈");
        assert_eq!(activity_style(&Activity::Tool("Other".into())).symbol, "⚙");
    }

    // --- mode_style ---

    #[test]
    fn mode_style_all_variants() {
        let modes = [
            InputMode::Normal,
            InputMode::Locked,
            InputMode::Pane,
            InputMode::Tab,
            InputMode::Resize,
            InputMode::Move,
            InputMode::Scroll,
            InputMode::EnterSearch,
            InputMode::Search,
            InputMode::RenameTab,
            InputMode::RenamePane,
            InputMode::Session,
            InputMode::Prompt,
            InputMode::Tmux,
        ];
        for mode in modes {
            let (color, label) = mode_style(mode);
            assert!(!label.is_empty(), "empty label for {mode:?}");
            // Colors should be non-zero (at least one component)
            assert!(color.0 > 0 || color.1 > 0 || color.2 > 0);
        }
    }

    #[test]
    fn mode_style_normal_is_green() {
        let (color, label) = mode_style(InputMode::Normal);
        assert_eq!(label, "NORMAL");
        assert_eq!(color, (80, 200, 120));
    }

    // --- fg / bg ---

    #[test]
    fn fg_produces_correct_ansi() {
        assert_eq!(fg(255, 0, 128), "\x1b[38;2;255;0;128m");
    }

    #[test]
    fn bg_produces_correct_ansi() {
        assert_eq!(bg(30, 30, 46), "\x1b[48;2;30;30;46m");
    }

    // --- display_width ---

    #[test]
    fn display_width_ascii() {
        assert_eq!(display_width("hello"), 5);
        assert_eq!(display_width(""), 0);
    }

    #[test]
    fn display_width_unicode() {
        assert_eq!(display_width("●"), 1);
        assert_eq!(display_width("⚡"), 1);
        assert_eq!(display_width("✎"), 1);
    }

    // --- arrow ---

    #[test]
    fn arrow_writes_separator_and_increments_col() {
        let mut buf = String::new();
        let mut col = 0;
        arrow(&mut buf, &mut col, (255, 0, 0), (0, 255, 0));
        assert_eq!(col, 1);
        assert!(buf.contains(SEPARATOR));
        assert!(buf.contains("38;2;255;0;0")); // fg from
        assert!(buf.contains("48;2;0;255;0")); // bg to
    }

    // --- notify_mode_label ---

    #[test]
    fn notify_mode_labels() {
        let (sym, label, _, _) = notify_mode_label(NotifyMode::Always);
        assert_eq!(sym, "●");
        assert!(label.contains("always"));

        let (sym, label, _, _) = notify_mode_label(NotifyMode::Unfocused);
        assert_eq!(sym, "◐");
        assert!(label.contains("unfocused"));

        let (sym, label, _, _) = notify_mode_label(NotifyMode::Never);
        assert_eq!(sym, "○");
        assert!(label.contains("off"));
    }

    // --- flash_mode_label ---

    #[test]
    fn flash_mode_labels() {
        let (sym, label, _, _) = flash_mode_label(FlashMode::Persist);
        assert_eq!(sym, "●");
        assert!(label.contains("persist"));

        let (sym, label, _, _) = flash_mode_label(FlashMode::Once);
        assert_eq!(sym, "◐");
        assert!(label.contains("brief"));

        let (sym, label, _, _) = flash_mode_label(FlashMode::Off);
        assert_eq!(sym, "○");
        assert!(label.contains("off"));
    }
}
