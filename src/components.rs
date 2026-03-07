use bevy::prelude::*;

/// Marks the player entity.
#[derive(Component)]
pub struct Player;

/// Marks the main 2D camera entity.
#[derive(Component)]
pub struct MainCamera;

/// Any entity with this component will have its Z updated each frame based on
/// its world Y position, creating correct painter's-algorithm depth ordering.
#[derive(Component)]
pub struct YSort;

/// Discrete tile-grid position for any entity on the map.
#[derive(Component, Clone, Copy, PartialEq, Eq, Debug)]
pub struct MapPosition {
    pub x: i32,
    pub y: i32,
}

impl MapPosition {
    pub fn new(x: i32, y: i32) -> Self {
        Self { x, y }
    }

    /// Convert tile coords to a world-space Vec3 at the given Z layer.
    pub fn to_world(&self, z: f32) -> Vec3 {
        Vec3::new(
            self.x as f32 * crate::TILE_SIZE,
            self.y as f32 * crate::TILE_SIZE,
            z,
        )
    }
}
