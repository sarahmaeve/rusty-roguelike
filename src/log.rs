use std::collections::VecDeque;

use bevy::{input::mouse::MouseWheel, prelude::*};

// ── Tunables ──────────────────────────────────────────────────────────────────

/// Maximum messages retained in the ring buffer.
const MAX_LOG_SIZE: usize = 100;
/// Lines shown in the HUD panel at once.
pub const LOG_LINES: usize = 5;
/// Opacity lost per second as messages age (newest = 1.0, floor = `OPACITY_MIN`).
const OPACITY_FADE_PER_SEC: f32 = 0.10;
/// Minimum opacity for old log lines.
const OPACITY_MIN: f32 = 0.50;
/// Seconds an alert is shown at full opacity before fading.
const ALERT_SHOW_SECS: f32 = 5.0;
/// Seconds the alert takes to fade out after `ALERT_SHOW_SECS`.
const ALERT_FADE_SECS: f32 = 1.0;
/// Base BG alpha for the alert panel (full-visibility).
const ALERT_BG_ALPHA: f32 = 0.88;

// Warm gold on dark — matches the rest of the HUD palette.
const FG: (f32, f32, f32)      = (0.95, 0.85, 0.55);
const BG: (f32, f32, f32, f32) = (0.05, 0.05, 0.12, 0.80);

// ── Internal log entry ────────────────────────────────────────────────────────

struct LogEntry {
    text:      String,
    /// `Time::elapsed_secs()` at the moment the message was pushed.
    timestamp: f32,
}

// ── Resource: GameLog ─────────────────────────────────────────────────────────

/// Holds all past game messages and the current HUD scroll position.
#[derive(Resource, Default)]
pub struct GameLog {
    messages:      VecDeque<LogEntry>,
    /// Lines scrolled up from the bottom.  0 = newest messages visible.
    scroll_offset: usize,
}

impl GameLog {
    /// Append a message recorded at `timestamp` (seconds since app start) and
    /// reset the view to the bottom so the new line is immediately visible.
    pub fn push(&mut self, text: impl Into<String>, timestamp: f32) {
        self.messages.push_back(LogEntry { text: text.into(), timestamp });
        if self.messages.len() > MAX_LOG_SIZE {
            self.messages.pop_front();
        }
        self.scroll_offset = 0;
    }

    /// Largest valid scroll offset (cannot scroll past the first message).
    fn max_scroll(&self) -> usize {
        self.messages.len().saturating_sub(LOG_LINES)
    }

    /// Returns `(text, timestamp)` pairs for the `LOG_LINES` lines visible at
    /// the current scroll offset, ordered oldest-first (index 0 = top of panel,
    /// `LOG_LINES − 1` = bottom / newest).  Empty slots are `("", 0.0)`.
    pub fn visible_entries(&self) -> [(&str, f32); LOG_LINES] {
        let len    = self.messages.len();
        let bottom = len.saturating_sub(self.scroll_offset);
        let top    = bottom.saturating_sub(LOG_LINES);
        let mut entries = [("", 0.0_f32); LOG_LINES];
        // Align the slice to the bottom of the array so newer lines are lower.
        let range_len = bottom - top;
        let start_idx = LOG_LINES - range_len;
        for (i, entry) in self.messages.range(top..bottom).enumerate() {
            entries[start_idx + i] = (entry.text.as_str(), entry.timestamp);
        }
        entries
    }
}

// ── Event: GameMessage ────────────────────────────────────────────────────────

/// Fire this from any system to append a line to the game log.
///
/// ```rust
/// // Normal log line:
/// log_events.send(GameMessage::new("You found a key."));
///
/// // Log line + large centred alert for five seconds:
/// log_events.send(GameMessage::alert("The dungeon shakes!"));
/// ```
#[derive(Event)]
pub struct GameMessage {
    pub text:  String,
    /// When `true` the message is also displayed as a large centred overlay.
    pub alert: bool,
}

impl GameMessage {
    /// A normal log line with no alert overlay.
    pub fn new(s: impl Into<String>) -> Self {
        Self { text: s.into(), alert: false }
    }

    /// A log line that also triggers the centred alert overlay.
    pub fn alert(s: impl Into<String>) -> Self {
        Self { text: s.into(), alert: true }
    }
}

// ── Resource: AlertState ──────────────────────────────────────────────────────

#[derive(Resource, Default)]
struct AlertState {
    text:       String,
    /// `Time::elapsed_secs()` when the most recent alert was triggered.
    start_time: f32,
    active:     bool,
}

// ── HUD components ────────────────────────────────────────────────────────────

/// One of the `LOG_LINES` text nodes in the log panel (0 = top / oldest).
#[derive(Component)]
struct LogLine(usize);

/// The outermost alert overlay node (full-width centering strip).
#[derive(Component)]
struct AlertContainer;

/// The inner padded panel that carries the alert background colour.
#[derive(Component)]
struct AlertPanel;

/// The text node inside the alert panel.
#[derive(Component)]
struct AlertText;

// ── Startup ───────────────────────────────────────────────────────────────────

fn spawn_log_panel(mut commands: Commands) {
    // ── Scrollable message log (bottom-left) ──────────────────────────────────
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
                    TextColor(Color::srgba(FG.0, FG.1, FG.2, 1.0)),
                ));
            }
        });

    // ── Centred alert overlay (hidden until triggered) ────────────────────────
    commands
        .spawn((
            AlertContainer,
            Node {
                position_type:   PositionType::Absolute,
                top:             Val::Percent(28.0),
                width:           Val::Percent(100.0),
                justify_content: JustifyContent::Center,
                ..Default::default()
            },
            Visibility::Hidden,
        ))
        .with_children(|strip| {
            strip
                .spawn((
                    AlertPanel,
                    Node {
                        padding: UiRect::axes(Val::Px(24.0), Val::Px(14.0)),
                        ..Default::default()
                    },
                    BackgroundColor(Color::srgba(BG.0, BG.1, BG.2, ALERT_BG_ALPHA)),
                    BorderRadius::all(Val::Px(8.0)),
                ))
                .with_children(|panel| {
                    panel.spawn((
                        AlertText,
                        Text::new(""),
                        TextFont  { font_size: 28.0, ..Default::default() },
                        TextColor(Color::srgba(FG.0, FG.1, FG.2, 1.0)),
                    ));
                });
        });
}

// ── Update systems ────────────────────────────────────────────────────────────

fn receive_messages(
    mut events: EventReader<GameMessage>,
    mut log:    ResMut<GameLog>,
    mut alert:  ResMut<AlertState>,
    time:       Res<Time>,
) {
    let now = time.elapsed_secs();
    for msg in events.read() {
        log.push(&msg.text, now);
        if msg.alert {
            alert.text       = msg.text.clone();
            alert.start_time = now;
            alert.active     = true;
        }
    }
}

fn scroll_log(mut wheel: EventReader<MouseWheel>, mut log: ResMut<GameLog>) {
    for ev in wheel.read() {
        if ev.y > 0.0 {
            let max = log.max_scroll();
            log.scroll_offset = (log.scroll_offset + 1).min(max);
        } else if ev.y < 0.0 {
            log.scroll_offset = log.scroll_offset.saturating_sub(1);
        }
    }
}

/// Rewrites text and opacity for every log line every frame.
/// Runs unconditionally because opacity changes continuously even when no new
/// messages arrive.
fn update_log_hud(
    log:  Res<GameLog>,
    time: Res<Time>,
    mut line_q: Query<(&LogLine, &mut Text, &mut TextColor)>,
) {
    let now     = time.elapsed_secs();
    let entries = log.visible_entries();
    for (LogLine(i), mut text, mut color) in &mut line_q {
        let (line_text, timestamp) = entries[*i];
        **text = line_text.to_string();
        let alpha = if line_text.is_empty() {
            0.0
        } else {
            let age = (now - timestamp).max(0.0);
            (1.0 - age * OPACITY_FADE_PER_SEC).max(OPACITY_MIN)
        };
        color.0 = Color::srgba(FG.0, FG.1, FG.2, alpha);
    }
}

/// Drives the alert overlay: shows it when active, fades it out over the final
/// `ALERT_FADE_SECS` of its lifespan, then hides it.
fn update_alert(
    mut alert:         ResMut<AlertState>,
    time:              Res<Time>,
    mut container_q:   Query<&mut Visibility,    With<AlertContainer>>,
    mut panel_q:       Query<&mut BackgroundColor, With<AlertPanel>>,
    mut text_q:        Query<(&mut Text, &mut TextColor), With<AlertText>>,
) {
    let Ok(mut vis)                 = container_q.get_single_mut() else { return };
    let Ok(mut panel_bg)            = panel_q.get_single_mut()     else { return };
    let Ok((mut text, mut t_color)) = text_q.get_single_mut()      else { return };

    if !alert.active {
        *vis = Visibility::Hidden;
        return;
    }

    let age   = time.elapsed_secs() - alert.start_time;
    let total = ALERT_SHOW_SECS + ALERT_FADE_SECS;

    if age >= total {
        alert.active = false;
        *vis = Visibility::Hidden;
        return;
    }

    *vis   = Visibility::Inherited;
    **text = alert.text.clone();

    let alpha = if age < ALERT_SHOW_SECS {
        1.0
    } else {
        1.0 - (age - ALERT_SHOW_SECS) / ALERT_FADE_SECS
    };

    t_color.0  = Color::srgba(FG.0, FG.1, FG.2, alpha);
    panel_bg.0 = Color::srgba(BG.0, BG.1, BG.2, ALERT_BG_ALPHA * alpha);
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct GameLogPlugin;

impl Plugin for GameLogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameLog>()
            .init_resource::<AlertState>()
            .add_event::<GameMessage>()
            .add_systems(Startup, spawn_log_panel)
            .add_systems(
                Update,
                (
                    receive_messages,
                    scroll_log,
                    update_log_hud.after(receive_messages).after(scroll_log),
                    update_alert.after(receive_messages),
                ),
            );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn log_with_ts(msgs: &[(&str, f32)]) -> GameLog {
        let mut log = GameLog::default();
        for &(m, t) in msgs { log.push(m, t); }
        log
    }

    fn log_with(msgs: &[&str]) -> GameLog {
        log_with_ts(&msgs.iter().map(|&m| (m, 0.0_f32)).collect::<Vec<_>>())
    }

    // ── visible_entries ───────────────────────────────────────────────────────

    #[test]
    fn empty_log_returns_blank_lines() {
        let log = GameLog::default();
        let entries = log.visible_entries();
        assert!(entries.iter().all(|(t, _)| t.is_empty()));
    }

    #[test]
    fn single_message_appears_at_bottom() {
        let log = log_with(&["hello"]);
        let e   = log.visible_entries();
        assert_eq!(e[LOG_LINES - 1].0, "hello");
        assert!(e[..LOG_LINES - 1].iter().all(|(t, _)| t.is_empty()));
    }

    #[test]
    fn five_messages_fill_all_lines() {
        let log = log_with(&["a", "b", "c", "d", "e"]);
        let e   = log.visible_entries();
        let texts: Vec<&str> = e.iter().map(|&(t, _)| t).collect();
        assert_eq!(texts, vec!["a", "b", "c", "d", "e"]);
    }

    #[test]
    fn sixth_message_pushes_first_off_top() {
        let log = log_with(&["a", "b", "c", "d", "e", "f"]);
        let e   = log.visible_entries();
        assert_eq!(e[0].0, "b");
        assert_eq!(e[LOG_LINES - 1].0, "f");
    }

    #[test]
    fn new_message_resets_scroll_to_bottom() {
        let mut log = log_with(&["a", "b", "c", "d", "e", "f"]);
        log.scroll_offset = 1;
        log.push("g", 0.0);
        assert_eq!(log.scroll_offset, 0);
        assert_eq!(log.visible_entries()[LOG_LINES - 1].0, "g");
    }

    #[test]
    fn scroll_up_reveals_older_messages() {
        let mut log = log_with(&["a", "b", "c", "d", "e", "f"]);
        let max = log.max_scroll();
        log.scroll_offset = (log.scroll_offset + 1).min(max);
        let e = log.visible_entries();
        assert_eq!(e[0].0, "a");
        assert_eq!(e[LOG_LINES - 1].0, "e");
    }

    #[test]
    fn scroll_cannot_exceed_max() {
        let log = log_with(&["x"]);
        assert_eq!(log.max_scroll(), 0);
    }

    #[test]
    fn log_capped_at_max_size() {
        let mut log = GameLog::default();
        for i in 0..MAX_LOG_SIZE + 5 {
            log.push(format!("msg {i}"), 0.0);
        }
        assert_eq!(log.messages.len(), MAX_LOG_SIZE);
    }

    // ── Timestamps ────────────────────────────────────────────────────────────

    #[test]
    fn timestamps_preserved_in_visible_entries() {
        let log = log_with_ts(&[("a", 1.0), ("b", 2.0), ("c", 3.0)]);
        let e   = log.visible_entries();
        // Three messages → padded to LOG_LINES (5): first 2 slots are empty.
        assert_eq!(e[LOG_LINES - 3].1, 1.0);
        assert_eq!(e[LOG_LINES - 2].1, 2.0);
        assert_eq!(e[LOG_LINES - 1].1, 3.0);
    }

    // ── Opacity formula ───────────────────────────────────────────────────────

    #[test]
    fn opacity_newest_is_full() {
        let age   = 0.0_f32;
        let alpha = (1.0 - age * OPACITY_FADE_PER_SEC).max(OPACITY_MIN);
        assert!((alpha - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn opacity_floors_at_min() {
        // After 5 seconds at 10%/s the value would be 0.5, matching OPACITY_MIN.
        let age   = 5.0_f32;
        let alpha = (1.0 - age * OPACITY_FADE_PER_SEC).max(OPACITY_MIN);
        assert!((alpha - OPACITY_MIN).abs() < f32::EPSILON);
    }

    #[test]
    fn opacity_does_not_go_below_min_for_very_old_messages() {
        let age   = 100.0_f32;
        let alpha = (1.0 - age * OPACITY_FADE_PER_SEC).max(OPACITY_MIN);
        assert!(alpha >= OPACITY_MIN);
    }
}
