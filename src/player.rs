use std::collections::HashMap;

use bevy::{prelude::*, sprite::Anchor, window::PrimaryWindow};
use bevy_light_2d::prelude::*;

use crate::{
    components::{MainCamera, MapPosition, Player, YSort},
    map::Map,
    ISO_STEP_X, ISO_STEP_Y, TILE_SCALE,
};

// ── Beam wall-occlusion raycast ───────────────────────────────────────────────

/// Step the ray `origin + dir * t` in 8-unit increments until it enters a wall
/// tile or `max_dist` is reached.  Returns the last clear distance (≤ max_dist).
///
/// The step (8 world units) is well below the minimum tile crossing distance
/// (~71 units for the isometric grid), so no wall tile can be skipped.
fn wall_cast(map: &Map, origin: Vec2, dir: Vec2, max_dist: f32) -> f32 {
    const STEP: f32 = 8.0;
    let mut dist = STEP;
    while dist <= max_dist {
        let p = origin + dir * dist;
        let diff = p.x / ISO_STEP_X;
        let sum  = -p.y / ISO_STEP_Y;
        let tx = ((diff + sum) / 2.0).round() as i32;
        let ty = ((sum  - diff) / 2.0).round() as i32;
        if !map.is_walkable(tx, ty) {
            return dist - STEP;
        }
        dist += STEP;
    }
    max_dist
}

const RUN_FRAME_COUNT: usize = 10;
/// Seconds per run animation frame (≈10 fps).
const RUN_FRAME_SECS: f32 = 0.1;

// ── Torch-flicker tunables ────────────────────────────────────────────────────

const TORCH_RADIUS: f32 = 350.0;
/// Peak excursion of the radius (the "edge" flicker).
const TORCH_RADIUS_VAR: f32 = 60.0;
const TORCH_INTENSITY: f32 = 3.5;
/// Peak excursion of the intensity (the "core" flicker, kept subtle).
const TORCH_INTENSITY_VAR: f32 = 0.25;
/// Seconds between each auto-travel step (mouse double-click).
const AUTO_STEP_SECS: f32 = 0.15;
/// Two clicks within this window count as a double-click.
const DOUBLE_CLICK_SECS: f32 = 0.3;

// ── Lantern tunables ──────────────────────────────────────────────────────────

/// Sprite brightness multiplier for `LightType::Dark` (0.4 = 40 %).
const DARK_SPRITE_INTENSITY: f32 = 0.5;

const LANTERN_RADIUS: f32 = 60.0;
const LANTERN_INTENSITY: f32 = 2.5;
/// Beam starts at 20 % less intensity than the lantern base.
const BEAM_BASE_FACTOR: f32 = 0.80;
/// Each additional segment reduces beam intensity by a further 10 %.
const BEAM_DECAY: f32 = 0.70;
/// Number of PointLight2d entities used to approximate the directional beam.
const BEAM_SEGMENTS: usize = 8;
/// World-space distance between consecutive beam-light centers.
const BEAM_SEGMENT_SPACING: f32 = 60.0;
/// Radius of each beam-segment PointLight2d — exceeds half the spacing so
/// adjacent segments overlap, producing a continuous cone rather than spheres.
const BEAM_LIGHT_RADIUS: f32 = 80.0;
/// Maximum range of the directional beam (world units).
const BEAM_MAX_DIST: f32 = BEAM_SEGMENT_SPACING * BEAM_SEGMENTS as f32;

// ── Facing direction ──────────────────────────────────────────────────────────

/// Last movement direction, used to pick the correct directional sprite set.
///
/// Asset mapping (first integer in filename = direction index):
///   0 = North (dy=-1, dx=0)
///   1 = NorthEast (dy=-1, dx=+1)
///   2 = East (dx=+1, dy=0)
///   3 = SouthEast (dy=+1, dx=+1)
///   4 = South (dy=+1, dx=0)
///   5 = SouthWest (dy=+1, dx=-1)
///   6 = West (dx=-1, dy=0)
///   7 = NorthWest (dy=-1, dx=-1)
#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
enum FacingDir {
    North,
    NorthEast,
    East,
    SouthEast,
    #[default]
    South,
    SouthWest,
    West,
    NorthWest,
}

// ── Animation components ──────────────────────────────────────────────────────

/// Per-direction sprite sets for all 8 facing directions.
#[derive(Component)]
struct PlayerSprites {
    idle: [Handle<Image>; 8],
    run: [[Handle<Image>; RUN_FRAME_COUNT]; 8],
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
    /// Remaining steps for mouse-driven auto-travel.
    /// Stored in *reverse* order so `pop()` yields the next step.
    path: Vec<(i32, i32)>,
    /// Fires every AUTO_STEP_SECS to advance one tile along the path.
    step_timer: Timer,
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
            path: Vec::new(),
            step_timer: Timer::from_seconds(AUTO_STEP_SECS, TimerMode::Repeating),
        }
    }

    /// Call when the player takes a step to (re)start the run animation.
    fn trigger(&mut self, facing: FacingDir) {
        self.facing = facing;
        self.running = true;
        self.run_cooldown.reset();
    }
}

// ── Torch-flicker component ───────────────────────────────────────────────────

/// Drives the per-frame torch-light flicker on the player entity.
/// `t` accumulates elapsed seconds and feeds layered sine oscillators.
#[derive(Component, Default)]
struct TorchFlicker {
    t: f32,
}

// ── Light type ────────────────────────────────────────────────────────────────

/// Selects the lighting behaviour attached to the player.
#[derive(Component, Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LightType {
    /// Flickering torch — large omnidirectional radius.
    #[default]
    Torch,
    /// Steady lantern — small omnidirectional radius plus a directional beam.
    Lantern,
    /// Near-darkness — no world light; the player sprite is tinted to 40 %.
    Dark,
}

impl LightType {
    /// Advance to the next mode in the cycle: Torch → Lantern → Dark → Torch.
    fn next(self) -> Self {
        match self {
            Self::Torch   => Self::Lantern,
            Self::Dark    => Self::Torch,
            Self::Lantern => Self::Dark,
        }
    }
}

// ── Lantern beam-light component ──────────────────────────────────────────────

/// Marks one of the free-standing entities used to fake the lantern's
/// directional beam.  Spawned as top-level entities (not player children) to
/// avoid complications with the player's parent scale.
#[derive(Component)]
struct LanternBeamLight {
    /// Index into `BEAM_SEGMENT_CENTERS` (0 = closest to player).
    segment: usize,
}

// ── Public resources ──────────────────────────────────────────────────────────

/// Set to `true` while the player's run animation is active; `false` when idle.
/// Consumed by other plugins (e.g. HUD) to react to movement state.
#[derive(Resource, Default)]
pub struct PlayerMoving(pub bool);

// ── Click-state resource ──────────────────────────────────────────────────────

#[derive(Resource, Default)]
struct ClickState {
    last_click_time: f32,
}

// ── BFS pathfinding ───────────────────────────────────────────────────────────

/// Returns a path from `start` (exclusive) to `goal` (inclusive) as a list of
/// grid positions stored in *reverse* order so that `pop()` yields the next
/// step. Returns `None` if no walkable path exists.
fn bfs_path(map: &Map, start: (i32, i32), goal: (i32, i32)) -> Option<Vec<(i32, i32)>> {
    if start == goal {
        return Some(Vec::new());
    }

    let mut came_from: HashMap<(i32, i32), (i32, i32)> = HashMap::new();
    let mut queue = std::collections::VecDeque::new();

    came_from.insert(start, start);
    queue.push_back(start);

    while let Some(current) = queue.pop_front() {
        if current == goal {
            // Reconstruct path in reverse so pop() gives the first step.
            let mut path = Vec::new();
            let mut c = current;
            while c != start {
                path.push(c);
                c = came_from[&c];
            }
            // path = [goal, …, first_step]; pop() → first_step ✓
            return Some(path);
        }

        for (dx, dy) in [(0_i32, 1_i32), (0, -1), (1, 0), (-1, 0)] {
            let next = (current.0 + dx, current.1 + dy);
            if map.is_walkable(next.0, next.1) && !came_from.contains_key(&next) {
                came_from.insert(next, current);
                queue.push_back(next);
            }
        }
    }

    None
}

// ── Startup system: spawn the player ─────────────────────────────────────────

fn spawn_player(
    mut commands: Commands,
    map: Res<Map>,
    asset_server: Res<AssetServer>,
) {
    let sprites = PlayerSprites {
        idle: std::array::from_fn(|d| {
            asset_server.load(format!("Characters/Male/Male_{d}_Idle0.png"))
        }),
        run: std::array::from_fn(|d| {
            std::array::from_fn(|i| {
                asset_server.load(format!("Characters/Male/Male_{d}_Run{i}.png"))
            })
        }),
    };
    let initial_idle = sprites.idle[FacingDir::South as usize].clone();

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
            TorchFlicker::default(),
            LightType::default(),
            Sprite {
                image: initial_idle,
                // Ground-contact point (feet) at 20% from the bottom of the
                // sprite image, matching the wall tile anchor convention so the
                // character stands correctly on the isometric floor plane.
                anchor: Anchor::Custom(Vec2::new(0.0, -0.30)),
                ..Default::default()
            },
            Transform::from_xyz(world.x, world.y, 0.0).with_scale(Vec3::splat(TILE_SCALE)),
            PointLight2d {
                radius: TORCH_RADIUS,
                intensity: TORCH_INTENSITY,
                color: Color::srgb(1.0, 0.82, 0.45),
                cast_shadows: true,
                ..Default::default()
            },
        ))
        .with_children(|parent| {
            parent.spawn((
                Sprite {
                    color: Color::srgba(0.0, 0.0, 0.0, 0.45),
                    custom_size: Some(Vec2::new(ISO_STEP_X * 0.6, ISO_STEP_Y * 0.3)),
                    ..Default::default()
                },
                Transform::from_xyz(0.0, 0.0, -0.01),
            ));
        });

    // Spawn free-standing beam-light entities for the lantern.
    // Inactive (intensity = 0) while the player uses the Torch light type.
    // `update_lantern` repositions and activates them each frame as needed.
    for segment in 0..BEAM_SEGMENTS {
        commands.spawn((
            LanternBeamLight { segment },
            PointLight2d {
                radius: 0.0,
                intensity: 0.0,
                color: Color::srgb(1.0, 0.95, 0.7),
                cast_shadows: true,
                ..Default::default()
            },
            Transform::default(),
        ));
    }
}

// ── Direction helper ──────────────────────────────────────────────────────────

fn dir_to_facing(dx: i32, dy: i32) -> FacingDir {
    match (dx.signum(), dy.signum()) {
        (0, -1)  => FacingDir::North,
        (1, -1)  => FacingDir::NorthEast,
        (1, 0)   => FacingDir::East,
        (1, 1)   => FacingDir::SouthEast,
        (0, 1)   => FacingDir::South,
        (-1, 1)  => FacingDir::SouthWest,
        (-1, 0)  => FacingDir::West,
        (-1, -1) => FacingDir::NorthWest,
        _        => FacingDir::South,
    }
}

/// Convert a `FacingDir` to a normalised world-space 2-D direction, using the
/// same isometric projection as `MapPosition::to_world`:
///   world_x = (dx - dy) * ISO_STEP_X
///   world_y = -(dx + dy) * ISO_STEP_Y
fn facing_to_world_dir(facing: FacingDir) -> Vec2 {
    let (dx, dy): (f32, f32) = match facing {
        FacingDir::North     => ( 0.0, -1.0),
        FacingDir::NorthEast => ( 1.0, -1.0),
        FacingDir::East      => ( 1.0,  0.0),
        FacingDir::SouthEast => ( 1.0,  1.0),
        FacingDir::South     => ( 0.0,  1.0),
        FacingDir::SouthWest => (-1.0,  1.0),
        FacingDir::West      => (-1.0,  0.0),
        FacingDir::NorthWest => (-1.0, -1.0),
    };
    Vec2::new(
        (dx - dy) * ISO_STEP_X,
        -(dx + dy) * ISO_STEP_Y,
    )
    .normalize()
}

// ── Update system: keyboard movement ─────────────────────────────────────────

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

    // Any keyboard movement cancels auto-travel.
    anim.path.clear();

    let facing = dir_to_facing(dx, dy);

    let new_x = pos.x + dx;
    let new_y = pos.y + dy;

    if map.is_walkable(new_x, new_y) {
        pos.x = new_x;
        pos.y = new_y;
        let world = pos.to_world(0.0);
        transform.translation.x = world.x;
        transform.translation.y = world.y;
        anim.trigger(facing);
    } else {
        anim.facing = facing;
    }
}

// ── Update system: mouse double-click travel ──────────────────────────────────

fn handle_mouse_click(
    mouse: Res<ButtonInput<MouseButton>>,
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    camera_q: Query<(&Camera, &GlobalTransform), With<MainCamera>>,
    map: Res<Map>,
    mut click_state: ResMut<ClickState>,
    mut player_q: Query<(&MapPosition, &mut PlayerAnimation), With<Player>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Ok(window) = windows.get_single() else { return; };
    let Ok((camera, camera_tf)) = camera_q.get_single() else { return; };
    let Some(cursor_pos) = window.cursor_position() else { return; };
    let Ok(world_pos) = camera.viewport_to_world_2d(camera_tf, cursor_pos) else { return; };

    let now = time.elapsed_secs();
    let is_double = (now - click_state.last_click_time) < DOUBLE_CLICK_SECS;
    click_state.last_click_time = now;

    if !is_double {
        return;
    }

    // Invert the isometric projection:
    //   wx = (gx - gy) * ISO_STEP_X  →  gx - gy = wx / ISO_STEP_X
    //   wy = -(gx + gy) * ISO_STEP_Y →  gx + gy = -wy / ISO_STEP_Y
    let sum  = -world_pos.y / ISO_STEP_Y;
    let diff =  world_pos.x / ISO_STEP_X;
    let target_x = ((diff + sum) / 2.0).round() as i32;
    let target_y = ((sum  - diff) / 2.0).round() as i32;

    if !map.is_walkable(target_x, target_y) {
        return;
    }

    let Ok((pos, mut anim)) = player_q.get_single_mut() else { return; };

    if let Some(path) = bfs_path(&map, (pos.x, pos.y), (target_x, target_y)) {
        anim.path = path;
        anim.step_timer.reset();
    }
}

// ── Update system: advance one tile along the auto-travel path ────────────────

fn auto_step(
    time: Res<Time>,
    map: Res<Map>,
    mut query: Query<(&mut MapPosition, &mut Transform, &mut PlayerAnimation), With<Player>>,
) {
    let Ok((mut pos, mut transform, mut anim)) = query.get_single_mut() else { return; };

    if anim.path.is_empty() {
        return;
    }

    anim.step_timer.tick(time.delta());
    if !anim.step_timer.just_finished() {
        return;
    }

    let Some((nx, ny)) = anim.path.pop() else { return; };

    // Re-validate in case the map changed (e.g. future dynamic obstacles).
    if !map.is_walkable(nx, ny) {
        anim.path.clear();
        return;
    }

    let facing = dir_to_facing(nx - pos.x, ny - pos.y);

    pos.x = nx;
    pos.y = ny;
    let world = pos.to_world(0.0);
    transform.translation.x = world.x;
    transform.translation.y = world.y;
    anim.trigger(facing);
}

// ── Update system: drive the sprite animation ─────────────────────────────────

fn animate_player(
    time: Res<Time>,
    mut moving: ResMut<PlayerMoving>,
    mut query: Query<(&mut Sprite, &mut PlayerAnimation, &PlayerSprites), With<Player>>,
) {
    let Ok((mut sprite, mut anim, sprites)) = query.get_single_mut() else {
        moving.0 = false;
        return;
    };

    if anim.running {
        anim.run_cooldown.tick(time.delta());
        if anim.run_cooldown.finished() {
            anim.running = false;
            anim.frame = 0;
        }
    }

    moving.0 = anim.running;

    let dir = anim.facing as usize;

    if anim.running {
        anim.frame_timer.tick(time.delta());
        if anim.frame_timer.just_finished() {
            anim.frame = (anim.frame + 1) % RUN_FRAME_COUNT;
        }
        sprite.image = sprites.run[dir][anim.frame].clone();
    } else {
        sprite.image = sprites.idle[dir].clone();
    }
}

// ── Update system: torch-light flicker ───────────────────────────────────────

/// Modulates the player's `PointLight2d` each frame to simulate a torch.
///
/// Strategy: layer four sine oscillators at incommensurate frequencies so the
/// combination never repeats and sounds organic.  Slower oscillators dominate
/// the *intensity* signal (the bright core stays relatively steady) while
/// faster oscillators dominate the *radius* signal (the lit edge dances a lot).
fn flicker_torch(
    time: Res<Time>,
    mut query: Query<(&mut PointLight2d, &mut TorchFlicker, &LightType), With<Player>>,
) {
    let Ok((mut light, mut flicker, light_type)) = query.get_single_mut() else {
        return;
    };
    if *light_type != LightType::Torch {
        return;
    }

    flicker.t += time.delta_secs();
    let t = flicker.t;

    // Four oscillators at frequencies chosen to be mutually irrational so the
    // waveform never becomes periodic at a human-perceptible timescale.
    let s1 = (t * 1.7_f32).sin();   // slow sway
    let s2 = (t * 4.3_f32).sin();   // medium flicker
    let s3 = (t * 11.0_f32).sin();  // fast edge shimmer
    let s4 = (t * 23.7_f32).sin();  // micro-flutter

    // Core (intensity): weighted toward slow oscillators → subtle breathing.
    let core = s1 * 0.50 + s2 * 0.35 + s3 * 0.15;
    // Edge (radius): weighted toward fast oscillators → lively boundary dance.
    let edge = s1 * 0.15 + s2 * 0.25 + s3 * 0.35 + s4 * 0.25;

    light.intensity = (TORCH_INTENSITY + core * TORCH_INTENSITY_VAR).max(0.5);
    light.radius    = (TORCH_RADIUS    + edge * TORCH_RADIUS_VAR   ).max(150.0);
}

// ── Update system: L key cycles light type ────────────────────────────────────

fn toggle_light_type(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut query: Query<&mut LightType, With<Player>>,
) {
    if !keyboard.just_pressed(KeyCode::KeyL) {
        return;
    }
    let Ok(mut light_type) = query.get_single_mut() else { return; };
    *light_type = light_type.next();
}

// ── Update system: apply active light type each frame ─────────────────────────

/// Applies the player's current `LightType` every frame:
///
/// - **Torch** — handled entirely by `flicker_torch`; sprite stays white.
/// - **Lantern** — base sphere locked to radius/intensity constants; beam
///   lights repositioned along the facing direction; sprite stays white.
/// - **Dark** — world `PointLight2d` silenced; sprite tinted to 40 % so the
///   player is barely visible without illuminating anything else.
fn apply_light_type(
    map: Res<Map>,
    mut player_q: Query<
        (&Transform, &PlayerAnimation, &LightType, &mut PointLight2d, &mut Sprite),
        With<Player>,
    >,
    mut beam_q: Query<(&mut Transform, &mut PointLight2d, &LanternBeamLight), Without<Player>>,
) {
    let Ok((player_tf, anim, light_type, mut player_light, mut sprite)) =
        player_q.get_single_mut()
    else {
        return;
    };

    match *light_type {
        LightType::Torch => {
            // flicker_torch drives the PointLight2d; just keep the sprite white.
            //sprite.color = Color::WHITE;
            sprite.color = Color::srgb(1.0, 0.95, 0.2);
        }
        LightType::Lantern => {
            player_light.radius = LANTERN_RADIUS;
            player_light.intensity = LANTERN_INTENSITY;
            sprite.color = Color::WHITE;
        }
        LightType::Dark => {
            player_light.intensity = 0.0;
            player_light.radius = 0.0;
            // sprite.color = Color::srgb(0.3, 0.7, 0.9);
            sprite.color = Color::srgb(DARK_SPRITE_INTENSITY, DARK_SPRITE_INTENSITY, DARK_SPRITE_INTENSITY);
        }
    }

    let player_pos = player_tf.translation.truncate();
    let beam_dir = (*light_type == LightType::Lantern)
        .then(|| facing_to_world_dir(anim.facing));

    // How far the beam travels before hitting a wall (0 when not in lantern mode).
    let clear_dist = match beam_dir {
        Some(dir) => wall_cast(&map, player_pos, dir, BEAM_MAX_DIST),
        None => 0.0,
    };

    for (mut beam_tf, mut beam_light, beam) in &mut beam_q {
        match beam_dir {
            Some(dir) => {
                let dist = (beam.segment as f32 + 0.5) * BEAM_SEGMENT_SPACING;
                if dist <= clear_dist {
                    beam_tf.translation = (player_pos + dir * dist).extend(0.0);
                    beam_light.intensity = LANTERN_INTENSITY
                        * BEAM_BASE_FACTOR
                        * BEAM_DECAY.powi(beam.segment as i32);
                    beam_light.radius = BEAM_LIGHT_RADIUS;
                } else {
                    beam_light.intensity = 0.0;
                }
            }
            None => {
                beam_light.intensity = 0.0;
            }
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerMoving>()
            .init_resource::<ClickState>()
            .add_systems(Startup, spawn_player)
            .add_systems(
                Update,
                (
                    player_movement,
                    handle_mouse_click,
                    auto_step.after(player_movement),
                    animate_player,
                    toggle_light_type,
                    flicker_torch.after(toggle_light_type),
                    apply_light_type.after(toggle_light_type),
                ),
            );
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::generate_map;

    #[test]
    fn bfs_reaches_room_center() {
        let map = generate_map();
        let (ax, ay) = map.rooms[0].center();
        let (bx, by) = map.rooms.last().unwrap().center();
        // If there is only one room the path is empty (already there).
        if (ax, ay) == (bx, by) {
            assert!(bfs_path(&map, (ax, ay), (bx, by)).unwrap().is_empty());
            return;
        }
        let path = bfs_path(&map, (ax, ay), (bx, by));
        assert!(path.is_some(), "rooms should be connected via corridors");
        let path = path.unwrap();
        // Last element popped is the first step — must be adjacent to start.
        let first_step = *path.last().unwrap();
        let dist = (first_step.0 - ax).abs() + (first_step.1 - ay).abs();
        assert_eq!(dist, 1, "first step must be one tile away");
    }

    #[test]
    fn bfs_same_start_and_goal() {
        let map = generate_map();
        let (cx, cy) = map.rooms[0].center();
        let path = bfs_path(&map, (cx, cy), (cx, cy)).unwrap();
        assert!(path.is_empty());
    }

    #[test]
    fn dir_to_facing_cardinals() {
        assert_eq!(dir_to_facing(0, -1) as usize, FacingDir::North as usize);
        assert_eq!(dir_to_facing(1, 0)  as usize, FacingDir::East  as usize);
        assert_eq!(dir_to_facing(0, 1)  as usize, FacingDir::South as usize);
        assert_eq!(dir_to_facing(-1, 0) as usize, FacingDir::West  as usize);
    }

    #[test]
    fn dir_to_facing_diagonals() {
        assert_eq!(dir_to_facing(1, -1)  as usize, FacingDir::NorthEast as usize);
        assert_eq!(dir_to_facing(1, 1)   as usize, FacingDir::SouthEast as usize);
        assert_eq!(dir_to_facing(-1, 1)  as usize, FacingDir::SouthWest as usize);
        assert_eq!(dir_to_facing(-1, -1) as usize, FacingDir::NorthWest as usize);
    }

    #[test]
    fn facing_to_world_dir_is_normalised() {
        let all = [
            FacingDir::North,
            FacingDir::NorthEast,
            FacingDir::East,
            FacingDir::SouthEast,
            FacingDir::South,
            FacingDir::SouthWest,
            FacingDir::West,
            FacingDir::NorthWest,
        ];
        for facing in all {
            let d = facing_to_world_dir(facing);
            assert!(
                (d.length() - 1.0).abs() < 1e-5,
                "{facing:?} direction length {} is not 1",
                d.length()
            );
        }
    }

    #[test]
    fn facing_to_world_dir_opposites_cancel() {
        let pairs = [
            (FacingDir::North, FacingDir::South),
            (FacingDir::East, FacingDir::West),
            (FacingDir::NorthEast, FacingDir::SouthWest),
            (FacingDir::NorthWest, FacingDir::SouthEast),
        ];
        for (a, b) in pairs {
            let sum = facing_to_world_dir(a) + facing_to_world_dir(b);
            assert!(
                sum.length() < 1e-5,
                "{a:?} and {b:?} should point in opposite directions (sum length = {})",
                sum.length()
            );
        }
    }

    #[test]
    fn light_type_cycle_is_complete() {
        // Every mode must eventually return to Torch after enough presses.
        let start = LightType::Torch;
        let next1 = start.next();
        let next2 = next1.next();
        let next3 = next2.next();
        assert_eq!(next1, LightType::Lantern);
        assert_eq!(next2, LightType::Dark);
        assert_eq!(next3, LightType::Torch, "cycle must return to Torch");
    }

    #[test]
    fn dark_sprite_intensity_in_range() {
        assert!(
            DARK_SPRITE_INTENSITY > 0.0 && DARK_SPRITE_INTENSITY <= 1.0,
            "DARK_SPRITE_INTENSITY must be in (0, 1]"
        );
    }

    #[test]
    fn beam_segment_centers_within_max_dist() {
        for seg in 0..BEAM_SEGMENTS {
            let center = (seg as f32 + 0.5) * BEAM_SEGMENT_SPACING;
            assert!(
                center <= BEAM_MAX_DIST,
                "beam segment {seg} center {center} exceeds BEAM_MAX_DIST {BEAM_MAX_DIST}"
            );
        }
    }

    #[test]
    fn beam_segments_overlap() {
        // Each segment's radius must exceed half the spacing so adjacent lights
        // overlap, preventing dark bands between them.
        assert!(
            BEAM_LIGHT_RADIUS > BEAM_SEGMENT_SPACING / 2.0,
            "BEAM_LIGHT_RADIUS {BEAM_LIGHT_RADIUS} must exceed half BEAM_SEGMENT_SPACING {}",
            BEAM_SEGMENT_SPACING / 2.0,
        );
    }

    #[test]
    fn beam_intensity_declines_each_segment() {
        let mut prev = f32::MAX;
        for seg in 0..BEAM_SEGMENTS {
            let intensity = LANTERN_INTENSITY * BEAM_BASE_FACTOR * BEAM_DECAY.powi(seg as i32);
            assert!(
                intensity < prev,
                "segment {seg} intensity {intensity} should be less than previous {prev}"
            );
            prev = intensity;
        }
    }

    #[test]
    fn bfs_unreachable_returns_none() {
        let map = generate_map();
        // (0,0) is a wall tile and is surrounded by walls — completely isolated.
        let (cx, cy) = map.rooms[0].center();
        // Try to reach a position that is guaranteed to be a wall with no floor
        // neighbours; (0,0) is always wall in our generator.
        assert!(bfs_path(&map, (cx, cy), (0, 0)).is_none());
    }
}
