use std::collections::VecDeque;

use bevy::{input::mouse::MouseWheel, prelude::*};

// ── Tunables ──────────────────────────────────────────────────────────────────

/// Maximum number of messages retained in the log buffer.
const MAX_LOG_SIZE: usize = 100;
/// Number of lines shown in the HUD panel at once.
pub const LOG_LINES: usize = 5;

// Warm gold on dark — matches the rest of the HUD palette.
const FG: (f32, f32, f32)       = (0.95, 0.85, 0.55);
const BG: (f32, f32, f32, f32)  = (0.05, 0.05, 0.12, 0.80);

// ── Resource ──────────────────────────────────────────────────────────────────

/// Holds all past game messages and the current scroll position.
#[derive(Resource, Default)]
pub struct GameLog {
    messages:      VecDeque<String>,
    /// Lines scrolled up from the bottom.  0 = most recent messages visible.
    scroll_offset: usize,
}

impl GameLog {
    /// Append a message and reset the view to the bottom.
    pub fn push(&mut self, msg: impl Into<String>) {
        self.messages.push_back(msg.into());
        if self.messages.len() > MAX_LOG_SIZE {
            self.messages.pop_front();
        }
        self.scroll_offset = 0;
    }

    /// The largest valid `scroll_offset` (can't scroll past the first message).
    fn max_scroll(&self) -> usize {
        self.messages.len().saturating_sub(LOG_LINES)
    }

    /// Returns the `LOG_LINES` lines visible at the current scroll offset,
    /// oldest-first (index 0 = top of panel, index LOG_LINES-1 = bottom).
    /// Short results are left-padded with empty strings.
    fn visible_lines(&self) -> [&str; LOG_LINES] {
        let len    = self.messages.len();
        let bottom = len.saturating_sub(self.scroll_offset);
        let top    = bottom.saturating_sub(LOG_LINES);
        let mut lines = [""; LOG_LINES];
        // The range may be shorter than LOG_LINES; offset into the array so
        // that the newest line always lands at index LOG_LINES-1.
        let range_len = bottom - top;
        let start_idx = LOG_LINES - range_len;
        for (i, msg) in self.messages.range(top..bottom).enumerate() {
            lines[start_idx + i] = msg.as_str();
        }
        lines
    }
}

// ── Event ─────────────────────────────────────────────────────────────────────

/// Fire this event from any system to append a line to the game log.
#[derive(Event)]
pub struct GameMessage(pub String);

impl GameMessage {
    pub fn new(s: impl Into<String>) -> Self {
        Self(s.into())
    }
}

// ── HUD component ─────────────────────────────────────────────────────────────

/// Marks one of the `LOG_LINES` text nodes in the log panel.
/// Index 0 = topmost (oldest visible), `LOG_LINES - 1` = bottommost (newest).
#[derive(Component)]
struct LogLine(usize);

// ── Startup ───────────────────────────────────────────────────────────────────

fn spawn_log_panel(mut commands: Commands) {
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left:   Val::Px(20.0),
                bottom: Val::Px(100.0),
                width:  Val::Px(320.0),
                flex_direction: FlexDirection::Column,
                padding: UiRect::all(Val::Px(8.0)),
                row_gap: Val::Px(2.0),
                ..Default::default()
            },
            BackgroundColor(Color::srgba(BG.0, BG.1, BG.2, BG.3)),
            BorderRadius::all(Val::Px(6.0)),
        ))
        .with_children(|panel| {
            for i in 0..LOG_LINES {
                panel.spawn((
                    LogLine(i),
                    Text::new(""),
                    TextFont  { font_size: 13.0, ..Default::default() },
                    TextColor(Color::srgba(FG.0, FG.1, FG.2, 0.85)),
                ));
            }
        });
}

// ── Update systems ────────────────────────────────────────────────────────────

fn receive_messages(mut events: EventReader<GameMessage>, mut log: ResMut<GameLog>) {
    for msg in events.read() {
        log.push(msg.0.clone());
    }
}

fn scroll_log(mut wheel: EventReader<MouseWheel>, mut log: ResMut<GameLog>) {
    for ev in wheel.read() {
        if ev.y > 0.0 {
            // Scroll up → show older messages.
            let max = log.max_scroll();
            log.scroll_offset = (log.scroll_offset + 1).min(max);
        } else if ev.y < 0.0 {
            // Scroll down → show newer messages.
            log.scroll_offset = log.scroll_offset.saturating_sub(1);
        }
    }
}

fn update_log_hud(log: Res<GameLog>, mut line_q: Query<(&LogLine, &mut Text)>) {
    if !log.is_changed() {
        return;
    }
    let lines = log.visible_lines();
    for (LogLine(i), mut text) in &mut line_q {
        **text = lines[*i].to_string();
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct GameLogPlugin;

impl Plugin for GameLogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameLog>()
            .add_event::<GameMessage>()
            .add_systems(Startup, spawn_log_panel)
            .add_systems(
                Update,
                (
                    receive_messages,
                    scroll_log,
                    update_log_hud.after(receive_messages).after(scroll_log),
                ),
            );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn log_with(msgs: &[&str]) -> GameLog {
        let mut log = GameLog::default();
        for m in msgs { log.push(*m); }
        log
    }

    #[test]
    fn empty_log_returns_blank_lines() {
        let log = GameLog::default();
        assert!(log.visible_lines().iter().all(|l| l.is_empty()));
    }

    #[test]
    fn single_message_appears_at_bottom() {
        let log = log_with(&["hello"]);
        let lines = log.visible_lines();
        assert_eq!(lines[LOG_LINES - 1], "hello");
        assert!(lines[..LOG_LINES - 1].iter().all(|l| l.is_empty()));
    }

    #[test]
    fn five_messages_fill_all_lines() {
        let msgs = ["a", "b", "c", "d", "e"];
        let log = log_with(&msgs);
        assert_eq!(log.visible_lines(), msgs);
    }

    #[test]
    fn sixth_message_pushes_first_off_top() {
        let log = log_with(&["a", "b", "c", "d", "e", "f"]);
        let lines = log.visible_lines();
        assert_eq!(lines[0], "b");
        assert_eq!(lines[LOG_LINES - 1], "f");
    }

    #[test]
    fn new_message_resets_scroll_to_bottom() {
        let mut log = log_with(&["a", "b", "c", "d", "e", "f"]);
        log.scroll_offset = 1;
        log.push("g");
        assert_eq!(log.scroll_offset, 0);
        assert_eq!(log.visible_lines()[LOG_LINES - 1], "g");
    }

    #[test]
    fn scroll_up_reveals_older_messages() {
        let log_msgs = ["a", "b", "c", "d", "e", "f"];
        let mut log = log_with(&log_msgs);
        let max = log.max_scroll();
        log.scroll_offset = (log.scroll_offset + 1).min(max);
        let lines = log.visible_lines();
        assert_eq!(lines[0], "a");
        assert_eq!(lines[LOG_LINES - 1], "e");
    }

    #[test]
    fn scroll_cannot_exceed_max() {
        let mut log = log_with(&["x"]);
        let max = log.max_scroll();
        log.scroll_offset = max + 999;
        // max_scroll is 0 for a single message, so clamp should keep it at 0.
        assert_eq!(max, 0);
    }

    #[test]
    fn log_capped_at_max_size() {
        let mut log = GameLog::default();
        for i in 0..MAX_LOG_SIZE + 5 {
            log.push(format!("msg {i}"));
        }
        assert_eq!(log.messages.len(), MAX_LOG_SIZE);
    }
}
