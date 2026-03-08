use bevy::{prelude::*, sprite::Anchor};
use rand::Rng;

use crate::{
    components::{MapPosition, Player, WallTile, YSort},
    ISO_STEP_X, ISO_STEP_Y, MAP_HEIGHT, MAP_WIDTH, TILE_SCALE,
};

// ── Dungeon generation tunables ───────────────────────────────────────────────

const MAX_ROOMS: usize = 12;
const MIN_ROOM_SIZE: i32 = 3;
const MAX_ROOM_SIZE: i32 = 8;

// ── Tile type ─────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TileType {
    Wall,
    Floor,
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
    pub tiles: Vec<TileType>,
    pub rooms: Vec<Rect>,
}

impl Map {
    fn new() -> Self {
        Self {
            width: MAP_WIDTH,
            height: MAP_HEIGHT,
            tiles: vec![TileType::Wall; (MAP_WIDTH * MAP_HEIGHT) as usize],
            rooms: Vec::new(),
        }
    }

    pub fn idx(&self, x: i32, y: i32) -> usize {
        (y * self.width + x) as usize
    }

    pub fn in_bounds(&self, x: i32, y: i32) -> bool {
        x >= 0 && x < self.width && y >= 0 && y < self.height
    }

    pub fn is_walkable(&self, x: i32, y: i32) -> bool {
        self.in_bounds(x, y) && self.tiles[self.idx(x, y)] == TileType::Floor
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
    let floor_variants: [Handle<Image>; 4] = [
        asset_server.load("Isometric/stone_N.png"),
        asset_server.load("Isometric/stone_E.png"),
        asset_server.load("Isometric/stone_S.png"),
        asset_server.load("Isometric/stone_W.png"),
    ];
    let wall_n: Handle<Image> = asset_server.load("Isometric/stoneWall_N.png");
    let wall_s: Handle<Image> = asset_server.load("Isometric/stoneWall_S.png");
    let wall_e: Handle<Image> = asset_server.load("Isometric/stoneWall_E.png");
    let wall_w: Handle<Image> = asset_server.load("Isometric/stoneWall_W.png");
    // Corner pieces — each covers two perpendicular wall faces:
    //   corner_N: floor south (dy+1) AND east (dx+1)
    //   corner_E: floor south (dy+1) AND west (dx-1)
    //   corner_S: floor north (dy-1) AND west (dx-1)
    //   corner_W: floor north (dy-1) AND east (dx+1)
    let corner_n: Handle<Image> = asset_server.load("Isometric/stoneWallCorner_N.png");
    let corner_s: Handle<Image> = asset_server.load("Isometric/stoneWallCorner_S.png");
    let corner_e: Handle<Image> = asset_server.load("Isometric/stoneWallCorner_E.png");
    let corner_w: Handle<Image> = asset_server.load("Isometric/stoneWallCorner_W.png");

    // Anchor that places the sprite's isometric diamond center at the world pos.
    // In the ~256×320 tile images, the diamond center sits ~30% below image center.
    let floor_anchor = Anchor::Custom(Vec2::new(0.0, -0.30));

    let mut rng = rand::thread_rng();

    for y in 0..map.height {
        for x in 0..map.width {
            let wx = (x as f32 - y as f32) * ISO_STEP_X;
            let wy = -(x as f32 + y as f32) * ISO_STEP_Y;
            // Fixed depth for floor: higher col+row → higher z → in front of
            // tiles farther from the viewer. Offset well below YSort objects.
            let floor_z = (x + y) as f32 * 0.001 - 200.0;

            match map.tiles[map.idx(x, y)] {
                TileType::Floor => {
                    let variant = rng.gen_range(0..floor_variants.len());
                    commands.spawn((
                        Sprite {
                            image: floor_variants[variant].clone(),
                            anchor: floor_anchor,
                            ..Default::default()
                        },
                        Transform::from_xyz(wx, wy, floor_z)
                            .with_scale(Vec3::splat(TILE_SCALE)),
                    ));
                }
                TileType::Wall => {
                    let s = map.is_walkable(x, y + 1);
                    let n = map.is_walkable(x, y - 1);
                    let e = map.is_walkable(x + 1, y);
                    let w = map.is_walkable(x - 1, y);

                    // Corner pieces take priority — they cover two perpendicular faces
                    // and fill the visual gap that occurs where two wall runs meet.
                    // Falls through to single-face walls, then skips interior voids.
                    let wall_tex = if s && e { corner_n.clone() }
                    else if s && w           { corner_e.clone() }
                    else if n && w           { corner_s.clone() }
                    else if n && e           { corner_w.clone() }
                    else if s                { wall_n.clone() }
                    else if n                { wall_s.clone() }
                    else if e                { wall_w.clone() }
                    else if w                { wall_e.clone() }
                    else                     { continue; }; // interior void — skip


                    commands.spawn((
                        YSort,
                        WallTile,
                        Sprite {
                            image: wall_tex,
                            // Same anchor as floor tiles: ground-contact line sits at
                            // ~80% from the top of each wall image (20% from bottom),
                            // matching the floor diamond centre convention.
                            anchor: Anchor::Custom(Vec2::new(0.0, -0.30)),
                            ..Default::default()
                        },
                        Transform::from_xyz(wx, wy, 0.0)
                            .with_scale(Vec3::splat(TILE_SCALE))
                    ));
                }
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
    fn map_bounds_correct() {
        let map = generate_map();
        assert!(!map.in_bounds(-1, 0));
        assert!(!map.in_bounds(0, -1));
        assert!(!map.in_bounds(MAP_WIDTH, 0));
        assert!(!map.in_bounds(0, MAP_HEIGHT));
        assert!(map.in_bounds(0, 0));
        assert!(map.in_bounds(MAP_WIDTH - 1, MAP_HEIGHT - 1));
    }
}
