use bevy::{prelude::*, sprite::Anchor};
use rand::Rng;

use crate::{components::YSort, MAP_HEIGHT, MAP_WIDTH, TILE_SIZE};

// ── Dungeon generation tunables ───────────────────────────────────────────────

const MAX_ROOMS: usize = 15;
const MIN_ROOM_SIZE: i32 = 4;
const MAX_ROOM_SIZE: i32 = 12;

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

    // ── Carving helpers ───────────────────────────────────────────────────────

    fn carve_room(&mut self, room: &Rect) {
        for y in (room.y1 + 1)..room.y2 {
            for x in (room.x1 + 1)..room.x2 {
                let idx = self.idx(x, y);
                self.tiles[idx] = TileType::Floor;
            }
        }
    }

    fn carve_h_corridor(&mut self, x1: i32, x2: i32, y: i32) {
        // 3 tiles tall so the player is never occluded by adjacent walls.
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
        // 3 tiles wide so the player is never occluded by adjacent walls.
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

/// Builds a rooms-and-corridors dungeon procedurally.
pub fn generate_map() -> Map {
    let mut map = Map::new();
    let mut rng = rand::thread_rng();

    for _ in 0..MAX_ROOMS {
        let w = rng.gen_range(MIN_ROOM_SIZE..=MAX_ROOM_SIZE);
        let h = rng.gen_range(MIN_ROOM_SIZE..=MAX_ROOM_SIZE);
        let x = rng.gen_range(1..MAP_WIDTH - w - 1);
        let y = rng.gen_range(1..MAP_HEIGHT - h - 1);

        let new_room = Rect::new(x, y, w, h);

        // Skip rooms that overlap an existing one.
        if map.rooms.iter().any(|r| r.intersects(&new_room)) {
            continue;
        }

        map.carve_room(&new_room);

        // Connect to the previous room with an L-shaped corridor.
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

// ── Startup system: spawn tile sprites ───────────────────────────────────────

fn spawn_map_tiles(mut commands: Commands, map: Res<Map>) {
    const WALL_FACE_COLOR: Color = Color::srgb(0.25, 0.18, 0.12);
    const WALL_TOP_COLOR: Color = Color::srgb(0.38, 0.28, 0.18);
    const FLOOR_COLOR: Color = Color::srgb(0.35, 0.32, 0.28);
    // Floor tiles directly adjacent to a south-facing wall are slightly darker
    // to approximate contact shadow / ambient occlusion.
    const FLOOR_SHADOW_COLOR: Color = Color::srgb(0.22, 0.20, 0.18);

    for y in 0..map.height {
        for x in 0..map.width {
            let wx = x as f32 * TILE_SIZE;
            let wy = y as f32 * TILE_SIZE;

            match map.tiles[map.idx(x, y)] {
                TileType::Floor => {
                    // Darken the floor tile if the wall immediately above it (y+1)
                    // has a south-facing face — i.e. that wall will cast a shadow down.
                    let in_wall_shadow = y + 1 < map.height
                        && map.tiles[map.idx(x, y + 1)] == TileType::Wall;
                    let color = if in_wall_shadow {
                        FLOOR_SHADOW_COLOR
                    } else {
                        FLOOR_COLOR
                    };

                    commands.spawn((
                        Sprite {
                            color,
                            custom_size: Some(Vec2::splat(TILE_SIZE)),
                            ..Default::default()
                        },
                        Transform::from_xyz(wx, wy, -1.0),
                    ));
                }
                TileType::Wall => {
                    // Top surface — always visible from above, sits flat at tile level.
                    commands.spawn((
                        Sprite {
                            color: WALL_TOP_COLOR,
                            custom_size: Some(Vec2::splat(TILE_SIZE)),
                            ..Default::default()
                        },
                        Transform::from_xyz(wx, wy, -0.5),
                    ));

                    // South face — only rendered when this wall borders a floor tile
                    // below it. In a top-down view this is the only face the viewer
                    // can see; rendering it everywhere would occlude narrow corridors.
                    let has_south_face = y > 0
                        && map.tiles[map.idx(x, y - 1)] == TileType::Floor;

                    if has_south_face {
                        commands.spawn((
                            YSort,
                            Sprite {
                                color: WALL_FACE_COLOR,
                                custom_size: Some(Vec2::new(TILE_SIZE, TILE_SIZE * 1.5)),
                                anchor: Anchor::TopCenter,
                                ..Default::default()
                            },
                            // Position at the bottom edge of the wall tile, hang downward.
                            Transform::from_xyz(wx, wy, 0.0),
                        ));
                    }
                }
            }
        }
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct MapPlugin;

impl Plugin for MapPlugin {
    fn build(&self, app: &mut App) {
        let map = generate_map();
        app.insert_resource(map)
            .add_systems(Startup, spawn_map_tiles);
    }
}
