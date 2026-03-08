use bevy::{prelude::*, sprite::Anchor};
use rand::Rng;

use crate::{
    components::{MapPosition, MapTile, Player, PropTile, WallTile, YSort},
    ISO_STEP_X, ISO_STEP_Y, MAP_HEIGHT, MAP_WIDTH, TILE_SCALE,
};

// ── Dungeon generation tunables ───────────────────────────────────────────────

const MAX_ROOMS: usize = 12;
const MIN_ROOM_SIZE: i32 = 3;
const MAX_ROOM_SIZE: i32 = 8;

// ── Tile type ─────────────────────────────────────────────────────────────────

/// The base type of a map cell.  Determines rendering and tile-level walkability
/// (see [`Map::is_walkable`]).  Gameplay-level passability also accounts for
/// props — see [`Map::is_passable`].
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
    /// Spiral staircase — level transition point.
    Stairs,

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
                | Self::Stairs
                | Self::DoorOpen
                | Self::Archway
        )
    }

    /// Returns `true` if this tile renders as a floor sprite (random variant).
    pub fn is_floor_like(self) -> bool {
        matches!(
            self,
            Self::Floor | Self::Dirt | Self::Planks | Self::Bridge | Self::Stairs
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
            Self::Stairs => "stairsSpiral",
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
}

// ── Prop type ─────────────────────────────────────────────────────────────────

/// An object placed on top of a floor tile.  Props block player movement
/// (see [`Map::is_passable`]) but do not block the lantern beam.
/// Each variant has four directional sprites (`_N`, `_E`, `_S`, `_W`).
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
    pub rooms: Vec<Rect>,
}

impl Map {
    fn new() -> Self {
        let size = (MAP_WIDTH * MAP_HEIGHT) as usize;
        Self {
            width: MAP_WIDTH,
            height: MAP_HEIGHT,
            tiles: vec![TileType::Wall; size],
            props: vec![None; size],
            rooms: Vec::new(),
        }
    }

    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.width + x) as usize
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height
    }

    /// Tile-level walkability check — ignores props.
    /// Used by wall-face selection, lantern beam casting, and map generation.
    pub fn is_walkable(&self, x: i32, y: i32) -> bool {
        self.in_bounds(x, y) && self.tiles[self.idx(x, y)].is_walkable()
    }

    /// Gameplay passability — `false` if the tile is impassable *or* a prop
    /// occupies the cell.  Used by player movement and BFS pathfinding.
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

    map
}

// ── Startup system: spawn isometric tile sprites ──────────────────────────────

fn spawn_map_tiles(mut commands: Commands, map: Res<Map>, asset_server: Res<AssetServer>) {
    const DIRS: [&str; 4] = ["N", "E", "S", "W"];
    // Anchor places the sprite's isometric diamond center at the world position.
    // In the ~256×320 tile images the diamond center sits ~30% below image center.
    let anchor = Anchor::Custom(Vec2::new(0.0, -0.30));
    let mut rng = rand::thread_rng();

    for y in 0..map.height {
        for x in 0..map.width {
            let wx = (x as f32 - y as f32) * ISO_STEP_X;
            let wy = -(x as f32 + y as f32) * ISO_STEP_Y;
            // Fixed depth for floor tiles: higher col+row → higher z → rendered
            // in front of tiles farther from the viewer.  Offset well below
            // YSort objects so floors never obscure entities.
            let floor_z = (x + y) as f32 * 0.001 - 200.0;

            let tile = map.tiles[map.idx(x, y)];

            if tile.is_floor_like() {
                // ── Floor ─────────────────────────────────────────────────────
                let dir = DIRS[rng.gen_range(0..4)];
                let image = asset_server
                    .load(format!("Isometric/{}_{dir}.png", tile.floor_asset_prefix()));
                commands.spawn((
                    MapTile,
                    Sprite { image, anchor, ..Default::default() },
                    Transform::from_xyz(wx, wy, floor_z).with_scale(Vec3::splat(TILE_SCALE)),
                ));
            } else {
                // ── Wall-like ─────────────────────────────────────────────────
                let s = map.is_walkable(x, y + 1);
                let n = map.is_walkable(x, y - 1);
                let e = map.is_walkable(x + 1, y);
                let w = map.is_walkable(x - 1, y);

                let image = if tile == TileType::Wall {
                    // Standard walls use corner pieces where two runs meet.
                    // Corner pieces cover two perpendicular faces, filling the
                    // visual gap that forms at run intersections.
                    //   corner_N: floor south (dy+1) AND east (dx+1)
                    //   corner_E: floor south (dy+1) AND west (dx-1)
                    //   corner_S: floor north (dy-1) AND west (dx-1)
                    //   corner_W: floor north (dy-1) AND east (dx+1)
                    if s && e      { asset_server.load("Isometric/stoneWallCorner_N.png") }
                    else if s && w { asset_server.load("Isometric/stoneWallCorner_E.png") }
                    else if n && w { asset_server.load("Isometric/stoneWallCorner_S.png") }
                    else if n && e { asset_server.load("Isometric/stoneWallCorner_W.png") }
                    else if s      { asset_server.load("Isometric/stoneWall_N.png") }
                    else if n      { asset_server.load("Isometric/stoneWall_S.png") }
                    else if e      { asset_server.load("Isometric/stoneWall_W.png") }
                    else if w      { asset_server.load("Isometric/stoneWall_E.png") }
                    else           { continue; } // interior void — no visible face
                } else {
                    // Other wall variants have no corner sprites; pick the first
                    // matching single-face direction.
                    let prefix = tile.wall_asset_prefix();
                    if s      { asset_server.load(format!("Isometric/{prefix}_N.png")) }
                    else if n { asset_server.load(format!("Isometric/{prefix}_S.png")) }
                    else if e { asset_server.load(format!("Isometric/{prefix}_W.png")) }
                    else if w { asset_server.load(format!("Isometric/{prefix}_E.png")) }
                    else      { continue; } // interior void — no visible face
                };

                let mut entity = commands.spawn((
                    MapTile,
                    YSort,
                    Sprite { image, anchor, ..Default::default() },
                    Transform::from_xyz(wx, wy, 0.0).with_scale(Vec3::splat(TILE_SCALE)),
                ));
                // Only solid, non-walkable walls participate in occlusion fading.
                if tile.is_occluding_wall() {
                    entity.insert(WallTile);
                }
            }

            // ── Prop ──────────────────────────────────────────────────────────
            // Spawned on top of the floor tile at the same grid cell.
            // Props use YSort so they depth-order correctly with the player.
            if let Some(prop) = map.props[map.idx(x, y)] {
                let dir = DIRS[rng.gen_range(0..4)];
                let image = asset_server
                    .load(format!("Isometric/{}_{dir}.png", prop.asset_prefix()));
                commands.spawn((
                    MapTile,
                    PropTile,
                    YSort,
                    Sprite { image, anchor, ..Default::default() },
                    Transform::from_xyz(wx, wy, 0.0).with_scale(Vec3::splat(TILE_SCALE)),
                ));
            }
        }
    }
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
    const FADE_SPEED: f32 = 10.0; // alpha units per second

    let Ok(player_pos) = player_q.get_single() else {
        return;
    };
    let player_depth = player_pos.x + player_pos.y; // higher → closer to viewer
    let player_col   = player_pos.x - player_pos.y; // isometric screen column

    for (transform, mut sprite) in wall_q.iter_mut() {
        // Recover integer grid coords from the world-space Transform.
        // wx = (x − y) * ISO_STEP_X  →  x − y = wx / ISO_STEP_X
        // wy = −(x + y) * ISO_STEP_Y  →  x + y = −wy / ISO_STEP_Y
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
        let map = generate_map();
        app.insert_resource(map)
            .add_systems(Startup, spawn_map_tiles)
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
        // Generated maps have no props, so passable == walkable.
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

    // ── TileType::is_walkable ─────────────────────────────────────────────────

    #[test]
    fn walkable_tile_types() {
        for t in [
            TileType::Floor,
            TileType::Dirt,
            TileType::Planks,
            TileType::Bridge,
            TileType::Stairs,
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
            TileType::Stairs,
        ] {
            assert!(t.is_floor_like());
            // Prefix must be non-empty and not contain a path separator.
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
        // Every tile that occludes the player must also be impassable, so the
        // player can never stand inside an occluding wall.
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
