use std::collections::HashMap;

use bevy::{ecs::system::SystemParam, prelude::*, sprite::Anchor, window::PrimaryWindow};
use bevy_light_2d::prelude::*;

use crate::{
    components::{CharacterKind, ChestContents, Door, DoorState, ItemKind, MainCamera, MapPosition, MapTile, Player, PropTile, StairsMidTile, StairsUpTile, YSort, YSortBias},
    inventory::{Inventory, SelectedSlot, UseItemEvent},
    log::GameMessage,
    map::{spawn_floor_doors, spawn_floor_tiles, DoorRegistry, Dungeon, Map, TileType, VoidOutcome},
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
/// Seconds per run animation frame (≈10 fps).  Used for Male and as fallback.
const RUN_FRAME_SECS: f32 = 0.1;
/// Seconds per animation frame during female auto-travel walk (slower pace).
const FEMALE_AUTO_WALK_FRAME_SECS: f32 = 0.08;
/// Seconds per animation frame during female auto-travel run (faster pace).
const FEMALE_AUTO_RUN_FRAME_SECS: f32 = 0.055;

// ── Female character constants ────────────────────────────────────────────────

/// Number of facing directions in the Female asset pack (every 22.5°).
const FEMALE_DIR_COUNT: usize = 16;
/// Frames in each Female walk spritesheet (6 columns × 4 rows).
const FEMALE_WALK_FRAME_COUNT: usize = 24;
/// Frames in each Female walk-back spritesheet (6 columns × 4 rows, same grid).
const FEMALE_WALKBACK_FRAME_COUNT: usize = 24;
/// Frames in each Female run spritesheet (5 columns × 4 rows).
const FEMALE_RUN_FRAME_COUNT: usize = 20;
/// Frames in each Female idle spritesheet (4 columns × 4 rows).
const FEMALE_IDLE_FRAME_COUNT: usize = 16;
/// Frames in each Female jump spritesheet (6 columns × 4 rows, same grid as walk).
const FEMALE_JUMP_FRAME_COUNT: usize = 24;
/// Seconds per jump animation frame (≈25 fps).  The full arc plays in ~0.96 s.
const FEMALE_JUMP_FRAME_SECS: f32 = 0.06;
/// Seconds per frame during the walk-back phase of a jump.
const JUMP_WALKBACK_FRAME_SECS: f32 = 0.04;
/// Seconds per frame during the run-forward phase of a jump.
const JUMP_RUNFWD_FRAME_SECS: f32 = 0.035;
/// Peak height of the jump arc in world units (screen-space pixels at TILE_SCALE).
const JUMP_ARC_HEIGHT: f32 = 30.0;
/// Cell size of each Female spritesheet frame, in pixels.
const FEMALE_CELL_PX: u32 = 256;
/// Sprite anchor for the Female character.
///
/// Bevy anchor coordinates: −0.5 = bottom edge, +0.5 = top edge.
/// The Female 256×256 cells have empty space below the feet; the feet sit at
/// roughly 32 % from the bottom, giving an anchor of −0.5 + 0.32 = −0.18.
/// Tune this value if the character still floats or sinks into the floor.
const FEMALE_ANCHOR: Vec2 = Vec2::new(0.0, -0.18);
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
/// Seconds per tile during auto-travel walk steps (first, last, or short path).
/// Original suggestion was 0.30 and 0.20 for walk / run.
const AUTO_WALK_STEP_SECS: f32 = 0.45;
/// Seconds per tile during auto-travel run steps (middle of a long path).
/// Faster than walk so running feels like covering ground more quickly.
const AUTO_RUN_STEP_SECS: f32 = 0.30;
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
        // The Female asset pack uses 0° = screen-up (NW in isometric), rotating
        // clockwise.  Derived from the projection world_x=(dx-dy)*ISO_STEP_X,
        // world_y=-(dx+dy)*ISO_STEP_Y:
        //   NW → pure up     →   0°
        //   N  → right+up    →  45°
        //   NE → pure right  →  90°
        //   E  → right+down  → 135°
        //   SE → pure down   → 180°
        //   S  → left+down   → 225°
        //   SW → pure left   → 270°
        //   W  → left+up     → 315°
        let deg: u32 = match self {
            FacingDir::NorthWest =>   0,
            FacingDir::North     =>  45,
            FacingDir::NorthEast =>  90,
            FacingDir::East      => 135,
            FacingDir::SouthEast => 180,
            FacingDir::South     => 225,
            FacingDir::SouthWest => 270,
            FacingDir::West      => 315,
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
    /// Body layer walk-back sheets, one per direction (6×4 grid, 256×256 px cells).
    walkback_body:   [Handle<Image>; FEMALE_DIR_COUNT],
    /// Shadow layer walk-back sheets, one per direction.
    walkback_shadow: [Handle<Image>; FEMALE_DIR_COUNT],
    /// Body layer run sheets, one per direction (5×4 grid, 256×256 px cells).
    run_body:    [Handle<Image>; FEMALE_DIR_COUNT],
    /// Shadow layer run sheets, one per direction.
    run_shadow:  [Handle<Image>; FEMALE_DIR_COUNT],
    /// Body layer jump sheets, one per direction (6×4 grid, 256×256 px cells).
    jump_body:   [Handle<Image>; FEMALE_DIR_COUNT],
    /// Shadow layer jump sheets, one per direction.
    jump_shadow: [Handle<Image>; FEMALE_DIR_COUNT],
    /// Shared atlas layout for all idle sheets (4 columns × 4 rows).
    idle_layout: Handle<TextureAtlasLayout>,
    /// Shared atlas layout for all walk sheets (6 columns × 4 rows).
    walk_layout: Handle<TextureAtlasLayout>,
    /// Shared atlas layout for all run sheets (5 columns × 4 rows).
    run_layout:  Handle<TextureAtlasLayout>,
    /// Shared atlas layout for all jump sheets (6 columns × 4 rows, same as walk).
    jump_layout: Handle<TextureAtlasLayout>,
}

/// The three phases of a jump sequence: walk-back, run-forward, arc.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum JumpPhase {
    /// Walk backwards one tile (using WalkBack_Unarmed sprites).
    /// Facing direction is preserved.
    #[default]
    WalkBack,
    /// Run forward one tile back to the origin (using Run sprites).
    RunForward,
    /// Parabolic jump arc over two tiles (using Jump sprites).
    Arc,
}

#[derive(Component)]
struct PlayerAnimation {
    facing: FacingDir,
    /// True while the player is in motion (walk or run); false when idle.
    running: bool,
    /// True when motion was triggered by mouse auto-travel (run animation);
    /// false when triggered by keyboard (walk animation).
    auto_running: bool,
    frame: usize,
    /// Advances through animation frames while moving.
    frame_timer: Timer,
    /// Reset on each step; idle resumes when this expires.
    run_cooldown: Timer,
    /// Remaining steps for mouse-driven auto-travel.
    /// Stored in *reverse* order so `pop()` yields the next step.
    path: Vec<(i32, i32)>,
    /// Total length of the path when it was first assigned.  Used to
    /// determine whether a given step is the first or last of the journey
    /// so the walk→run→walk envelope can be applied correctly.
    path_total: usize,
    /// Fires to advance one tile along the path; duration adjusts per step
    /// between [`AUTO_WALK_STEP_SECS`] and [`AUTO_RUN_STEP_SECS`].
    step_timer: Timer,

    // ── Auto-travel interpolation ─────────────────────────────────────────────
    /// World-space position at the start of the current interpolated step.
    lerp_from: Vec3,
    /// World-space position at the end of the current interpolated step.
    lerp_to: Vec3,
    /// True while an auto-travel step is being interpolated between tiles.
    lerping: bool,

    // ── Jump state ────────────────────────────────────────────────────────────
    /// True while any jump phase is in progress.  Blocks all other movement.
    jumping: bool,
    /// Current phase of the three-phase jump sequence.
    jump_phase: JumpPhase,
    /// The grid tile the jump will land on (two tiles ahead of origin).
    jump_target: (i32, i32),
    /// The tile the player started the jump on (before walking back).
    jump_origin: (i32, i32),
    /// The tile one step behind the origin (walk-back destination).
    jump_back_tile: (i32, i32),
    /// Set to `true` once [`MapPosition`] has been updated to `jump_target`
    /// (happens at the arc midpoint so the player snaps onto the landing tile
    /// at the peak of the jump rather than at touchdown).
    jump_midpoint_reached: bool,
    /// Current frame index in the active jump-phase spritesheet.
    jump_frame: usize,
    /// Per-frame timer for the active jump-phase animation.
    jump_frame_timer: Timer,
}

impl PlayerAnimation {
    fn new() -> Self {
        Self {
            facing: FacingDir::default(),
            running: false,
            auto_running: false,
            frame: 0,
            frame_timer: Timer::from_seconds(RUN_FRAME_SECS, TimerMode::Repeating),
            run_cooldown: Timer::from_seconds(
                RUN_FRAME_SECS * RUN_FRAME_COUNT as f32,
                TimerMode::Once,
            ),
            path: Vec::new(),
            path_total: 0,
            step_timer: Timer::from_seconds(AUTO_WALK_STEP_SECS, TimerMode::Repeating),
            lerp_from: Vec3::ZERO,
            lerp_to: Vec3::ZERO,
            lerping: false,
            jumping: false,
            jump_phase: JumpPhase::default(),
            jump_target: (0, 0),
            jump_origin: (0, 0),
            jump_back_tile: (0, 0),
            jump_midpoint_reached: false,
            jump_frame: 0,
            jump_frame_timer: Timer::from_seconds(JUMP_WALKBACK_FRAME_SECS, TimerMode::Repeating),
        }
    }

    /// Begin the three-phase jump sequence.
    ///
    /// `origin` is the player's starting tile, `back_tile` is one tile behind
    /// (walk-back destination), and `target` is two tiles ahead (landing tile).
    fn trigger_jump(&mut self, origin: (i32, i32), back_tile: (i32, i32), target: (i32, i32)) {
        self.jumping = true;
        self.jump_phase = JumpPhase::WalkBack;
        self.jump_origin = origin;
        self.jump_back_tile = back_tile;
        self.jump_target = target;
        self.jump_midpoint_reached = false;
        self.jump_frame = 0;
        self.jump_frame_timer
            .set_duration(std::time::Duration::from_secs_f32(JUMP_WALKBACK_FRAME_SECS));
        self.jump_frame_timer.reset();
        // Clear motion state so idle resumes after landing.
        self.running = false;
        self.path.clear();
        self.lerping = false;
    }

    /// Transition from the current jump phase to the next, resetting frame
    /// counters and adjusting the per-frame timer duration.  Returns `true`
    /// if the jump sequence is complete (Arc phase finished).
    fn advance_jump_phase(&mut self) -> bool {
        match self.jump_phase {
            JumpPhase::WalkBack => {
                self.jump_phase = JumpPhase::RunForward;
                self.jump_frame = 0;
                self.jump_frame_timer
                    .set_duration(std::time::Duration::from_secs_f32(JUMP_RUNFWD_FRAME_SECS));
                self.jump_frame_timer.reset();
                false
            }
            JumpPhase::RunForward => {
                self.jump_phase = JumpPhase::Arc;
                self.jump_frame = 0;
                self.jump_frame_timer
                    .set_duration(std::time::Duration::from_secs_f32(FEMALE_JUMP_FRAME_SECS));
                self.jump_frame_timer.reset();
                false
            }
            JumpPhase::Arc => true,
        }
    }

    /// Keyboard step or auto-travel walk step: plays the walk animation.
    fn trigger_walk(&mut self, facing: FacingDir) {
        // Reset frame counter when switching from run to walk to avoid
        // an out-of-range atlas index (run has fewer frames than walk).
        if self.auto_running {
            self.frame = 0;
        }
        self.facing = facing;
        self.running = true;
        self.auto_running = false;
        self.run_cooldown
            .set_duration(std::time::Duration::from_secs_f32(AUTO_WALK_STEP_SECS));
        self.run_cooldown.reset();
        self.frame_timer
            .set_duration(std::time::Duration::from_secs_f32(FEMALE_AUTO_WALK_FRAME_SECS));
        self.step_timer
            .set_duration(std::time::Duration::from_secs_f32(AUTO_WALK_STEP_SECS));
    }

    /// Auto-travel run step (middle of a long path): plays the run animation.
    fn trigger_run(&mut self, facing: FacingDir) {
        // Reset frame counter when switching from walk to run to avoid
        // starting mid-cycle with a walk-length frame index.
        if !self.auto_running {
            self.frame = 0;
        }
        self.facing = facing;
        self.running = true;
        self.auto_running = true;
        self.run_cooldown
            .set_duration(std::time::Duration::from_secs_f32(AUTO_RUN_STEP_SECS));
        self.run_cooldown.reset();
        self.frame_timer
            .set_duration(std::time::Duration::from_secs_f32(FEMALE_AUTO_RUN_FRAME_SECS));
        self.step_timer
            .set_duration(std::time::Duration::from_secs_f32(AUTO_RUN_STEP_SECS));
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
            door_q.get(**entity).map(|d| !d.is_passable()).unwrap_or(false)
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
    // Run sheets: 5 columns × 4 rows, each cell 256×256 px → 20 frames.
    let run_layout = layouts.add(TextureAtlasLayout::from_grid(
        UVec2::splat(FEMALE_CELL_PX),
        5,
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
    let walkback_body: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/WalkBack_Unarmed/WalkBack_Unarmed_Body_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let walkback_shadow: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/WalkBack_Unarmed/WalkBack_Unarmed_Shadow_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let run_body: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/RunUnarmed/Run_Unarmed_Body_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let run_shadow: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/RunUnarmed/Run_Unarmed_Shadow_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let jump_body: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/Jump_Unarmed/Jump_Unarmed_Body_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    let jump_shadow: [Handle<Image>; FEMALE_DIR_COUNT] = std::array::from_fn(|i| {
        asset_server.load(format!(
            "Characters/Female/Jump_Unarmed/Jump_Unarmed_Shadow_{:03}.png",
            FEMALE_ANGLES[i]
        ))
    });
    // Jump sheets use the same 6×4 grid as the walk sheets.
    let jump_layout = walk_layout.clone();

    let initial_dir = FacingDir::South.to_female_dir_index();
    let initial_body   = idle_body[initial_dir].clone();
    let initial_shadow = idle_shadow[initial_dir].clone();

    let female_sprites = FemaleSprites {
        idle_body,
        idle_shadow,
        walk_body,
        walk_shadow,
        walkback_body,
        walkback_shadow,
        run_body,
        run_shadow,
        jump_body,
        jump_shadow,
        idle_layout: idle_layout.clone(),
        walk_layout: walk_layout.clone(),
        run_layout: run_layout.clone(),
        jump_layout,
    };

    commands
        .spawn((
            Player,
            CharacterKind::Female,
            YSort,
            YSortBias(0.001),
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
                anchor: Anchor::Custom(FEMALE_ANCHOR),
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
                    anchor: Anchor::Custom(FEMALE_ANCHOR),
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

/// Convert a `FacingDir` to the grid delta (dx, dy) used for jump targeting.
/// Diagonal facings produce diagonal deltas, giving 8-directional jump support.
fn facing_to_grid_delta(facing: FacingDir) -> (i32, i32) {
    match facing {
        FacingDir::North     => ( 0, -1),
        FacingDir::NorthEast => ( 1, -1),
        FacingDir::East      => ( 1,  0),
        FacingDir::SouthEast => ( 1,  1),
        FacingDir::South     => ( 0,  1),
        FacingDir::SouthWest => (-1,  1),
        FacingDir::West      => (-1,  0),
        FacingDir::NorthWest => (-1, -1),
    }
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
            .is_some_and(|d| !d.is_passable())
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

    // Movement is locked while a jump arc is in progress.
    if anim.jumping {
        return;
    }

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
        anim.trigger_walk(facing);
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
    inventory: Res<Inventory>,
    mut selected_slot: ResMut<SelectedSlot>,
    mut use_item_events: EventWriter<UseItemEvent>,
    mut player_q: Query<(&MapPosition, &mut PlayerAnimation), With<Player>>,
) {
    if !mouse.just_pressed(MouseButton::Left) {
        return;
    }

    let Some(world_pos) = cursor.world_pos() else { return; };

    // Invert the isometric projection:
    //   wx = (gx - gy) * ISO_STEP_X  →  gx - gy = wx / ISO_STEP_X
    //   wy = -(gx + gy) * ISO_STEP_Y →  gx + gy = -wy / ISO_STEP_Y
    let sum  = -world_pos.y / ISO_STEP_Y;
    let diff =  world_pos.x / ISO_STEP_X;
    let target_x = ((diff + sum) / 2.0).round() as i32;
    let target_y = ((sum  - diff) / 2.0).round() as i32;

    // If an inventory item is selected, use it on the clicked tile (any tile).
    if let Some(slot_idx) = selected_slot.0 {
        if let Some(&item) = inventory.items().get(slot_idx) {
            use_item_events.send(UseItemEvent { item, target: (target_x, target_y) });
            selected_slot.0 = None;
            return;
        }
        // Slot no longer has an item — clear stale selection and fall through.
        selected_slot.0 = None;
    }

    let Ok((pos, mut anim)) = player_q.get_single_mut() else { return; };

    // Don't process clicks while jumping or auto-travelling.
    if anim.jumping || !anim.path.is_empty() {
        return;
    }

    let now = time.elapsed_secs();
    let is_double = (now - click_state.last_click_time) < DOUBLE_CLICK_SECS;
    click_state.last_click_time = now;

    let dx = target_x - pos.x;
    let dy = target_y - pos.y;

    if !is_double {
        // Single click: turn the player to face the clicked tile.
        if dx != 0 || dy != 0 {
            anim.facing = dir_to_facing(dx, dy);
        }
        return;
    }

    let map = dungeon.current_map();
    if !map.is_passable(target_x, target_y) {
        return;
    }

    let closed_doors = doors.closed_positions();
    if let Some(path) = bfs_path(map, &closed_doors, (pos.x, pos.y), (target_x, target_y)) {
        anim.path_total = path.len();
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

    if anim.jumping {
        return;
    }

    // If a lerp is active, interpolate and wait for the step timer.
    // This must run even when the path is empty — the last tile pop
    // empties the path but its lerp still needs to complete.
    if anim.lerping {
        let step_dur = anim.step_timer.duration().as_secs_f32();
        let elapsed  = anim.step_timer.elapsed_secs();
        let t = (elapsed / step_dur).clamp(0.0, 1.0);
        transform.translation.x = anim.lerp_from.x + (anim.lerp_to.x - anim.lerp_from.x) * t;
        transform.translation.y = anim.lerp_from.y + (anim.lerp_to.y - anim.lerp_from.y) * t;

        anim.step_timer.tick(time.delta());
        if !anim.step_timer.just_finished() {
            return;
        }

        // Snap to the exact destination.
        transform.translation.x = anim.lerp_to.x;
        transform.translation.y = anim.lerp_to.y;
        anim.lerping = false;
    }

    if anim.path.is_empty() {
        return;
    }

    // When lerping is false and the path is non-empty we either just
    // finished a step or the path was freshly assigned — pop the next
    // tile immediately so the first step starts without a timer delay.

    let Some((nx, ny)) = anim.path.pop() else { return; };

    // Re-validate in case a door was closed along the auto-travel path.
    let map = dungeon.current_map();
    if !map.is_passable(nx, ny) || doors.is_closed_at((nx, ny)) {
        anim.path.clear();
        return;
    }

    let facing = dir_to_facing(nx - pos.x, ny - pos.y);

    // Walk→run→walk envelope: use the run animation only for the middle
    // tiles of a path that is long enough to have a distinct run phase.
    // With path_total <= 2 the whole journey uses the walk animation.
    // With path_total >= 3 the first and last tiles walk; the rest run.
    let remaining = anim.path.len(); // steps still to go after this one
    let step_index = anim.path_total.saturating_sub(remaining + 1);
    let is_first = step_index == 0;
    let is_last  = remaining == 0;
    let use_run  = anim.path_total >= 3 && !is_first && !is_last;

    // Set up smooth interpolation from current position to next tile.
    anim.lerp_from = Vec3::new(transform.translation.x, transform.translation.y, 0.0);
    let next_pos = MapPosition::new(nx, ny);
    anim.lerp_to = next_pos.to_world(0.0);
    anim.lerping = true;
    anim.step_timer.reset();

    // Update MapPosition immediately so game logic tracks the new tile.
    pos.x = nx;
    pos.y = ny;

    if use_run {
        anim.trigger_run(facing);
    } else {
        anim.trigger_walk(facing);
    }
}

// ── Update system: drive the sprite animation ─────────────────────────────────

type AnimatePlayerItem = (
    &'static mut Sprite,
    &'static mut PlayerAnimation,
    &'static CharacterKind,
    Option<&'static PlayerSprites>,
    Option<&'static FemaleSprites>,
    Option<&'static Children>,
);

fn animate_player(
    time: Res<Time>,
    mut moving: ResMut<PlayerMoving>,
    mut player_q: Query<AnimatePlayerItem, With<Player>>,
    mut shadow_q: Query<&mut Sprite, (With<PlayerShadow>, Without<Player>)>,
    mut jump_land_events: EventWriter<JumpLandedEvent>,
) {
    let Ok((mut sprite, mut anim, kind, male_sprites, female_sprites, children)) =
        player_q.get_single_mut()
    else {
        moving.0 = false;
        return;
    };

    // Advance run-cooldown and transition to idle when it expires.
    // Skip the cooldown while auto-travel steps remain — the next auto_step
    // call will reset it, and expiring here would flash Idle for one frame.
    if anim.running && anim.path.is_empty() {
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

            // ── Jump animation (takes priority) ───────────────────────────────
            if anim.jumping {
                anim.jump_frame_timer.tick(time.delta());

                // Determine frame count for the current phase.
                let phase_frame_count = match anim.jump_phase {
                    JumpPhase::WalkBack   => FEMALE_WALKBACK_FRAME_COUNT,
                    JumpPhase::RunForward => FEMALE_RUN_FRAME_COUNT,
                    JumpPhase::Arc        => FEMALE_JUMP_FRAME_COUNT,
                };

                if anim.jump_frame_timer.just_finished() {
                    if anim.jump_frame + 1 >= phase_frame_count {
                        // Current phase complete — advance to the next one.
                        let finished = anim.advance_jump_phase();
                        if finished {
                            anim.jumping = false;
                            anim.jump_frame = FEMALE_JUMP_FRAME_COUNT - 1;
                            jump_land_events.send(JumpLandedEvent);
                        }
                    } else {
                        anim.jump_frame += 1;
                    }
                }

                // Select spritesheet and layout for the current phase.
                let (body_img, shadow_img, layout) = match anim.jump_phase {
                    JumpPhase::WalkBack => (
                        sprites.walkback_body[dir_i].clone(),
                        sprites.walkback_shadow[dir_i].clone(),
                        sprites.walk_layout.clone(), // same 6×4 grid
                    ),
                    JumpPhase::RunForward => (
                        sprites.run_body[dir_i].clone(),
                        sprites.run_shadow[dir_i].clone(),
                        sprites.run_layout.clone(),
                    ),
                    JumpPhase::Arc => (
                        sprites.jump_body[dir_i].clone(),
                        sprites.jump_shadow[dir_i].clone(),
                        sprites.jump_layout.clone(),
                    ),
                };

                sprite.image = body_img;
                if let Some(atlas) = &mut sprite.texture_atlas {
                    atlas.layout = layout.clone();
                    atlas.index  = anim.jump_frame;
                }
                if let Some(children) = children {
                    for &child in children.iter() {
                        if let Ok(mut shadow_sprite) = shadow_q.get_mut(child) {
                            shadow_sprite.image = shadow_img.clone();
                            if let Some(atlas) = &mut shadow_sprite.texture_atlas {
                                atlas.layout = layout.clone();
                                atlas.index  = anim.jump_frame;
                            }
                        }
                    }
                }
                return;
            }

            // ── Normal idle / walk / run animation ────────────────────────────

            // Advance the frame timer for all states — idle is also a 16-frame
            // animated loop, unlike the Male single-frame idle.
            anim.frame_timer.tick(time.delta());
            if anim.frame_timer.just_finished() {
                let max = if anim.running && anim.auto_running {
                    FEMALE_RUN_FRAME_COUNT
                } else if anim.running {
                    FEMALE_WALK_FRAME_COUNT
                } else {
                    FEMALE_IDLE_FRAME_COUNT
                };
                anim.frame = (anim.frame + 1) % max;
            }

            let (body_img, shadow_img, layout) = if anim.running && anim.auto_running {
                (
                    sprites.run_body[dir_i].clone(),
                    sprites.run_shadow[dir_i].clone(),
                    sprites.run_layout.clone(),
                )
            } else if anim.running {
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

// ── Update system: open adjacent chests ──────────────────────────────────────

/// Press **E** to open a closed chest adjacent (4-directional) to the player.
///
/// Opening a chest:
/// - Swaps the sprite to `chestOpen_*` using the stored facing direction.
/// - Transfers the contained item into [`Inventory`] (no-op if full).
/// - Removes the [`ChestContents`] component so the chest cannot be opened again.
fn interact_with_chests(
    keyboard: Res<ButtonInput<KeyCode>>,
    player_q: Query<&MapPosition, With<Player>>,
    dungeon: Res<Dungeon>,
    mut chest_q: Query<(Entity, &MapPosition, &ChestContents, &mut Sprite), With<PropTile>>,
    mut inventory: ResMut<Inventory>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    if !keyboard.just_pressed(KeyCode::KeyE) {
        return;
    }
    let Ok(pos) = player_q.get_single() else { return };
    let map = dungeon.current_map();
    if map.tiles[map.idx(pos.x, pos.y)].is_stair() {
        return;
    }

    for (entity, chest_pos, contents, mut sprite) in &mut chest_q {
        let dx = (chest_pos.x - pos.x).abs();
        let dy = (chest_pos.y - pos.y).abs();
        if dx + dy == 1 && inventory.add(contents.item) {
            let dir = contents.facing.as_str();
            sprite.image = asset_server.load(format!("Isometric/chestOpen_{dir}.png"));
            commands.entity(entity).remove::<ChestContents>();
            break;
        }
    }
}

// ── Update system: interact with adjacent doors ───────────────────────────────

/// Press **E** to toggle a door adjacent (4-directional) to the player.
///
/// - Swaps the sprite between `stoneWallDoorOpen_*` and `stoneWallDoorClosed_*`.
/// - Cycles `door.state` between [`DoorState::Closed`] and [`DoorState::Open`].
/// - Locked doors are silently skipped — use the correct item to unlock first.
/// - Only one door is toggled per keypress (the first match in NESW order).
/// - Does nothing when the player is standing on a stair tile (stairs take priority).
fn interact_with_doors(
    keyboard: Res<ButtonInput<KeyCode>>,
    player_q: Query<&MapPosition, With<Player>>,
    dungeon: Res<Dungeon>,
    door_registry: Res<DoorRegistry>,
    mut door_q: Query<(&mut Door, &mut Sprite)>,
    asset_server: Res<AssetServer>,
    mut log: EventWriter<GameMessage>,
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

        // Locked doors cannot be toggled with E.
        if door.state == DoorState::Locked {
            log.send(GameMessage::new("The door is locked. You cannot open it."));
            break;
        }

        door.state = match door.state {
            DoorState::Closed => DoorState::Open,
            DoorState::Open   => DoorState::Closed,
            DoorState::Locked => unreachable!(),
        };
        let state_str = if door.state == DoorState::Open { "Open" } else { "Closed" };
        let dir = door.facing.as_str();
        sprite.image = asset_server.load(format!("Isometric/stoneWallDoor{state_str}_{dir}.png"));
        break; // one door per keypress
    }
}

// ── Update system: apply item use ─────────────────────────────────────────────

/// Handles [`UseItemEvent`] fired when the player uses an inventory item on a
/// world tile.  Currently only the Key→Locked-door interaction is implemented.
fn apply_item_use(
    mut events: EventReader<UseItemEvent>,
    mut inventory: ResMut<Inventory>,
    door_registry: Res<DoorRegistry>,
    mut door_q: Query<(&mut Door, &mut Sprite)>,
    asset_server: Res<AssetServer>,
) {
    for ev in events.read() {
        match ev.item {
            ItemKind::Key => {
                let Some(&entity) = door_registry.0.get(&ev.target) else { continue };
                let Ok((mut door, mut sprite)) = door_q.get_mut(entity) else { continue };
                if door.state == DoorState::Locked {
                    door.state = DoorState::Closed;
                    let dir = door.facing.as_str();
                    sprite.image =
                        asset_server.load(format!("Isometric/stoneWallDoorClosed_{dir}.png"));
                    inventory.remove(ev.item);
                }
            }
        }
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

// ── Event: jump landed ────────────────────────────────────────────────────────

/// Fired by [`animate_player`] when a jump animation completes.
/// Consumed by [`on_jump_land`] to apply any [`VoidOutcome`].
#[derive(Event)]
struct JumpLandedEvent;

// ── Update system: J key triggers a jump ──────────────────────────────────────

/// Press **J** to jump two tiles in the current facing direction.
///
/// The intermediate tile is vaulted over regardless of its type.
/// Landing is allowed on any walkable tile or [`TileType::Void`]; solid walls
/// and closed doors cannot be landed on.  The jump can be performed over any
/// tile (including floors, props, and void gaps) as long as the landing is valid.
fn trigger_jump_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    dungeon: Res<Dungeon>,
    doors: DoorParams<'_, '_>,
    mut player_q: Query<(&MapPosition, &mut PlayerAnimation), With<Player>>,
    mut log: EventWriter<GameMessage>,
) {
    if !keyboard.just_pressed(KeyCode::KeyJ) {
        return;
    }
    let Ok((pos, mut anim)) = player_q.get_single_mut() else { return; };

    // No chained jumps.
    if anim.jumping {
        return;
    }

    let (dx, dy) = facing_to_grid_delta(anim.facing);
    let land_x = pos.x + dx * 2;
    let land_y = pos.y + dy * 2;

    let map = dungeon.current_map();
    if !map.in_bounds(land_x, land_y) {
        log.send(GameMessage::new("Nothing to jump to."));
        return;
    }

    let land_tile = map.tiles[map.idx(land_x, land_y)];

    // Void is always a valid (if dangerous) landing.
    // Walkable + no prop + no closed door → normal landing.
    // Everything else (solid walls, closed doors) → blocked.
    let can_land = land_tile == TileType::Void
        || (map.is_passable(land_x, land_y) && !doors.is_closed_at((land_x, land_y)));

    if !can_land {
        log.send(GameMessage::new("Can't jump there."));
        return;
    }

    // The player walks back one tile before running up and jumping.
    let back_x = pos.x - dx;
    let back_y = pos.y - dy;
    if !map.in_bounds(back_x, back_y)
        || !map.is_passable(back_x, back_y)
        || doors.is_closed_at((back_x, back_y))
    {
        log.send(GameMessage::alert("Not enough room to jump."));
        return;
    }

    anim.trigger_jump((pos.x, pos.y), (back_x, back_y), (land_x, land_y));
}

// ── Update system: apply jump arc to Transform ────────────────────────────────

/// Applies per-phase transform interpolation for the three-phase jump.
///
/// - **WalkBack**: linearly interpolates from `jump_origin` to `jump_back_tile`.
/// - **RunForward**: linearly interpolates from `jump_back_tile` to `jump_origin`.
/// - **Arc**: parabolic arc from `jump_origin` to `jump_target` (two tiles ahead).
///
/// Runs after [`animate_player`] so `jump_frame` is already updated.
fn apply_jump_arc(
    mut player_q: Query<(&mut MapPosition, &mut Transform, &mut PlayerAnimation), With<Player>>,
) {
    let Ok((mut pos, mut transform, mut anim)) = player_q.get_single_mut() else { return; };

    if !anim.jumping {
        return;
    }

    let origin_world = MapPosition::new(anim.jump_origin.0, anim.jump_origin.1).to_world(0.0);
    let back_world   = MapPosition::new(anim.jump_back_tile.0, anim.jump_back_tile.1).to_world(0.0);
    let target_world = MapPosition::new(anim.jump_target.0, anim.jump_target.1).to_world(0.0);

    match anim.jump_phase {
        JumpPhase::WalkBack => {
            let max_frame = (FEMALE_WALKBACK_FRAME_COUNT - 1).max(1) as f32;
            let t = anim.jump_frame as f32 / max_frame;
            let wx = origin_world.x + (back_world.x - origin_world.x) * t;
            let wy = origin_world.y + (back_world.y - origin_world.y) * t;
            transform.translation.x = wx;
            transform.translation.y = wy;

            // Snap MapPosition to back tile at the end of the walk-back.
            if anim.jump_frame >= FEMALE_WALKBACK_FRAME_COUNT / 2 {
                pos.x = anim.jump_back_tile.0;
                pos.y = anim.jump_back_tile.1;
            }
        }

        JumpPhase::RunForward => {
            let max_frame = (FEMALE_RUN_FRAME_COUNT - 1).max(1) as f32;
            let t = anim.jump_frame as f32 / max_frame;
            let wx = back_world.x + (origin_world.x - back_world.x) * t;
            let wy = back_world.y + (origin_world.y - back_world.y) * t;
            transform.translation.x = wx;
            transform.translation.y = wy;

            // Snap MapPosition to origin at the end of the run-forward.
            if anim.jump_frame >= FEMALE_RUN_FRAME_COUNT / 2 {
                pos.x = anim.jump_origin.0;
                pos.y = anim.jump_origin.1;
            }
        }

        JumpPhase::Arc => {
            // Snap MapPosition to the landing tile at the arc midpoint.
            if !anim.jump_midpoint_reached
                && anim.jump_frame >= FEMALE_JUMP_FRAME_COUNT / 2
            {
                anim.jump_midpoint_reached = true;
                pos.x = anim.jump_target.0;
                pos.y = anim.jump_target.1;
            }

            let max_frame = (FEMALE_JUMP_FRAME_COUNT - 1).max(1) as f32;
            let t = anim.jump_frame as f32 / max_frame;
            // Linear horizontal/vertical interpolation from origin to target.
            let wx = origin_world.x + (target_world.x - origin_world.x) * t;
            let wy = origin_world.y + (target_world.y - origin_world.y) * t;
            // Parabolic vertical arc on top.
            let arc = JUMP_ARC_HEIGHT * 4.0 * t * (1.0 - t);
            transform.translation.x = wx;
            transform.translation.y = wy + arc;
        }
    }
}

// ── Update system: handle jump landing ───────────────────────────────────────

/// Consumes [`JumpLandedEvent`] and applies the [`VoidOutcome`] if the player
/// has landed on a [`TileType::Void`] cell.
fn on_jump_land(
    mut events: EventReader<JumpLandedEvent>,
    dungeon: Res<Dungeon>,
    mut player_q: Query<(&mut MapPosition, &mut Transform, &mut PlayerAnimation), With<Player>>,
    mut log: EventWriter<GameMessage>,
    mut level_transition: EventWriter<LevelTransition>,
    mut app_exit: EventWriter<AppExit>,
) {
    for _ in events.read() {
        let Ok((mut pos, mut transform, mut anim)) = player_q.get_single_mut() else { continue };

        let map = dungeon.current_map();
        if !map.in_bounds(pos.x, pos.y) {
            continue;
        }
        if map.tiles[map.idx(pos.x, pos.y)] != TileType::Void {
            continue;
        }

        let outcome = map
            .void_outcomes
            .get(&(pos.x, pos.y))
            .cloned()
            .unwrap_or(VoidOutcome::Hazard { damage: 10 });

        match outcome {
            VoidOutcome::NextFloor => {
                let next = dungeon.current_floor + 1;
                if next < dungeon.floors.len() {
                    let landing = dungeon.floors[next].rooms[0].center();
                    log.send(GameMessage::alert("You fall through to the next level!"));
                    level_transition.send(LevelTransition {
                        destination_floor: next,
                        exit_pos: landing,
                    });
                } else {
                    log.send(GameMessage::alert("You fall into the abyss…"));
                    app_exit.send(AppExit::Success);
                }
            }
            VoidOutcome::Hazard { damage } => {
                let msg = format!("You fall into a pit! (-{damage} HP)");
                log.send(GameMessage::alert(msg));
                // Respawn at the first room centre of the current floor.
                let (rx, ry) = map.rooms[0].center();
                pos.x = rx;
                pos.y = ry;
                let world = pos.to_world(0.0);
                transform.translation.x = world.x;
                transform.translation.y = world.y;
                anim.facing = FacingDir::South;
            }
            VoidOutcome::Warp { floor, pos: (wx, wy) } => {
                log.send(GameMessage::alert("You fall through a dimensional rift!"));
                level_transition.send(LevelTransition {
                    destination_floor: floor,
                    exit_pos: (wx, wy),
                });
            }
            VoidOutcome::Terminal => {
                log.send(GameMessage::alert(
                    "You fall into the abyss… and are lost forever.",
                ));
                app_exit.send(AppExit::Success);
            }
        }
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
type CullTileItem = (
    &'static Transform,
    &'static mut Visibility,
    Option<&'static StairsUpTile>,
    Option<&'static StairsMidTile>,
);

fn cull_map_tiles(
    player_q: Query<(&Transform, &PointLight2d, &LightType), With<Player>>,
    beam_q: Query<(&Transform, &PointLight2d), With<LanternBeamLight>>,
    mut tile_q: Query<CullTileItem, With<MapTile>>,
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

// ── Tile hover highlight ──────────────────────────────────────────────────────

/// Colour for a walkable tile outline (green, semi-transparent).
const HIGHLIGHT_WALKABLE: Color = Color::srgba(0.0, 0.85, 0.0, 0.7);
/// Colour for a non-walkable tile outline (red, semi-transparent).
const HIGHLIGHT_BLOCKED: Color = Color::srgba(0.85, 0.0, 0.0, 0.7);

/// Draws a thin isometric diamond outline around the tile under the cursor.
fn draw_tile_highlight(
    cursor: WorldCursor<'_, '_>,
    dungeon: Res<Dungeon>,
    doors: DoorParams<'_, '_>,
    mut gizmos: Gizmos,
) {
    let Some(world_pos) = cursor.world_pos() else { return };

    // Convert cursor world position to grid coordinates.
    let sum  = -world_pos.y / ISO_STEP_Y;
    let diff =  world_pos.x / ISO_STEP_X;
    let gx = ((diff + sum) / 2.0).round() as i32;
    let gy = ((sum  - diff) / 2.0).round() as i32;

    let map = dungeon.current_map();

    // Choose colour based on walkability.
    let passable = map.is_passable(gx, gy) && !doors.is_closed_at((gx, gy));
    let color = if passable { HIGHLIGHT_WALKABLE } else { HIGHLIGHT_BLOCKED };

    // Tile-centre in world space.
    let cx = (gx as f32 - gy as f32) * ISO_STEP_X;
    let cy = -(gx as f32 + gy as f32) * ISO_STEP_Y;

    // Four vertices of the isometric diamond.
    let top    = Vec2::new(cx,              cy + ISO_STEP_Y);
    let right  = Vec2::new(cx + ISO_STEP_X, cy);
    let bottom = Vec2::new(cx,              cy - ISO_STEP_Y);
    let left   = Vec2::new(cx - ISO_STEP_X, cy);

    // Draw the diamond at a high Z so it renders on top of floor tiles.
    let z = 100.0;
    let to3 = |v: Vec2| Vec3::new(v.x, v.y, z);

    gizmos.line(to3(top),    to3(right),  color);
    gizmos.line(to3(right),  to3(bottom), color);
    gizmos.line(to3(bottom), to3(left),   color);
    gizmos.line(to3(left),   to3(top),    color);
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<PlayerMoving>()
            .init_resource::<ClickState>()
            .add_event::<LevelTransition>()
            .add_event::<JumpLandedEvent>()
            .add_systems(Startup, spawn_player)
            .add_systems(
                Update,
                (
                    player_movement,
                    handle_mouse_click,
                    trigger_jump_system.after(player_movement),
                    auto_step.after(player_movement).after(handle_mouse_click),
                    animate_player.after(trigger_jump_system).after(auto_step),
                    apply_jump_arc.after(animate_player),
                    on_jump_land.after(apply_jump_arc),
                    interact_with_stairs,
                    execute_level_transition.after(interact_with_stairs),
                    interact_with_chests.after(interact_with_stairs),
                    interact_with_doors.after(interact_with_stairs),
                    apply_item_use,
                    toggle_light_type,
                    flicker_torch.after(toggle_light_type),
                    apply_light_type.after(toggle_light_type),
                    cull_map_tiles.after(apply_light_type),
                    draw_tile_highlight,
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

    // ── Jump mechanic ─────────────────────────────────────────────────────────

    #[test]
    fn facing_to_grid_delta_cardinals() {
        assert_eq!(facing_to_grid_delta(FacingDir::North), ( 0, -1));
        assert_eq!(facing_to_grid_delta(FacingDir::East),  ( 1,  0));
        assert_eq!(facing_to_grid_delta(FacingDir::South), ( 0,  1));
        assert_eq!(facing_to_grid_delta(FacingDir::West),  (-1,  0));
    }

    #[test]
    fn facing_to_grid_delta_diagonals() {
        assert_eq!(facing_to_grid_delta(FacingDir::NorthEast), ( 1, -1));
        assert_eq!(facing_to_grid_delta(FacingDir::SouthEast), ( 1,  1));
        assert_eq!(facing_to_grid_delta(FacingDir::SouthWest), (-1,  1));
        assert_eq!(facing_to_grid_delta(FacingDir::NorthWest), (-1, -1));
    }

    #[test]
    fn jump_arc_peaks_at_midpoint() {
        // The parabola 4·t·(1-t) peaks at t = 0.5 with value 1.0.
        let mid_t = 0.5_f32;
        let arc = JUMP_ARC_HEIGHT * 4.0 * mid_t * (1.0 - mid_t);
        assert!((arc - JUMP_ARC_HEIGHT).abs() < 1e-5, "arc should peak at JUMP_ARC_HEIGHT");
    }

    #[test]
    fn jump_arc_zero_at_endpoints() {
        for t in [0.0_f32, 1.0_f32] {
            let arc = JUMP_ARC_HEIGHT * 4.0 * t * (1.0 - t);
            assert!(arc.abs() < 1e-5, "arc at t={t} should be 0, got {arc}");
        }
    }

    #[test]
    fn jump_arc_height_is_positive() {
        assert!(JUMP_ARC_HEIGHT > 0.0);
    }

    #[test]
    fn jump_frame_count_matches_walk_layout() {
        // Jump uses the same 6×4 spritesheet grid as Walk.
        assert_eq!(FEMALE_JUMP_FRAME_COUNT, FEMALE_WALK_FRAME_COUNT);
    }

    #[test]
    fn player_animation_starts_not_jumping() {
        let anim = PlayerAnimation::new();
        assert!(!anim.jumping);
    }

    #[test]
    fn trigger_jump_sets_jumping_state() {
        let mut anim = PlayerAnimation::new();
        anim.trigger_jump((3, 4), (2, 4), (5, 4));
        assert!(anim.jumping);
        assert_eq!(anim.jump_phase, JumpPhase::WalkBack);
        assert_eq!(anim.jump_origin, (3, 4));
        assert_eq!(anim.jump_back_tile, (2, 4));
        assert_eq!(anim.jump_target, (5, 4));
        assert!(!anim.jump_midpoint_reached);
        assert_eq!(anim.jump_frame, 0);
        assert!(anim.path.is_empty(), "trigger_jump must clear auto-travel path");
        assert!(!anim.running, "trigger_jump must clear running state");
    }

    #[test]
    fn advance_jump_phase_cycles_correctly() {
        let mut anim = PlayerAnimation::new();
        anim.trigger_jump((3, 4), (2, 4), (5, 4));
        assert_eq!(anim.jump_phase, JumpPhase::WalkBack);

        assert!(!anim.advance_jump_phase());
        assert_eq!(anim.jump_phase, JumpPhase::RunForward);
        assert_eq!(anim.jump_frame, 0);

        assert!(!anim.advance_jump_phase());
        assert_eq!(anim.jump_phase, JumpPhase::Arc);
        assert_eq!(anim.jump_frame, 0);

        assert!(anim.advance_jump_phase(), "Arc should signal completion");
    }

    #[test]
    fn walkback_frame_count_matches_walk_layout() {
        // WalkBack uses the same 6×4 spritesheet grid as Walk.
        assert_eq!(FEMALE_WALKBACK_FRAME_COUNT, FEMALE_WALK_FRAME_COUNT);
    }

    /// Single-click facing uses `dir_to_facing` with the grid delta from the
    /// player to the clicked tile.  Large deltas must resolve the same way as
    /// unit deltas (signum collapses the magnitude).
    #[test]
    fn dir_to_facing_large_deltas() {
        // Clicking far to the south-east (dx=+5, dy=+3) should face SouthEast.
        assert_eq!(dir_to_facing(5, 3) as usize, FacingDir::SouthEast as usize);
        // Pure cardinal at distance: dx=0, dy=-10 → North.
        assert_eq!(dir_to_facing(0, -10) as usize, FacingDir::North as usize);
        // Diagonal at distance: dx=-7, dy=-7 → NorthWest.
        assert_eq!(dir_to_facing(-7, -7) as usize, FacingDir::NorthWest as usize);
    }

    /// Clicking the player's own tile (dx=0, dy=0) must not change facing.
    #[test]
    fn dir_to_facing_zero_delta_is_south() {
        // dir_to_facing(0,0) falls through to the default arm (South).
        assert_eq!(dir_to_facing(0, 0) as usize, FacingDir::South as usize);
    }

    #[test]
    fn trigger_walk_resets_frame_on_run_to_walk_transition() {
        let mut anim = PlayerAnimation::new();
        // Simulate being in run state with a frame beyond walk count.
        anim.running = true;
        anim.auto_running = true;
        anim.frame = FEMALE_RUN_FRAME_COUNT - 1;
        anim.trigger_walk(FacingDir::North);
        assert_eq!(anim.frame, 0, "frame must reset when switching from run to walk");
        assert!(!anim.auto_running);
    }

    #[test]
    fn trigger_run_resets_frame_on_walk_to_run_transition() {
        let mut anim = PlayerAnimation::new();
        anim.running = true;
        anim.auto_running = false;
        anim.frame = FEMALE_WALK_FRAME_COUNT - 1;
        anim.trigger_run(FacingDir::East);
        assert_eq!(anim.frame, 0, "frame must reset when switching from walk to run");
        assert!(anim.auto_running);
    }

    #[test]
    fn trigger_walk_preserves_frame_when_already_walking() {
        let mut anim = PlayerAnimation::new();
        anim.running = true;
        anim.auto_running = false;
        anim.frame = 10;
        anim.trigger_walk(FacingDir::South);
        assert_eq!(anim.frame, 10, "frame should be preserved when walk-to-walk");
    }

    #[test]
    fn run_cooldown_matches_step_duration() {
        let mut anim = PlayerAnimation::new();
        anim.trigger_walk(FacingDir::North);
        assert!(
            (anim.run_cooldown.duration().as_secs_f32() - AUTO_WALK_STEP_SECS).abs() < 1e-5,
            "walk cooldown should match walk step duration"
        );
        anim.trigger_run(FacingDir::North);
        assert!(
            (anim.run_cooldown.duration().as_secs_f32() - AUTO_RUN_STEP_SECS).abs() < 1e-5,
            "run cooldown should match run step duration"
        );
    }
}
