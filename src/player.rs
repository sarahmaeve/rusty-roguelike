use bevy::prelude::*;
use bevy_light_2d::prelude::*;

use crate::{
    components::{MapPosition, Player, YSort},
    map::Map,
    ISO_STEP_X, ISO_STEP_Y, TILE_SCALE,
};

const RUN_FRAME_COUNT: usize = 10;
/// Seconds per run animation frame (≈10 fps).
const RUN_FRAME_SECS: f32 = 0.1;

// ── Facing direction ──────────────────────────────────────────────────────────

/// Last movement direction, used to pick the correct directional sprite set.
///
/// Screen-space directions for this isometric projection:
///   East  (dx=+1) → screen SE  — Male_1 frames, flip_x=false
///   West  (dx=-1) → screen NW  — Male_1 frames, flip_x=true  (mirrored)
///   South (dy=+1) → screen SW  — Male_3 frames (toward viewer)
///   North (dy=-1) → screen NE  — Male_0 frames (away from viewer)
#[derive(Clone, Copy, PartialEq, Eq, Default)]
enum FacingDir {
    East,
    West,
    #[default]
    South,
    North,
}

// ── Animation components ──────────────────────────────────────────────────────

/// Per-direction sprite sets. West mirrors East via `flip_x`.
#[derive(Component)]
struct PlayerSprites {
    north_idle: Handle<Image>,
    north_run: [Handle<Image>; RUN_FRAME_COUNT],
    east_idle: Handle<Image>,
    east_run: [Handle<Image>; RUN_FRAME_COUNT],
    south_idle: Handle<Image>,
    south_run: [Handle<Image>; RUN_FRAME_COUNT],
}

#[derive(Component)]
struct PlayerAnimation {
    facing: FacingDir,
    running: bool,
    frame: usize,
    /// Advances through run frames while running.
    frame_timer: Timer,
    /// Reset on each step; idle resumes when this expires.
    run_cooldown: Timer,
}

impl PlayerAnimation {
    fn new() -> Self {
        Self {
            facing: FacingDir::default(),
            running: false,
            frame: 0,
            frame_timer: Timer::from_seconds(RUN_FRAME_SECS, TimerMode::Repeating),
            run_cooldown: Timer::from_seconds(
                RUN_FRAME_SECS * RUN_FRAME_COUNT as f32,
                TimerMode::Once,
            ),
        }
    }

    /// Call when the player takes a step to (re)start the run animation.
    fn trigger(&mut self, facing: FacingDir) {
        self.facing = facing;
        self.running = true;
        self.run_cooldown.reset();
    }
}

// ── Startup system: spawn the player ─────────────────────────────────────────

fn spawn_player(
    mut commands: Commands,
    map: Res<Map>,
    asset_server: Res<AssetServer>,
) {
    let load_run = |variant: u8| -> [Handle<Image>; RUN_FRAME_COUNT] {
        std::array::from_fn(|i| {
            asset_server.load(format!("Characters/Male/Male_{variant}_Run{i}.png"))
        })
    };

    let sprites = PlayerSprites {
        north_idle: asset_server.load("Characters/Male/Male_0_Idle0.png"),
        north_run: load_run(0),
        east_idle: asset_server.load("Characters/Male/Male_1_Idle0.png"),
        east_run: load_run(1),
        south_idle: asset_server.load("Characters/Male/Male_3_Idle0.png"),
        south_run: load_run(3),
    };
    // Default facing is South; start with the south idle sprite.
    let initial_idle = sprites.south_idle.clone();

    let (cx, cy) = map.rooms[0].center();
    let pos = MapPosition::new(cx, cy);
    let world = pos.to_world(0.0);

    commands
        .spawn((
            Player,
            YSort,
            pos,
            sprites,
            PlayerAnimation::new(),
            Sprite {
                image: initial_idle,
                ..Default::default()
            },
            Transform::from_xyz(world.x, world.y, 0.0).with_scale(Vec3::splat(TILE_SCALE)),
            PointLight2d {
                radius: 350.0,
                intensity: 3.5,
                color: Color::srgb(1.0, 0.82, 0.45),
                cast_shadows: false,
                ..Default::default()
            },
        ))
        .with_children(|parent| {
            // Isometric drop shadow: a flattened ellipse below the player sprite.
            parent.spawn((
                Sprite {
                    color: Color::srgba(0.0, 0.0, 0.0, 0.45),
                    custom_size: Some(Vec2::new(ISO_STEP_X * 0.6, ISO_STEP_Y * 0.3)),
                    ..Default::default()
                },
                Transform::from_xyz(0.0, -ISO_STEP_Y * 0.5, -0.01),
            ));
        });
}

// ── Update system: tile-based movement ───────────────────────────────────────

fn player_movement(
    keyboard: Res<ButtonInput<KeyCode>>,
    map: Res<Map>,
    mut query: Query<(&mut MapPosition, &mut Transform, &mut PlayerAnimation), With<Player>>,
) {
    let Ok((mut pos, mut transform, mut anim)) = query.get_single_mut() else {
        return;
    };

    let mut dx = 0_i32;
    let mut dy = 0_i32;

    if keyboard.just_pressed(KeyCode::KeyW) || keyboard.just_pressed(KeyCode::ArrowUp) {
        dy = 1;
    } else if keyboard.just_pressed(KeyCode::KeyS) || keyboard.just_pressed(KeyCode::ArrowDown) {
        dy = -1;
    } else if keyboard.just_pressed(KeyCode::KeyA) || keyboard.just_pressed(KeyCode::ArrowLeft) {
        dx = -1;
    } else if keyboard.just_pressed(KeyCode::KeyD) || keyboard.just_pressed(KeyCode::ArrowRight) {
        dx = 1;
    }

    if dx == 0 && dy == 0 {
        return;
    }

    // Determine facing before attempting move so the sprite updates even on
    // a blocked step (gives the player feedback that input was received).
    let facing = match (dx, dy) {
        (1, _)  => FacingDir::East,
        (-1, _) => FacingDir::West,
        (_, 1)  => FacingDir::South,
        _       => FacingDir::North,
    };

    let new_x = pos.x + dx;
    let new_y = pos.y + dy;

    if map.is_walkable(new_x, new_y) {
        pos.x = new_x;
        pos.y = new_y;
        let world = pos.to_world(0.0);
        // Z is managed by y_sort each frame; only update X/Y here.
        transform.translation.x = world.x;
        transform.translation.y = world.y;
        anim.trigger(facing);
    } else {
        // Wall bump: update facing without triggering run animation.
        anim.facing = facing;
    }
}

// ── Update system: drive the sprite animation ─────────────────────────────────

fn animate_player(
    time: Res<Time>,
    mut query: Query<(&mut Sprite, &mut PlayerAnimation, &PlayerSprites), With<Player>>,
) {
    let Ok((mut sprite, mut anim, sprites)) = query.get_single_mut() else {
        return;
    };

    if anim.running {
        anim.run_cooldown.tick(time.delta());
        if anim.run_cooldown.finished() {
            anim.running = false;
            anim.frame = 0;
        }
    }

    // West mirrors East with a horizontal flip.
    sprite.flip_x = anim.facing == FacingDir::West;

    let (idle, run) = match anim.facing {
        FacingDir::North => (&sprites.north_idle, &sprites.north_run),
        FacingDir::East | FacingDir::West => (&sprites.east_idle, &sprites.east_run),
        FacingDir::South => (&sprites.south_idle, &sprites.south_run),
    };

    if anim.running {
        anim.frame_timer.tick(time.delta());
        if anim.frame_timer.just_finished() {
            anim.frame = (anim.frame + 1) % RUN_FRAME_COUNT;
        }
        sprite.image = run[anim.frame].clone();
    } else {
        sprite.image = idle.clone();
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player)
            .add_systems(Update, (player_movement, animate_player));
    }
}
