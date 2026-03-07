use std::collections::HashMap;

use bevy::{prelude::*, window::PrimaryWindow};
use bevy_light_2d::prelude::*;

use crate::{
    components::{MainCamera, MapPosition, Player, YSort},
    map::Map,
    ISO_STEP_X, ISO_STEP_Y, TILE_SCALE,
};

const RUN_FRAME_COUNT: usize = 10;
/// Seconds per run animation frame (≈10 fps).
const RUN_FRAME_SECS: f32 = 0.1;
/// Seconds between each auto-travel step (mouse double-click).
const AUTO_STEP_SECS: f32 = 0.15;
/// Two clicks within this window count as a double-click.
const DOUBLE_CLICK_SECS: f32 = 0.3;

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

    let facing = match (nx - pos.x, ny - pos.y) {
        (1, _)  => FacingDir::East,
        (-1, _) => FacingDir::West,
        (_, 1)  => FacingDir::South,
        _       => FacingDir::North,
    };

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
        app.init_resource::<ClickState>()
            .add_systems(Startup, spawn_player)
            .add_systems(
                Update,
                (
                    player_movement,
                    handle_mouse_click,
                    auto_step.after(player_movement),
                    animate_player,
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
    fn bfs_unreachable_returns_none() {
        let map = generate_map();
        // (0,0) is a wall tile and is surrounded by walls — completely isolated.
        let (cx, cy) = map.rooms[0].center();
        // Try to reach a position that is guaranteed to be a wall with no floor
        // neighbours; (0,0) is always wall in our generator.
        assert!(bfs_path(&map, (cx, cy), (0, 0)).is_none());
    }
}
