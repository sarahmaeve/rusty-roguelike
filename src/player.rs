use std::collections::HashMap;

use bevy::{ecs::system::SystemParam, prelude::*, sprite::Anchor, window::PrimaryWindow};
use bevy_light_2d::prelude::*;

use crate::{
    components::{CharacterKind, Door, MainCamera, MapPosition, MapTile, Player, StairsMidTile, StairsUpTile, YSort},
    map::{spawn_floor_doors, spawn_floor_tiles, DoorRegistry, Dungeon, Map, TileType},
    ISO_STEP_X, ISO_STEP_Y, TILE_SCALE,
};
// wall_cast     → Map::is_walkable  (tile-level; props and doors do not block beams)
// movement/BFS  → Map::is_passable + DoorRegistry  (tile + no prop + no closed door)

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

// ── Female character constants ────────────────────────────────────────────────

/// Number of facing directions in the Female asset pack (every 22.5°).
const FEMALE_DIR_COUNT: usize = 16;
/// Frames in each Female walk spritesheet (6 columns × 4 rows).
const FEMALE_WALK_FRAME_COUNT: usize = 24;
/// Frames in each Female idle spritesheet (4 columns × 4 rows).
const FEMALE_IDLE_FRAME_COUNT: usize = 16;
/// Cell size of each Female spritesheet frame, in pixels.
const FEMALE_CELL_PX: u32 = 256;
/// Degree values matching the Female asset filename suffixes.
const FEMALE_ANGLES: [u32; FEMALE_DIR_COUNT] = [
    0, 22, 45, 67, 90, 112, 135, 157, 180, 202, 225, 247, 270, 292, 315, 337,
];

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

const LANTERN_RADIUS: f32 = 120.0;
const LANTERN_INTENSITY: f32 = 2.25;
/// Beam starts at 20 % less intensity than the lantern base.
const BEAM_BASE_FACTOR: f32 = 0.75;
/// Each additional segment reduces beam intensity by a further 40 %.
const BEAM_DECAY: f32 = 0.60;
/// Number of PointLight2d entities used to approximate the directional beam.
const BEAM_SEGMENTS: usize = 6;
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

impl FacingDir {
    /// Returns the index into [`FEMALE_ANGLES`] whose angle is closest to this
    /// facing direction.
    ///
    /// The degree mapping assumes 0° = East in the Female asset pack, increasing
    /// clockwise.  If the in-game sprites point the wrong way, adjust the values
    /// in the `match` arm below.
    fn to_female_dir_index(self) -> usize {
        let deg: u32 = match self {
            FacingDir::East      =>   0,
            FacingDir::SouthEast =>  45,
            FacingDir::South     =>  90,
            FacingDir::SouthWest => 135,
            FacingDir::West      => 180,
            FacingDir::NorthWest => 225,
            FacingDir::North     => 270,
            FacingDir::NorthEast => 315,
        };
        FEMALE_ANGLES
            .iter()
            .enumerate()
            .min_by_key(|&(_, &a)| {
                let diff = (a as i32 - deg as i32).abs();
                diff.min(360 - diff) as u32
            })
            .unwrap()
            .0
    }
}

// ── Animation components ──────────────────────────────────────────────────────

/// Per-direction sprite sets for all 8 facing directions (Male character).
#[derive(Component)]
struct PlayerSprites {
    idle: [Handle<Image>; 8],
    run: [[Handle<Image>; RUN_FRAME_COUNT]; 8],
}

/// Spritesheet handles and atlas layouts for the Female character.
///
/// The Female asset pack provides one spritesheet per facing direction for each
/// animation state, with separate body and shadow layers.  The atlas layout is
/// shared across all directions since every sheet has the same grid dimensions.
#[derive(Component)]
struct FemaleSprites {
    /// Body layer idle sheets, one per direction (4×4 grid, 256×256 px cells).
    idle_body:   [Handle<Image>; FEMALE_DIR_COUNT],
    /// Shadow layer idle sheets, one per direction.
    idle_shadow: [Handle<Image>; FEMALE_DIR_COUNT],
    /// Body layer walk sheets, one per direction (6×4 grid, 256×256 px cells).
    walk_body:   [Handle<Image>; FEMALE_DIR_COUNT],
    /// Shadow layer walk sheets, one per direction.
    walk_shadow: [Handle<Image>; FEMALE_DIR_COUNT],
    /// Shared atlas layout for all idle sheets (4 columns × 4 rows).
    idle_layout: Handle<TextureAtlasLayout>,
    /// Shared atlas layout for all walk sheets (6 columns × 4 rows).
    walk_layout: Handle<TextureAtlasLayout>,
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

/// Marks the shadow sprite child of the player entity.
/// For the Female character this is an atlas sprite from the asset pack,
/// kept in sync with the body sprite each animation frame.
#[derive(Component)]
struct PlayerShadow;

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
/// step. Returns `None` if no passable path exists.
///
/// `closed_doors` is the set of grid positions currently blocked by a closed
/// door entity.  Closed door cells are passable at the tile level (their tile
/// is `Floor`) but must be treated as blocked for pathfinding purposes.
fn bfs_path(
    map: &Map,
    closed_doors: &std::collections::HashSet<(i32, i32)>,
    start: (i32, i32),
    goal: (i32, i32),
) -> Option<Vec<(i32, i32)>> {
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
            if map.is_passable(next.0, next.1)
                && !closed_doors.contains(&next)
                && !came_from.contains_key(&next)
            {
                came_from.insert(next, current);
                queue.push_back(next);
            }
        }
    }

    None
}

/// Builds the set of grid positions currently blocked by a closed door.
/// Called before pathfinding and single-step movement checks.
fn closed_door_positions(
    registry: &DoorRegistry,
    door_q: &Query<&Door>,
) -> std::collections::HashSet<(i32, i32)> {
    registry
        .0
        .iter()
        .filter(|(_, entity)| {
            door_q.get(**entity).map(|d| !d.open).unwrap_or(false)
        })
        .map(|(&pos, _)| pos)
        .collect()
}

// ── Startup system: spawn the player ─────────────────────────────────────────

fn spawn_player(
    mut commands: Commands,
    dungeon: Res<Dungeon>,
    asset_server: Res<AssetServer>,
    mut layouts: ResMut<Assets<TextureAtlasLayout>>,
) {
    let (cx, cy) = dungeon.current_map().rooms[0].center();
    let pos = MapPosition::new(cx, cy);
    let world = pos.to_world(0.0);

    // ── Female atlas layouts ──────────────────────────────────────────────────
    // Idle sheets: 4 columns × 4 rows, each cell 256×256 px → 16 frames.
    let idle_layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(FEMALE_CELL_PX),
        4,
        4,
        None,
        None,
    ));
    // Walk sheets: 6 columns × 4 rows, each cell 256×256 px → 24 frames.
    let walk_layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(FEMALE_CELL_PX),
        6,
        4,
        None,
        None,
    ));

    let idle_body: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/IdleUnarmed/Idle_Unarmed_Body_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let idle_shadow: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/IdleUnarmed/Idle_Unarmed_Shadow_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let walk_body: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/WalkForwardUnarmed/WalkForward_Unarmed_Body_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let walk_shadow: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/WalkForwardUnarmed/WalkForward_Unarmed_Shadow_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });

    let initial_dir = FacingDir::South.to_female_dir_index();
    let initial_body   = idle_body[initial_dir].clone();
    let initial_shadow = idle_shadow[initial_dir].clone();

    let female_sprites = FemaleSprites {
        idle_body,
        idle_shadow,
        walk_body,
        walk_shadow,
        idle_layout: idle_layout.clone(),
        walk_layout: walk_layout.clone(),
    };

    commands
        .spawn((
            Player,
            CharacterKind::Female,
            YSort,
            pos,
            female_sprites,
            PlayerAnimation::new(),
            TorchFlicker::default(),
            LightType::default(),
            Sprite {
                image: initial_body,
                texture_atlas: Some(TextureAtlas {
                    layout: idle_layout,
                    index: 0,
                }),
                // Ground-contact point at the feet, matching the isometric
                // tile anchor convention so the character stands on the floor.
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
            // Shadow sprite drawn just behind the body (z = -0.001 in local space).
            parent.spawn((
                PlayerShadow,
                Sprite {
                    image: initial_shadow,
                    texture_atlas: Some(TextureAtlas {
                        layout: walk_layout,
                        index: 0,
                    }),
                    anchor: Anchor::Custom(Vec2::new(0.0, -0.30)),
                    ..Default::default()
                },
                Transform::from_xyz(0.0, 0.0, -0.001),
            ));
        });

    // Spawn free-standing beam-light entities for the lantern.
    // Inactive (intensity = 0) while the player uses the Torch light type.
    // `apply_light_type` repositions and activates them each frame as needed.
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

// ── SystemParam bundles ───────────────────────────────────────────────────────

/// Converts the cursor's screen position to isometric world-space.
/// Bundles the window and camera queries needed for `viewport_to_world_2d`.
#[derive(SystemParam)]
struct WorldCursor<'w, 's> {
    windows: Query<'w, 's, &'static Window, With<PrimaryWindow>>,
    camera: Query<'w, 's, (&'static Camera, &'static GlobalTransform), With<MainCamera>>,
}

impl WorldCursor<'_, '_> {
    /// Returns the cursor's current world-space position, or `None` if the
    /// cursor is outside the window or the camera query fails.
    fn world_pos(&self) -> Option<Vec2> {
        let window = self.windows.get_single().ok()?;
        let (camera, camera_tf) = self.camera.get_single().ok()?;
        let cursor = window.cursor_position()?;
        camera.viewport_to_world_2d(camera_tf, cursor).ok()
    }
}

/// Door registry + read-only door query, bundled for use in movement and
/// click-handling systems that need to check closed-door positions.
#[derive(SystemParam)]
struct DoorParams<'w, 's> {
    registry: Res<'w, DoorRegistry>,
    doors: Query<'w, 's, &'static Door>,
}

impl DoorParams<'_, '_> {
    fn closed_positions(&self) -> std::collections::HashSet<(i32, i32)> {
        closed_door_positions(&self.registry, &self.doors)
    }

    fn is_closed_at(&self, pos: (i32, i32)) -> bool {
        self.registry
            .0
            .get(&pos)
            .and_then(|&e| self.doors.get(e).ok())
            .is_some_and(|d| !d.open)
    }
}

// ── Update system: keyboard movement ─────────────────────────────────────────

fn player_movement(
    keyboard: Res<ButtonInput<KeyCode>>,
    dungeon: Res<Dungeon>,
    doors: DoorParams<'_, '_>,
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

    let map = dungeon.current_map();

    // On a StairsMid tile, W/S (dy ≠ 0) navigate the shaft rather than
    // moving on the floor — let interact_with_stairs consume those presses.
    if dy != 0 && map.tiles[map.idx(pos.x, pos.y)] == TileType::StairsMid {
        return;
    }

    if map.is_passable(new_x, new_y) && !doors.is_closed_at((new_x, new_y)) {
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
    cursor: WorldCursor<'_, '_>,
    dungeon: Res<Dungeon>,
    doors: DoorParams<'_, '_>,
    mut click_state: ResMut<ClickState>,
    mut player_q: Query<(&MapPosition, &mut PlayerAnimation), With<Player>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Some(world_pos) = cursor.world_pos() else { return; };

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

    let map = dungeon.current_map();
    if !map.is_passable(target_x, target_y) {
        return;
    }

    let Ok((pos, mut anim)) = player_q.get_single_mut() else { return; };

    let closed_doors = doors.closed_positions();
    if let Some(path) = bfs_path(map, &closed_doors, (pos.x, pos.y), (target_x, target_y)) {
        anim.path = path;
        anim.step_timer.reset();
    }
}

// ── Update system: advance one tile along the auto-travel path ────────────────

fn auto_step(
    time: Res<Time>,
    dungeon: Res<Dungeon>,
    doors: DoorParams<'_, '_>,
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

    // Re-validate in case a door was closed along the auto-travel path.
    let map = dungeon.current_map();
    if !map.is_passable(nx, ny) || doors.is_closed_at((nx, ny)) {
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
    mut player_q: Query<
        (
            &mut Sprite,
            &mut PlayerAnimation,
            &CharacterKind,
            Option<&PlayerSprites>,
            Option<&FemaleSprites>,
            Option<&Children>,
        ),
        With<Player>,
    >,
    mut shadow_q: Query<&mut Sprite, (With<PlayerShadow>, Without<Player>)>,
) {
    let Ok((mut sprite, mut anim, kind, male_sprites, female_sprites, children)) =
        player_q.get_single_mut()
    else {
        moving.0 = false;
        return;
    };

    // Advance run-cooldown and transition to idle when it expires.
    if anim.running {
        anim.run_cooldown.tick(time.delta());
        if anim.run_cooldown.finished() {
            anim.running = false;
            anim.frame = 0;
        }
    }

    moving.0 = anim.running;

    match kind {
        CharacterKind::Male => {
            let Some(sprites) = male_sprites else { return };
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

        CharacterKind::Female => {
            let Some(sprites) = female_sprites else { return };
            let dir_i = anim.facing.to_female_dir_index();

            // Advance the frame timer for both idle and walk — Female idle is
            // also an animated loop (16 frames), unlike the Male single frame.
            anim.frame_timer.tick(time.delta());
            if anim.frame_timer.just_finished() {
                let max = if anim.running {
                    FEMALE_WALK_FRAME_COUNT
                } else {
                    FEMALE_IDLE_FRAME_COUNT
                };
                anim.frame = (anim.frame + 1) % max;
            }

            let (body_img, shadow_img, layout) = if anim.running {
                (
                    sprites.walk_body[dir_i].clone(),
                    sprites.walk_shadow[dir_i].clone(),
                    sprites.walk_layout.clone(),
                )
            } else {
                (
                    sprites.idle_body[dir_i].clone(),
                    sprites.idle_shadow[dir_i].clone(),
                    sprites.idle_layout.clone(),
                )
            };

            sprite.image = body_img;
            if let Some(atlas) = &mut sprite.texture_atlas {
                atlas.layout = layout.clone();
                atlas.index = anim.frame;
            }

            // Keep the shadow child in sync with the body.
            if let Some(children) = children {
                for &child in children.iter() {
                    if let Ok(mut shadow_sprite) = shadow_q.get_mut(child) {
                        shadow_sprite.image = shadow_img.clone();
                        if let Some(atlas) = &mut shadow_sprite.texture_atlas {
                            atlas.layout = layout.clone();
                            atlas.index = anim.frame;
                        }
                    }
                }
            }
        }
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
    dungeon: Res<Dungeon>,
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
        Some(dir) => wall_cast(dungeon.current_map(), player_pos, dir, BEAM_MAX_DIST),
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

// ── Update system: interact with adjacent doors ───────────────────────────────

/// Press **E** to toggle a door adjacent (4-directional) to the player.
///
/// - Swaps the sprite between `stoneWallDoorOpen_*` and `stoneWallDoorClosed_*`.
/// - Flips `door.open`.
/// - Only one door is toggled per keypress (the first match in NESW order).
/// - Does nothing when the player is standing on a stair tile (stairs take priority).
fn interact_with_doors(
    keyboard: Res<ButtonInput<KeyCode>>,
    player_q: Query<&MapPosition, With<Player>>,
    dungeon: Res<Dungeon>,
    door_registry: Res<DoorRegistry>,
    mut door_q: Query<(&mut Door, &mut Sprite)>,
    asset_server: Res<AssetServer>,
) {
    if !keyboard.just_pressed(KeyCode::KeyE) {
        return;
    }
    let Ok(pos) = player_q.get_single() else {
        return;
    };

    // Any stair tile takes priority — let interact_with_stairs handle it.
    let map = dungeon.current_map();
    if map.tiles[map.idx(pos.x, pos.y)].is_stair() {
        return;
    }

    for (dx, dy) in [(0_i32, 1_i32), (0, -1), (1, 0), (-1, 0)] {
        let adj = (pos.x + dx, pos.y + dy);
        let Some(&entity) = door_registry.0.get(&adj) else {
            continue;
        };
        let Ok((mut door, mut sprite)) = door_q.get_mut(entity) else {
            continue;
        };

        door.open = !door.open;
        let state = if door.open { "Open" } else { "Closed" };
        let dir = door.facing.as_str();
        sprite.image = asset_server.load(format!("Isometric/stoneWallDoor{state}_{dir}.png"));
        break; // one door per keypress
    }
}

// ── Event: level transition ───────────────────────────────────────────────────

/// Fired when the player uses a stair tile.  Consumed by
/// [`execute_level_transition`] in the same frame to swap the active floor.
#[derive(Event, Clone, Copy)]
pub struct LevelTransition {
    pub destination_floor: usize,
    pub exit_pos: (i32, i32),
}

// ── Update system: interact with stair tiles ──────────────────────────────────

/// Handles stair traversal for all three stair tile types:
///
/// - **`StairsDown`** — press **E** to descend.
/// - **`StairsUp`**   — press **E** to ascend.
/// - **`StairsMid`**  — press **W / ↑** to ascend, **S / ↓** to descend.
///   (Vertical movement keys are blocked in `player_movement` when on this tile.)
///
/// Fires a [`LevelTransition`] event consumed by [`execute_level_transition`].
fn interact_with_stairs(
    keyboard: Res<ButtonInput<KeyCode>>,
    player_q: Query<&MapPosition, With<Player>>,
    dungeon: Res<Dungeon>,
    mut events: EventWriter<LevelTransition>,
) {
    let Ok(pos) = player_q.get_single() else { return; };
    let map = dungeon.current_map();
    let tile = map.tiles[map.idx(pos.x, pos.y)];
    let Some(node) = map.stair_links.get(&(pos.x, pos.y)) else { return; };

    let link = match tile {
        TileType::StairsDown if keyboard.just_pressed(KeyCode::KeyE) => node.down.as_ref(),
        TileType::StairsUp   if keyboard.just_pressed(KeyCode::KeyE) => node.up.as_ref(),
        TileType::StairsMid => {
            let up   = keyboard.just_pressed(KeyCode::KeyW) || keyboard.just_pressed(KeyCode::ArrowUp);
            let down = keyboard.just_pressed(KeyCode::KeyS) || keyboard.just_pressed(KeyCode::ArrowDown);
            if up        { node.up.as_ref()   }
            else if down { node.down.as_ref() }
            else         { None }
        }
        _ => None,
    };

    if let Some(link) = link {
        events.send(LevelTransition {
            destination_floor: link.target_floor,
            exit_pos: link.target_pos,
        });
    }
}

// ── Update system: execute a level transition ─────────────────────────────────

/// Consumes a [`LevelTransition`] event: despawns all current floor tiles and
/// door entities, switches `Dungeon::current_floor`, spawns the new floor's
/// tiles and doors, and teleports the player to the exit stair position.
fn execute_level_transition(
    mut commands: Commands,
    mut dungeon: ResMut<Dungeon>,
    mut events: EventReader<LevelTransition>,
    tiles_q: Query<Entity, With<MapTile>>,
    mut registry: ResMut<DoorRegistry>,
    asset_server: Res<AssetServer>,
    mut player_q: Query<(&mut MapPosition, &mut Transform, &mut PlayerAnimation), With<Player>>,
) {
    // Only handle the first event per frame; discard any extras.
    let Some(ev) = events.read().next() else { return; };
    let destination_floor = ev.destination_floor;
    let exit_pos = ev.exit_pos;

    // Despawn all current floor entities (tiles, walls, doors).
    for entity in &tiles_q {
        commands.entity(entity).despawn();
    }
    registry.0.clear();

    // Activate the new floor.
    dungeon.current_floor = destination_floor;

    // Spawn new floor geometry and doors.
    spawn_floor_tiles(&mut commands, &dungeon.floors[destination_floor], &asset_server);
    spawn_floor_doors(
        &mut commands,
        &dungeon.floors[destination_floor],
        &asset_server,
        &mut registry,
    );

    // Teleport the player to the landing stair and cancel any auto-travel path.
    if let Ok((mut pos, mut transform, mut anim)) = player_q.get_single_mut() {
        anim.path.clear();
        pos.x = exit_pos.0;
        pos.y = exit_pos.1;
        let world = pos.to_world(0.0);
        transform.translation.x = world.x;
        transform.translation.y = world.y;
    }
}

// ── Update system: hide map tiles outside the player's light envelope ─────────

/// Hides every `MapTile` that falls outside the player's current light
/// envelope so unlit areas are not rendered at all.
///
/// The envelope is the union of two regions:
///
/// - **Base circle** — centred on the player:
///   - *Torch*: current flickering `PointLight2d` radius plus `TORCH_RADIUS_VAR`
///     as a margin, preventing tiles from popping in and out at the flame's
///     dancing edge.
///   - *Lantern*: the fixed base radius.
///   - *Dark*: 0 — every tile is hidden.
///
/// - **Beam circles** (lantern only) — one circle per active beam-segment
///   entity (those with `intensity > 0`), using each segment's own world
///   position and `PointLight2d` radius.  This correctly restricts visibility
///   to the beam direction rather than a full ring.
fn cull_map_tiles(
    player_q: Query<(&Transform, &PointLight2d, &LightType), With<Player>>,
    beam_q: Query<(&Transform, &PointLight2d), With<LanternBeamLight>>,
    mut tile_q: Query<
        (&Transform, &mut Visibility, Option<&StairsUpTile>, Option<&StairsMidTile>),
        With<MapTile>,
    >,
) {
    let Ok((player_tf, player_light, light_type)) = player_q.get_single() else {
        return;
    };
    let player_pos = player_tf.translation.truncate();

    let base_radius = match *light_type {
        LightType::Dark => 0.0,
        LightType::Torch => player_light.radius + TORCH_RADIUS_VAR,
        LightType::Lantern => player_light.radius,
    };

    // Collect active beam-segment circles (position + radius).
    let beams: Vec<(Vec2, f32)> = beam_q
        .iter()
        .filter(|(_, l)| l.intensity > 0.0)
        .map(|(tf, l)| (tf.translation.truncate(), l.radius))
        .collect();

    for (tile_tf, mut vis, stairs_up, stairs_mid) in tile_q.iter_mut() {
        // StairsUp and StairsMid tiles both have their shaft open from above;
        // ambient light from the opening keeps them always visible.
        if stairs_up.is_some() || stairs_mid.is_some() {
            *vis = Visibility::Inherited;
            continue;
        }

        let tile_pos = tile_tf.translation.truncate();
        let in_base = (tile_pos - player_pos).length() <= base_radius;
        let in_beam = beams.iter().any(|&(bp, br)| (tile_pos - bp).length() <= br);

        *vis = if in_base || in_beam {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerMoving>()
            .init_resource::<ClickState>()
            .add_event::<LevelTransition>()
            .add_systems(Startup, spawn_player)
            .add_systems(
                Update,
                (
                    player_movement,
                    handle_mouse_click,
                    auto_step.after(player_movement),
                    animate_player,
                    interact_with_stairs,
                    execute_level_transition.after(interact_with_stairs),
                    interact_with_doors.after(interact_with_stairs),
                    toggle_light_type,
                    flicker_torch.after(toggle_light_type),
                    apply_light_type.after(toggle_light_type),
                    cull_map_tiles.after(apply_light_type),
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
        let no_doors = std::collections::HashSet::new();
        if (ax, ay) == (bx, by) {
            assert!(bfs_path(&map, &no_doors, (ax, ay), (bx, by)).unwrap().is_empty());
            return;
        }
        let path = bfs_path(&map, &no_doors, (ax, ay), (bx, by));
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
        let path = bfs_path(&map, &Default::default(), (cx, cy), (cx, cy)).unwrap();
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
    fn bfs_closed_door_blocks_path() {
        let map = generate_map();
        let door = &map.doors[0];
        let door_pos = (door.x, door.y);
        let (cx, cy) = map.rooms[0].center();

        // The door tile is Floor, so BFS can reach it with an empty closed set.
        let no_doors: std::collections::HashSet<(i32, i32)> = Default::default();
        assert!(
            bfs_path(&map, &no_doors, (cx, cy), door_pos).is_some(),
            "BFS should reach the door tile when it is not in the closed set"
        );

        // When the door position is in the closed set, BFS must not reach it —
        // closed-door positions are never pushed onto the queue, so they cannot
        // be popped as the goal either.
        let closed = std::collections::HashSet::from([door_pos]);
        assert!(
            bfs_path(&map, &closed, (cx, cy), door_pos).is_none(),
            "BFS must not reach a tile listed in the closed-door set"
        );
    }

    #[test]
    fn bfs_unreachable_returns_none() {
        let map = generate_map();
        // (0,0) is a wall tile and is surrounded by walls — completely isolated.
        let (cx, cy) = map.rooms[0].center();
        // Try to reach a position that is guaranteed to be a wall with no floor
        // neighbours; (0,0) is always wall in our generator.
        assert!(bfs_path(&map, &Default::default(), (cx, cy), (0, 0)).is_none());
    }

    // ── cull_map_tiles radius logic ───────────────────────────────────────────

    /// Mirrors the base-radius formula in `cull_map_tiles` so changes in that
    /// function must also update this test.
    fn cull_base_radius(light_type: LightType, light_radius: f32) -> f32 {
        match light_type {
            LightType::Dark => 0.0,
            LightType::Torch => light_radius + TORCH_RADIUS_VAR,
            LightType::Lantern => light_radius,
        }
    }

    #[test]
    fn dark_mode_base_radius_is_zero() {
        assert_eq!(cull_base_radius(LightType::Dark, TORCH_RADIUS), 0.0);
    }

    #[test]
    fn torch_mode_base_radius_adds_flicker_margin() {
        let radius = cull_base_radius(LightType::Torch, TORCH_RADIUS);
        assert_eq!(radius, TORCH_RADIUS + TORCH_RADIUS_VAR);
    }

    #[test]
    fn torch_cull_radius_covers_max_flicker() {
        // The max flickered radius is TORCH_RADIUS + TORCH_RADIUS_VAR.
        // The cull radius (with margin) must be >= that, so no tile can pop
        // out at peak flicker.
        let max_flicker_radius = TORCH_RADIUS + TORCH_RADIUS_VAR;
        let cull = cull_base_radius(LightType::Torch, TORCH_RADIUS);
        assert!(
            cull >= max_flicker_radius,
            "cull radius {cull} must cover max flicker radius {max_flicker_radius}"
        );
    }

    #[test]
    fn lantern_base_radius_equals_light_radius() {
        let radius = cull_base_radius(LightType::Lantern, LANTERN_RADIUS);
        assert_eq!(radius, LANTERN_RADIUS);
    }
}
