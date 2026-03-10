use std::collections::HashMap;

use bevy::{prelude::*, sprite::Anchor};
use rand::Rng;

use crate::{
    components::{CardinalDir, ChestContents, Door, ItemKind, MapPosition, MapTile, Player, PropTile, StairsMidTile, StairsUpTile, WallTile, YSort, YSortBias},
    ISO_STEP_X, ISO_STEP_Y, MAP_HEIGHT, MAP_WIDTH, TILE_SCALE,
};

// ── Dungeon generation tunables ───────────────────────────────────────────────

const MAX_ROOMS: usize = 12;
const MIN_ROOM_SIZE: i32 = 3;
const MAX_ROOM_SIZE: i32 = 8;
const NUM_DUNGEON_FLOORS: usize = 3;

// ── Tile type ─────────────────────────────────────────────────────────────────

/// The base type of a map cell.  Determines rendering and tile-level walkability
/// (see [`Map::is_walkable`]).  Gameplay-level passability also accounts for
/// props — see [`Map::is_passable`].
// Variants beyond Wall/Floor are defined for fixed room designs and are not
// yet constructed by the procedural generator.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum TileType {
    // ── Impassable walls ──────────────────────────────────────────────────────
    /// Standard stone wall — rendered with corner pieces where two runs meet.
    Wall,
    /// Closed door — impassable; uses `stoneWallDoorClosed_*` sprites.
    DoorClosed,
    /// Damaged/collapsed wall section; uses `stoneWallBroken_*` sprites.
    BrokenWall,
    /// Wall with a window opening; uses `stoneWallWindow_*` sprites.
    Window,

    // ── Walkable floors ───────────────────────────────────────────────────────
    /// Standard stone floor — four random variants (`stone_N/E/S/W`).
    Floor,
    /// Dirt/earth floor — cave tunnels and earthy areas.
    Dirt,
    /// Wooden plank floor — rooms, platforms.
    Planks,
    /// Bridge section spanning a gap.
    Bridge,
    /// Spiral staircase descending to the next (deeper) floor.
    /// Press **E** while standing on it to descend.
    StairsDown,
    /// Intermediate stair tile connecting the floor above and the floor below.
    /// Press **W / ↑** to ascend or **S / ↓** to descend while standing on it.
    StairsMid,
    /// Staircase ascending to the previous (shallower) floor.
    /// Press **E** while standing on it to ascend.
    StairsUp,

    // ── Walkable wall openings ────────────────────────────────────────────────
    /// Open doorway — walkable; uses `stoneWallDoorOpen_*` sprites.
    /// Not marked as `WallTile` so it is never faded by the occlusion system.
    DoorOpen,
    /// Wide archway — walkable; uses `stoneWallArchway_*` sprites.
    Archway,
}

impl TileType {
    /// Returns `true` for any tile the player can stand on (ignores props).
    /// Used for wall-face selection, beam casting, and map generation.
    pub fn is_walkable(self) -> bool {
        matches!(
            self,
            Self::Floor
                | Self::Dirt
                | Self::Planks
                | Self::Bridge
                | Self::StairsDown
                | Self::StairsMid
                | Self::StairsUp
                | Self::DoorOpen
                | Self::Archway
        )
    }

    /// Returns `true` if this tile renders as a floor sprite (random variant).
    pub fn is_floor_like(self) -> bool {
        matches!(
            self,
            Self::Floor
                | Self::Dirt
                | Self::Planks
                | Self::Bridge
                | Self::StairsDown
                | Self::StairsMid
                | Self::StairsUp
        )
    }

    /// Asset prefix for floor-like tiles (e.g. `"stone"` → `stone_N.png`).
    /// Panics if called on a non-floor tile.
    pub fn floor_asset_prefix(self) -> &'static str {
        match self {
            Self::Floor => "stone",
            Self::Dirt => "dirt",
            Self::Planks => "planks",
            Self::Bridge => "bridge",
            Self::StairsDown | Self::StairsMid | Self::StairsUp => "stairsSpiral",
            _ => panic!("floor_asset_prefix called on non-floor TileType"),
        }
    }

    /// Asset prefix for wall-like tiles (e.g. `"stoneWallDoorOpen"` → `stoneWallDoorOpen_N.png`).
    /// Panics if called on a floor tile.
    pub fn wall_asset_prefix(self) -> &'static str {
        match self {
            Self::Wall => "stoneWall",
            Self::DoorClosed => "stoneWallDoorClosed",
            Self::DoorOpen => "stoneWallDoorOpen",
            Self::BrokenWall => "stoneWallBroken",
            Self::Window => "stoneWallWindow",
            Self::Archway => "stoneWallArchway",
            _ => panic!("wall_asset_prefix called on non-wall TileType"),
        }
    }

    /// Returns `true` if this tile should carry the `WallTile` marker so the
    /// occlusion system can fade it.  Walkable openings are excluded — the
    /// player should always be visible through an open door or archway.
    pub fn is_occluding_wall(self) -> bool {
        matches!(
            self,
            Self::Wall | Self::DoorClosed | Self::BrokenWall | Self::Window
        )
    }

    /// Returns `true` for any stair tile variant.  Used to give stair
    /// interaction priority over door interaction on the **E** key.
    pub fn is_stair(self) -> bool {
        matches!(self, Self::StairsDown | Self::StairsMid | Self::StairsUp)
    }
}

// ── Prop type ─────────────────────────────────────────────────────────────────

/// An object placed on top of a floor tile.  Props block player movement
/// (see [`Map::is_passable`]) but do not block the lantern beam.
/// Each variant has four directional sprites (`_N`, `_E`, `_S`, `_W`).
// Variants are defined for fixed room designs and not yet placed procedurally.
#[allow(dead_code)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum PropType {
    Barrel,
    Barrels,
    BarrelsStacked,
    ChestClosed,
    ChestOpen,
    WoodenCrate,
    WoodenCrates,
    WoodenPile,
    /// Stone column / pillar (`stoneColumn_*`).
    Pillar,
    /// Stone column with wooden cap (`stoneColumnWood_*`).
    PillarWood,
    TableRound,
    TableRoundChairs,
    TableShort,
    TableShortChairs,
    Chair,
    WoodenSupports,
    WoodenSupportsBeam,
}

impl PropType {
    /// Returns the asset filename prefix (without `_N.png` etc.).
    pub fn asset_prefix(self) -> &'static str {
        match self {
            Self::Barrel => "barrel",
            Self::Barrels => "barrels",
            Self::BarrelsStacked => "barrelsStacked",
            Self::ChestClosed => "chestClosed",
            Self::ChestOpen => "chestOpen",
            Self::WoodenCrate => "woodenCrate",
            Self::WoodenCrates => "woodenCrates",
            Self::WoodenPile => "woodenPile",
            Self::Pillar => "stoneColumn",
            Self::PillarWood => "stoneColumnWood",
            Self::TableRound => "tableRound",
            Self::TableRoundChairs => "tableRoundChairs",
            Self::TableShort => "tableShort",
            Self::TableShortChairs => "tableShortChairs",
            Self::Chair => "chair",
            Self::WoodenSupports => "woodenSupports",
            Self::WoodenSupportsBeam => "woodenSupportsBeam",
        }
    }
}

// ── Stair link ────────────────────────────────────────────────────────────────

/// One directional leg of a staircase — the floor and tile position the player
/// lands on after travelling in that direction.
pub struct StairLink {
    pub target_floor: usize,
    pub target_pos: (i32, i32),
}

/// The full traversal options for a single stair tile.
///
/// - `StairsDown` tiles have `down = Some(…)` and `up = None`.
/// - `StairsUp`   tiles have `up   = Some(…)` and `down = None`.
/// - `StairsMid`  tiles have both `Some(…)`.
pub struct StairNode {
    pub up:   Option<StairLink>,
    pub down: Option<StairLink>,
}

// ── Door placement descriptor ─────────────────────────────────────────────────

/// Describes a door to be spawned at startup.  Stored in [`Map::doors`] so
/// that `spawn_floor_doors` can create the entities and populate [`DoorRegistry`].
///
/// The tile at `(x, y)` must already be `TileType::Floor` in [`Map::tiles`]
/// (set by the generator) so that passability is correct when the door is open.
pub struct DoorPlacement {
    pub x: i32,
    pub y: i32,
    pub open: bool,
    /// Sprite face direction — matches the asset suffix convention (`_N` etc.)
    /// and is determined by which adjacent tile holds the room interior.
    pub facing: CardinalDir,
}

// ── Door registry ─────────────────────────────────────────────────────────────

/// Maps grid positions to their door entity.  Used by movement and
/// pathfinding systems to check whether a door at a given cell is closed.
#[derive(Resource, Default)]
pub struct DoorRegistry(pub HashMap<(i32, i32), Entity>);

// ── Axis-aligned rectangle (used for rooms) ───────────────────────────────────

#[derive(Clone, Copy)]
pub struct Rect {
    pub x1: i32,
    pub y1: i32,
    pub x2: i32,
    pub y2: i32,
}

impl Rect {
    pub fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self {
            x1: x,
            y1: y,
            x2: x + w,
            y2: y + h,
        }
    }

    pub fn center(&self) -> (i32, i32) {
        ((self.x1 + self.x2) / 2, (self.y1 + self.y2) / 2)
    }

    pub fn intersects(&self, other: &Rect) -> bool {
        self.x1 <= other.x2
            && self.x2 >= other.x1
            && self.y1 <= other.y2
            && self.y2 >= other.y1
    }
}

// ── Map resource ──────────────────────────────────────────────────────────────

#[derive(Resource)]
pub struct Map {
    pub width: i32,
    pub height: i32,
    /// One entry per cell; indexed by `idx(x, y)`.
    pub tiles: Vec<TileType>,
    /// Optional prop in each cell; same indexing as `tiles`.
    /// Props sit on top of floor tiles and block player movement.
    pub props: Vec<Option<PropType>>,
    /// Doors to be spawned at startup.  Each entry's tile is already `Floor`.
    pub doors: Vec<DoorPlacement>,
    pub rooms: Vec<Rect>,
    /// Maps stair tile positions on this floor to their traversal options.
    pub stair_links: HashMap<(i32, i32), StairNode>,
    /// Items stored inside closed chests, keyed by grid position.
    /// Tuple: (item kind, sprite facing direction for the open-chest asset).
    pub chest_items: HashMap<(i32, i32), (ItemKind, CardinalDir)>,
}

impl Map {
    fn new() -> Self {
        let size = (MAP_WIDTH * MAP_HEIGHT) as usize;
        Self {
            width: MAP_WIDTH,
            height: MAP_HEIGHT,
            tiles: vec![TileType::Wall; size],
            props: vec![None; size],
            doors: Vec::new(),
            rooms: Vec::new(),
            stair_links: HashMap::new(),
            chest_items: HashMap::new(),
        }
    }

    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.width + x) as usize
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height
    }

    /// Tile-level walkability check — ignores props and door entities.
    /// Used by wall-face selection, lantern beam casting, and map generation.
    pub fn is_walkable(&self, x: i32, y: i32) -> bool {
        self.in_bounds(x, y) && self.tiles[self.idx(x, y)].is_walkable()
    }

    /// Gameplay passability — `false` if the tile is impassable *or* a prop
    /// occupies the cell.  Does **not** check door entities (closed doors are
    /// handled separately in movement systems via [`DoorRegistry`]).
    pub fn is_passable(&self, x: i32, y: i32) -> bool {
        self.is_walkable(x, y) && self.props[self.idx(x, y)].is_none()
    }

    fn carve_room(&mut self, room: &Rect) {
        for y in (room.y1 + 1)..room.y2 {
            for x in (room.x1 + 1)..room.x2 {
                let idx = self.idx(x, y);
                self.tiles[idx] = TileType::Floor;
            }
        }
    }

    fn carve_h_corridor(&mut self, x1: i32, x2: i32, y: i32) {
        for x in x1.min(x2)..=x1.max(x2) {
            for dy in -1..=1_i32 {
                if self.in_bounds(x, y + dy) {
                    let idx = self.idx(x, y + dy);
                    self.tiles[idx] = TileType::Floor;
                }
            }
        }
    }

    fn carve_v_corridor(&mut self, y1: i32, y2: i32, x: i32) {
        for y in y1.min(y2)..=y1.max(y2) {
            for dx in -1..=1_i32 {
                if self.in_bounds(x + dx, y) {
                    let idx = self.idx(x + dx, y);
                    self.tiles[idx] = TileType::Floor;
                }
            }
        }
    }
}

// ── Dungeon resource ──────────────────────────────────────────────────────────

/// Holds all floors of the dungeon and tracks which floor the player is on.
/// Floor 0 is the shallowest; higher indices are deeper underground.
#[derive(Resource)]
pub struct Dungeon {
    pub floors: Vec<Map>,
    pub current_floor: usize,
}

impl Dungeon {
    /// Returns a reference to the currently active floor map.
    pub fn current_map(&self) -> &Map {
        &self.floors[self.current_floor]
    }
}

// ── Dungeon generator ─────────────────────────────────────────────────────────

pub fn generate_map() -> Map {
    let mut map = Map::new();
    let mut rng = rand::thread_rng();

    for _ in 0..MAX_ROOMS {
        let w = rng.gen_range(MIN_ROOM_SIZE..=MAX_ROOM_SIZE);
        let h = rng.gen_range(MIN_ROOM_SIZE..=MAX_ROOM_SIZE);
        let x = rng.gen_range(1..MAP_WIDTH - w - 1);
        let y = rng.gen_range(1..MAP_HEIGHT - h - 1);

        let new_room = Rect::new(x, y, w, h);

        if map.rooms.iter().any(|r| r.intersects(&new_room)) {
            continue;
        }

        map.carve_room(&new_room);

        if let Some(prev) = map.rooms.last() {
            let (px, py) = prev.center();
            let (nx, ny) = new_room.center();

            if rng.gen_bool(0.5) {
                map.carve_h_corridor(px, nx, py);
                map.carve_v_corridor(py, ny, nx);
            } else {
                map.carve_v_corridor(py, ny, px);
                map.carve_h_corridor(px, nx, ny);
            }
        }

        map.rooms.push(new_room);
    }

    place_test_door(&mut map);
    place_test_chest(&mut map);

    map
}

/// Generates a multi-floor dungeon linked by a single staircase shaft.
///
/// Each floor's stair tile sits at its last room's centre, forming a chain:
///
/// ```text
/// floor 0  StairsDown  ──down──▶  floor 1  StairsMid  ──down──▶  floor 2  StairsUp
///                                            ◀──up──                  ◀──up──
/// ```
///
/// - **`StairsDown`** (top floor): press **E** to descend.
/// - **`StairsMid`** (intermediate): press **W / ↑** to ascend, **S / ↓** to descend.
/// - **`StairsUp`** (bottom floor): press **E** to ascend.
pub fn generate_dungeon(num_floors: usize) -> Dungeon {
    assert!(num_floors >= 1, "dungeon must have at least one floor");

    let mut floors: Vec<Map> = (0..num_floors).map(|_| generate_map()).collect();

    // Collect each floor's stair position (last room centre) up front so we
    // can reference adjacent floors while building links.
    let positions: Vec<(i32, i32)> = floors
        .iter()
        .map(|m| m.rooms.last().expect("floor must have at least one room").center())
        .collect();

    for f in 0..num_floors {
        let (sx, sy) = positions[f];
        let si = floors[f].idx(sx, sy);

        let is_top    = f == 0;
        let is_bottom = f == num_floors - 1;

        floors[f].tiles[si] = match (is_top, is_bottom) {
            (true,  true)  => continue,           // single-floor: no stairs
            (true,  false) => TileType::StairsDown,
            (false, true)  => TileType::StairsUp,
            (false, false) => TileType::StairsMid,
        };

        floors[f].stair_links.insert(
            (sx, sy),
            StairNode {
                up:   (!is_top   ).then(|| StairLink { target_floor: f - 1, target_pos: positions[f - 1] }),
                down: (!is_bottom).then(|| StairLink { target_floor: f + 1, target_pos: positions[f + 1] }),
            },
        );
    }

    Dungeon { floors, current_floor: 0 }
}

/// Places a closed door on the south wall of the first room and carves one
/// floor tile outside it, guaranteeing the player can approach from both sides.
///
/// The door tile (`room.y2`, center x) is converted from Wall to Floor so that
/// `Map::is_walkable` returns `true` when the door entity is open.  The door
/// entity provides movement blocking when closed.
fn place_test_door(map: &mut Map) {
    let Some(room) = map.rooms.first().copied() else {
        return;
    };

    let cx = (room.x1 + room.x2) / 2;
    let door_y = room.y2; // south boundary wall row

    if !map.in_bounds(cx, door_y) {
        return;
    }

    // Determine facing from neighbors *before* modifying the tile, so the
    // adjacency check reflects the original carved interior.
    let facing = if map.is_walkable(cx, door_y - 1) {
        CardinalDir::S
    } else if map.is_walkable(cx, door_y + 1) {
        CardinalDir::N
    } else if map.is_walkable(cx + 1, door_y) {
        CardinalDir::W
    } else {
        CardinalDir::E
    };

    // Convert the wall tile to floor so the door position is walkable when open.
    let door_idx = map.idx(cx, door_y);
    map.tiles[door_idx] = TileType::Floor;

    // Carve one tile to the south so the player can reach the door from outside.
    if map.in_bounds(cx, door_y + 1) {
        let outside_idx = map.idx(cx, door_y + 1);
        map.tiles[outside_idx] = TileType::Floor;
    }

    map.doors.push(DoorPlacement {
        x: cx,
        y: door_y,
        open: false,
        facing,
    });
}

/// Places a closed chest containing a key one tile east of the first room's
/// centre.  The chest blocks movement (it sits in `Map::props`) and its
/// contents are recorded in `Map::chest_items` so `spawn_floor_tiles` can
/// attach the [`ChestContents`] component to the spawned entity.
fn place_test_chest(map: &mut Map) {
    let Some(room) = map.rooms.first().copied() else { return };
    let cx = (room.x1 + room.x2) / 2;
    let cy = (room.y1 + room.y2) / 2;
    let chest_x = cx + 1;
    let chest_y = cy;

    if !map.in_bounds(chest_x, chest_y) { return }
    if !map.is_walkable(chest_x, chest_y) { return }

    let idx = map.idx(chest_x, chest_y);
    map.props[idx] = Some(PropType::ChestClosed);
    map.chest_items.insert((chest_x, chest_y), (ItemKind::Key, CardinalDir::N));
}

// ── Tile/door spawning (pub: called from startup and level-transition) ─────────

/// Spawns isometric tile sprites for all cells in `map`.  Can be called from
/// both startup and level-transition systems.
pub fn spawn_floor_tiles(commands: &mut Commands, map: &Map, asset_server: &AssetServer) {
    const DIRS: [&str; 4] = ["N", "E", "S", "W"];
    let anchor = Anchor::Custom(Vec2::new(0.0, -0.30));
    let mut rng = rand::thread_rng();

    for y in 0..map.height {
        for x in 0..map.width {
            let wx = (x as f32 - y as f32) * ISO_STEP_X;
            let wy = -(x as f32 + y as f32) * ISO_STEP_Y;
            let floor_z = (x + y) as f32 * 0.001 - 200.0;

            let tile = map.tiles[map.idx(x, y)];

            if tile.is_floor_like() {
                // ── Floor ─────────────────────────────────────────────────────
                let dir = DIRS[rng.gen_range(0..4)];
                let image = asset_server
                    .load(format!("Isometric/{}_{dir}.png", tile.floor_asset_prefix()));
                let mut entity = commands.spawn((
                    MapTile,
                    Sprite { image, anchor, ..Default::default() },
                    Transform::from_xyz(wx, wy, floor_z).with_scale(Vec3::splat(TILE_SCALE)),
                ));
                // Stair tiles whose shaft opens from above are always visible —
                // the opening provides ambient light regardless of torch radius.
                match tile {
                    TileType::StairsUp  => { entity.insert(StairsUpTile); }
                    TileType::StairsMid => { entity.insert(StairsMidTile); }
                    _ => {}
                }
            } else {
                // ── Wall-like ─────────────────────────────────────────────────
                let s = map.is_walkable(x, y + 1);
                let n = map.is_walkable(x, y - 1);
                let e = map.is_walkable(x + 1, y);
                let w = map.is_walkable(x - 1, y);

                let image = if tile == TileType::Wall {
                    if s && e      { asset_server.load("Isometric/stoneWallCorner_N.png") }
                    else if s && w { asset_server.load("Isometric/stoneWallCorner_E.png") }
                    else if n && w { asset_server.load("Isometric/stoneWallCorner_S.png") }
                    else if n && e { asset_server.load("Isometric/stoneWallCorner_W.png") }
                    else if s      { asset_server.load("Isometric/stoneWall_N.png") }
                    else if n      { asset_server.load("Isometric/stoneWall_S.png") }
                    else if e      { asset_server.load("Isometric/stoneWall_W.png") }
                    else if w      { asset_server.load("Isometric/stoneWall_E.png") }
                    else           { continue; }
                } else {
                    let prefix = tile.wall_asset_prefix();
                    if s      { asset_server.load(format!("Isometric/{prefix}_N.png")) }
                    else if n { asset_server.load(format!("Isometric/{prefix}_S.png")) }
                    else if e { asset_server.load(format!("Isometric/{prefix}_W.png")) }
                    else if w { asset_server.load(format!("Isometric/{prefix}_E.png")) }
                    else      { continue; }
                };

                let mut entity = commands.spawn((
                    MapTile,
                    YSort,
                    Sprite { image, anchor, ..Default::default() },
                    Transform::from_xyz(wx, wy, 0.0).with_scale(Vec3::splat(TILE_SCALE)),
                ));
                if tile.is_occluding_wall() {
                    entity.insert(WallTile);
                }
            }

            // ── Prop ──────────────────────────────────────────────────────────
            if let Some(prop) = map.props[map.idx(x, y)] {
                // Chest props with recorded contents use a fixed facing so the
                // open-chest sprite can be loaded correctly; all others random.
                let chest = map.chest_items.get(&(x, y));
                let dir = chest
                    .map(|(_, d)| d.as_str())
                    .unwrap_or_else(|| DIRS[rng.gen_range(0..4)]);
                let image = asset_server
                    .load(format!("Isometric/{}_{dir}.png", prop.asset_prefix()));
                let mut entity = commands.spawn((
                    MapTile,
                    PropTile,
                    YSort,
                    MapPosition::new(x, y),
                    Sprite { image, anchor, ..Default::default() },
                    Transform::from_xyz(wx, wy, 0.0).with_scale(Vec3::splat(TILE_SCALE)),
                ));
                if let Some(&(item, facing)) = chest {
                    entity.insert(ChestContents { item, facing });
                }
            }
        }
    }
}

/// Spawns door entities for every [`DoorPlacement`] in `map` and registers
/// them in `registry`.  Can be called from both startup and level-transition systems.
pub fn spawn_floor_doors(
    commands: &mut Commands,
    map: &Map,
    asset_server: &AssetServer,
    registry: &mut DoorRegistry,
) {
    let anchor = Anchor::Custom(Vec2::new(0.0, -0.30));

    for placement in &map.doors {
        let wx = (placement.x as f32 - placement.y as f32) * ISO_STEP_X;
        let wy = -(placement.x as f32 + placement.y as f32) * ISO_STEP_Y;

        let state = if placement.open { "Open" } else { "Closed" };
        let dir = placement.facing.as_str();
        let image =
            asset_server.load(format!("Isometric/stoneWallDoor{state}_{dir}.png"));

        let entity = commands
            .spawn((
                MapTile,
                WallTile,
                Door { open: placement.open, facing: placement.facing },
                YSort,
                YSortBias(-0.001),
                Sprite { image, anchor, ..Default::default() },
                Transform::from_xyz(wx, wy, 0.0).with_scale(Vec3::splat(TILE_SCALE)),
            ))
            .id();

        registry.0.insert((placement.x, placement.y), entity);
    }
}

// ── Startup system wrappers ───────────────────────────────────────────────────

fn startup_spawn_tiles(
    mut commands: Commands,
    dungeon: Res<Dungeon>,
    asset_server: Res<AssetServer>,
) {
    spawn_floor_tiles(&mut commands, dungeon.current_map(), &asset_server);
}

fn startup_spawn_doors(
    mut commands: Commands,
    dungeon: Res<Dungeon>,
    asset_server: Res<AssetServer>,
    mut registry: ResMut<DoorRegistry>,
) {
    spawn_floor_doors(&mut commands, dungeon.current_map(), &asset_server, &mut registry);
}

// ── Update system: fade walls that occlude the player ────────────────────────

/// A wall at grid (x, y) occludes the player at (px, py) when:
///   1. x + y > px + py  — the wall is closer to the viewer (higher z after YSort)
///   2. |(x − y) − (px − py)| ≤ 1  — the wall shares the player's screen column
///
/// When occluding, the wall sprite alpha lerps down to OCCLUDED_ALPHA so the
/// player remains visible without the wall disappearing entirely.
fn fade_occluding_walls(
    time: Res<Time>,
    player_q: Query<&MapPosition, With<Player>>,
    mut wall_q: Query<(&Transform, &mut Sprite), With<WallTile>>,
) {
    const OCCLUDED_ALPHA: f32 = 0.25;
    const FADE_SPEED: f32 = 10.0;

    let Ok(player_pos) = player_q.get_single() else {
        return;
    };
    let player_depth = player_pos.x + player_pos.y;
    let player_col   = player_pos.x - player_pos.y;

    for (transform, mut sprite) in wall_q.iter_mut() {
        let wall_depth = (-transform.translation.y / ISO_STEP_Y).round() as i32;
        let wall_col   = ( transform.translation.x / ISO_STEP_X).round() as i32;

        let target_alpha = if wall_depth > player_depth
            && (wall_col - player_col).abs() <= 1
        {
            OCCLUDED_ALPHA
        } else {
            1.0
        };

        let current = sprite.color.alpha();
        let next = current + (target_alpha - current) * (time.delta_secs() * FADE_SPEED).min(1.0);
        sprite.color = sprite.color.with_alpha(next);
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        let dungeon = generate_dungeon(NUM_DUNGEON_FLOORS);
        app.insert_resource(dungeon)
            .init_resource::<DoorRegistry>()
            .add_systems(Startup, (startup_spawn_tiles, startup_spawn_doors).chain())
            .add_systems(Update, fade_occluding_walls);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_map_has_rooms() {
        let map = generate_map();
        assert!(!map.rooms.is_empty(), "dungeon must have at least one room");
    }

    #[test]
    fn first_room_center_is_walkable() {
        let map = generate_map();
        let (cx, cy) = map.rooms[0].center();
        assert!(map.is_walkable(cx, cy));
    }

    #[test]
    fn first_room_center_is_passable() {
        let map = generate_map();
        let (cx, cy) = map.rooms[0].center();
        assert!(map.is_passable(cx, cy));
    }

    #[test]
    fn map_bounds_correct() {
        let map = generate_map();
        assert!(!map.in_bounds(-1, 0));
        assert!(!map.in_bounds(0, -1));
        assert!(!map.in_bounds(MAP_WIDTH, 0));
        assert!(!map.in_bounds(0, MAP_HEIGHT));
        assert!(map.in_bounds(0, 0));
        assert!(map.in_bounds(MAP_WIDTH - 1, MAP_HEIGHT - 1));
    }

    #[test]
    fn props_len_matches_tiles() {
        let map = generate_map();
        assert_eq!(map.tiles.len(), map.props.len());
    }

    // ── Test door ─────────────────────────────────────────────────────────────

    #[test]
    fn test_door_is_placed() {
        let map = generate_map();
        assert_eq!(map.doors.len(), 1, "exactly one test door should be placed");
    }

    #[test]
    fn test_door_tile_is_floor() {
        let map = generate_map();
        let door = &map.doors[0];
        assert!(
            map.is_walkable(door.x, door.y),
            "door tile ({},{}) must be walkable (Floor)",
            door.x,
            door.y
        );
    }

    #[test]
    fn test_door_starts_closed() {
        let map = generate_map();
        assert!(!map.doors[0].open, "test door should start closed");
    }

    #[test]
    fn test_door_outside_tile_is_floor() {
        let map = generate_map();
        let door = &map.doors[0];
        let outside_y = door.y + 1;
        assert!(
            map.in_bounds(door.x, outside_y),
            "tile south of door must be in bounds"
        );
        assert!(
            map.is_walkable(door.x, outside_y),
            "tile south of door ({},{}) must be walkable",
            door.x,
            outside_y
        );
    }

    // ── Dungeon (multi-floor) ─────────────────────────────────────────────────

    #[test]
    fn generate_dungeon_has_correct_floor_count() {
        let dungeon = generate_dungeon(3);
        assert_eq!(dungeon.floors.len(), 3);
    }

    #[test]
    fn dungeon_starts_on_floor_zero() {
        let dungeon = generate_dungeon(2);
        assert_eq!(dungeon.current_floor, 0);
    }

    #[test]
    fn stairs_down_placed_on_all_but_last_floor() {
        let dungeon = generate_dungeon(3);
        for f in 0..2 {
            let has_down = dungeon.floors[f]
                .stair_links
                .values()
                .any(|node| node.down.is_some());
            assert!(has_down, "floor {f} must have a downward stair link");
        }
        // The last floor must NOT have a downward link.
        assert!(
            dungeon.floors[2].stair_links.values().all(|node| node.down.is_none()),
            "last floor must not have a downward link"
        );
    }

    #[test]
    fn intermediate_floor_has_stairsmid() {
        let dungeon = generate_dungeon(3);
        let has_mid = dungeon.floors[1].stair_links.keys().any(|&(x, y)| {
            dungeon.floors[1].tiles[dungeon.floors[1].idx(x, y)] == TileType::StairsMid
        });
        assert!(has_mid, "floor 1 of 3 must contain a StairsMid tile");
    }

    #[test]
    fn stairsmid_has_both_links() {
        let dungeon = generate_dungeon(3);
        for (&(x, y), node) in &dungeon.floors[1].stair_links {
            if dungeon.floors[1].tiles[dungeon.floors[1].idx(x, y)] == TileType::StairsMid {
                assert!(node.up.is_some(),   "StairsMid at ({x},{y}) must have an up link");
                assert!(node.down.is_some(), "StairsMid at ({x},{y}) must have a down link");
            }
        }
    }

    #[test]
    fn stairs_links_are_bidirectional() {
        let dungeon = generate_dungeon(3);
        for f in 0..dungeon.floors.len() {
            for (&pos, node) in &dungeon.floors[f].stair_links {
                if let Some(down) = &node.down {
                    let back = dungeon.floors[down.target_floor]
                        .stair_links
                        .get(&down.target_pos)
                        .expect("target floor must have a return stair node");
                    let up = back.up.as_ref().expect("return node must have an up link");
                    assert_eq!(up.target_floor, f);
                    assert_eq!(up.target_pos, pos);
                }
            }
        }
    }

    #[test]
    fn stair_tiles_are_walkable() {
        let dungeon = generate_dungeon(3);
        for (f, floor) in dungeon.floors.iter().enumerate() {
            for &(x, y) in floor.stair_links.keys() {
                assert!(
                    floor.is_walkable(x, y),
                    "stair tile at ({x},{y}) on floor {f} must be walkable"
                );
            }
        }
    }

    #[test]
    fn single_floor_dungeon_has_no_stairs() {
        let dungeon = generate_dungeon(1);
        assert!(dungeon.floors[0].stair_links.is_empty());
    }

    // ── TileType::is_walkable ─────────────────────────────────────────────────

    #[test]
    fn walkable_tile_types() {
        for t in [
            TileType::Floor,
            TileType::Dirt,
            TileType::Planks,
            TileType::Bridge,
            TileType::StairsDown,
            TileType::StairsMid,
            TileType::StairsUp,
            TileType::DoorOpen,
            TileType::Archway,
        ] {
            assert!(t.is_walkable(), "{t:?} should be walkable");
        }
    }

    #[test]
    fn impassable_tile_types() {
        for t in [
            TileType::Wall,
            TileType::DoorClosed,
            TileType::BrokenWall,
            TileType::Window,
        ] {
            assert!(!t.is_walkable(), "{t:?} should not be walkable");
        }
    }

    // ── Map::is_passable ──────────────────────────────────────────────────────

    #[test]
    fn prop_blocks_passability() {
        let mut map = generate_map();
        let (cx, cy) = map.rooms[0].center();
        assert!(map.is_passable(cx, cy));
        let idx = map.idx(cx, cy);
        map.props[idx] = Some(PropType::Barrel);
        assert!(!map.is_passable(cx, cy), "prop should block passability");
        assert!(map.is_walkable(cx, cy), "prop must not affect tile walkability");
    }

    // ── TileType helpers ──────────────────────────────────────────────────────

    #[test]
    fn floor_like_tiles_have_asset_prefix() {
        for t in [
            TileType::Floor,
            TileType::Dirt,
            TileType::Planks,
            TileType::Bridge,
            TileType::StairsDown,
            TileType::StairsMid,
            TileType::StairsUp,
        ] {
            assert!(t.is_floor_like());
            let p = t.floor_asset_prefix();
            assert!(!p.is_empty());
            assert!(!p.contains('/'));
        }
    }

    #[test]
    fn wall_like_tiles_have_asset_prefix() {
        for t in [
            TileType::Wall,
            TileType::DoorClosed,
            TileType::DoorOpen,
            TileType::BrokenWall,
            TileType::Window,
            TileType::Archway,
        ] {
            let p = t.wall_asset_prefix();
            assert!(!p.is_empty());
            assert!(!p.contains('/'));
        }
    }

    #[test]
    fn occluding_walls_are_impassable() {
        for t in [
            TileType::Wall,
            TileType::DoorClosed,
            TileType::BrokenWall,
            TileType::Window,
        ] {
            assert!(t.is_occluding_wall());
            assert!(!t.is_walkable(), "{t:?} is occluding but walkable");
        }
    }

    #[test]
    fn walkable_wall_openings_do_not_occlude() {
        for t in [TileType::DoorOpen, TileType::Archway] {
            assert!(!t.is_occluding_wall(), "{t:?} should not occlude");
            assert!(t.is_walkable());
        }
    }

    // ── PropType::asset_prefix ────────────────────────────────────────────────

    #[test]
    fn all_prop_prefixes_are_nonempty() {
        for p in [
            PropType::Barrel,
            PropType::Barrels,
            PropType::BarrelsStacked,
            PropType::ChestClosed,
            PropType::ChestOpen,
            PropType::WoodenCrate,
            PropType::WoodenCrates,
            PropType::WoodenPile,
            PropType::Pillar,
            PropType::PillarWood,
            PropType::TableRound,
            PropType::TableRoundChairs,
            PropType::TableShort,
            PropType::TableShortChairs,
            PropType::Chair,
            PropType::WoodenSupports,
            PropType::WoodenSupportsBeam,
        ] {
            let prefix = p.asset_prefix();
            assert!(!prefix.is_empty(), "{p:?} has empty asset prefix");
            assert!(!prefix.contains('/'), "{p:?} prefix contains a path separator");
        }
    }
}
