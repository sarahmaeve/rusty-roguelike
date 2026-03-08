use std::f32::consts::FRAC_PI_4;

use bevy::prelude::*;

use crate::player::PlayerMoving;

// ── Tunables ──────────────────────────────────────────────────────────────────

const SIZE: f32 = 150.0;
const HALF: f32 = SIZE / 2.0;
/// Half-length of each arm from the compass centre.
const HALF_ARM: f32 = 46.0;
const ARM_W: f32 = 2.5;

const IDLE_ALPHA: f32 = 0.85;
const MOVE_ALPHA: f32 = 0.08;
const FADE_SPEED: f32 = 5.0;

// Warm gold — same palette as the wall occlusion tint.
const FG: (f32, f32, f32) = (0.95, 0.85, 0.55);
const BG: (f32, f32, f32) = (0.05, 0.05, 0.12);

// ── Resources / components ────────────────────────────────────────────────────

#[derive(Resource)]
struct CompassAlpha(f32);

/// Marks the circular background node.
#[derive(Component)]
struct CompassBg;

/// Marks arms and the centre dot — tinted warm gold.
#[derive(Component)]
struct CompassFg;

/// Marks text labels — excluded from background-color updates.
#[derive(Component)]
struct CompassLabel;

// ── Type aliases ──────────────────────────────────────────────────────────────

type ArmBgQuery<'w, 's> = Query<
    'w,
    's,
    &'static mut BackgroundColor,
    (With<CompassFg>, Without<CompassBg>, Without<CompassLabel>),
>;

// ── Helpers ───────────────────────────────────────────────────────────────────

fn fg(a: f32) -> Color {
    Color::srgba(FG.0, FG.1, FG.2, a)
}
fn bg(a: f32) -> Color {
    Color::srgba(BG.0, BG.1, BG.2, a)
}

// ── Startup ───────────────────────────────────────────────────────────────────

fn spawn_compass(mut commands: Commands) {
    commands
        .spawn((
            CompassBg,
            Node {
                position_type: PositionType::Absolute,
                right: Val::Px(20.0),
                top: Val::Px(20.0),
                width: Val::Px(SIZE),
                height: Val::Px(SIZE),
                ..Default::default()
            },
            BackgroundColor(bg(0.0)),
            BorderRadius::all(Val::Px(HALF)),
        ))
        .with_children(|p| {
            // Two diagonal arms crossing at the compass centre.
            //
            // In this isometric layout the grid axes appear at 45°:
            //   "/"  arm (FRAC_PI_4):  N tip upper-right ↔ S tip lower-left
            //   "\"  arm (-FRAC_PI_4): W tip upper-left  ↔ E tip lower-right
            spawn_arm(p, FRAC_PI_4);
            spawn_arm(p, -FRAC_PI_4);

            // Centre dot.
            const DOT: f32 = 8.0;
            p.spawn((
                CompassFg,
                Node {
                    position_type: PositionType::Absolute,
                    left: Val::Px(HALF - DOT / 2.0),
                    top: Val::Px(HALF - DOT / 2.0),
                    width: Val::Px(DOT),
                    height: Val::Px(DOT),
                    ..Default::default()
                },
                BackgroundColor(fg(0.0)),
                BorderRadius::all(Val::Px(DOT / 2.0)),
            ));

            // One key label per tip: shows which key moves the player toward that tip.
            //
            // Isometric screen directions:
            //   upper-right → S key    lower-right → D key
            //   lower-left  → W key    upper-left  → A key
            spawn_label(p, "S", Corner::TopRight);
            spawn_label(p, "D", Corner::BottomRight);
            spawn_label(p, "W", Corner::BottomLeft);
            spawn_label(p, "A", Corner::TopLeft);
        });
}

/// One diagonal arm: a thin horizontal bar centred on the compass, rotated.
///
/// In Bevy 0.15, UI node transforms rotate around the node's own centre, so
/// positioning the bar centred at (HALF, HALF) and applying a rotation gives
/// a diagonal line through the compass centre with no extra pivot arithmetic.
fn spawn_arm(parent: &mut ChildBuilder, angle: f32) {
    parent.spawn((
        CompassFg,
        Node {
            position_type: PositionType::Absolute,
            left: Val::Px(HALF - HALF_ARM),
            top: Val::Px(HALF - ARM_W / 2.0),
            width: Val::Px(HALF_ARM * 2.0),
            height: Val::Px(ARM_W),
            ..Default::default()
        },
        BackgroundColor(fg(0.0)),
        Transform::from_rotation(Quat::from_rotation_z(angle)),
    ));
}

enum Corner {
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

fn spawn_label(parent: &mut ChildBuilder, text: &str, corner: Corner) {
    const PAD: f32 = 8.0;
    let (left, right, top, bottom) = match corner {
        Corner::TopLeft     => (Val::Px(PAD), Val::Auto,    Val::Px(PAD), Val::Auto),
        Corner::TopRight    => (Val::Auto,    Val::Px(PAD), Val::Px(PAD), Val::Auto),
        Corner::BottomLeft  => (Val::Px(PAD), Val::Auto,    Val::Auto,    Val::Px(PAD)),
        Corner::BottomRight => (Val::Auto,    Val::Px(PAD), Val::Auto,    Val::Px(PAD)),
    };
    parent.spawn((
        CompassFg,
        CompassLabel,
        Node {
            position_type: PositionType::Absolute,
            left,
            right,
            top,
            bottom,
            ..Default::default()
        },
        Text::new(text),
        TextFont {
            font_size: 15.0,
            ..Default::default()
        },
        TextColor(Color::srgba(1.0, 0.15, 0.15, 0.0)),
    ));
}

// ── Update: fade compass in/out based on movement state ───────────────────────

fn update_compass_alpha(
    moving: Res<PlayerMoving>,
    time: Res<Time>,
    mut state: ResMut<CompassAlpha>,
    mut bg_q: Query<&mut BackgroundColor, With<CompassBg>>,
    mut fg_bg_q: ArmBgQuery,
    mut fg_text_q: Query<&mut TextColor, With<CompassFg>>,
) {
    let target = if moving.0 { MOVE_ALPHA } else { IDLE_ALPHA };
    let a = &mut state.0;
    *a += (target - *a) * (time.delta_secs() * FADE_SPEED).min(1.0);
    let alpha = *a;

    for mut c in &mut bg_q {
        c.0 = bg(alpha * 0.5);
    }
    for mut c in &mut fg_bg_q {
        c.0 = fg(alpha);
    }
    for mut c in &mut fg_text_q {
        c.0 = Color::srgba(1.0, 0.15, 0.15, alpha);
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct HudPlugin;

impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CompassAlpha(0.0))
            .add_systems(Startup, spawn_compass)
            .add_systems(Update, update_compass_alpha);
    }
}
