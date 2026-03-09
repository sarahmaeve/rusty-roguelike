use bevy::prelude::*;

/// Marks the player entity.
#[derive(Component)]
pub struct Player;

/// Marks the main 2D camera entity.
#[derive(Component)]
pub struct MainCamera;

/// Any entity with this component will have its Z updated each frame based on
/// its world Y position, giving correct painter's-algorithm depth ordering for
/// the isometric view (lower on screen = higher Z = rendered in front).
#[derive(Component)]
pub struct YSort;

/// Marks a wall tile sprite so the occlusion system can fade it when it lies
/// between the camera and the player.
#[derive(Component)]
pub struct WallTile;

/// Marks any map tile (floor or wall) so the lighting-cull system can hide
/// tiles that fall outside the player's current light envelope.
#[derive(Component)]
pub struct MapTile;

/// Marks a prop entity (barrel, chest, crate, etc.) placed on top of a floor
/// tile.  Props use `YSort` for depth ordering and block player movement via
/// [`Map::is_passable`], but do not block the lantern beam.
#[derive(Component)]
pub struct PropTile;

/// Marks a tile entity whose staircase originates on a different (shallower)
/// floor — i.e. a `StairsUp` tile.  These tiles are always fully visible
/// regardless of the player's current light radius, because the opening
/// above them provides its own ambient illumination.
#[derive(Component)]
pub struct StairsUpTile;

/// Marks an intermediate stair tile (`StairsMid`) that sits between two floors.
/// Like `StairsUpTile`, the shaft opening above keeps it always visible.
/// Press **W / ↑** to ascend or **S / ↓** to descend while standing on it.
#[derive(Component)]
pub struct StairsMidTile;

/// Isometric wall-face direction, matching the asset naming convention
/// (`_N`, `_E`, `_S`, `_W` suffixes).  The suffix indicates which face of the
/// wall tile is visible to the player.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum CardinalDir {
    N,
    E,
    S,
    W,
}

impl CardinalDir {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::N => "N",
            Self::E => "E",
            Self::S => "S",
            Self::W => "W",
        }
    }
}

/// An interactive door entity.  The map tile at its grid position is always
/// `TileType::Floor`; this entity provides the visual and movement blocking.
/// Toggle with the E key when adjacent.
#[derive(Component)]
pub struct Door {
    pub open: bool,
    /// Which face of the door sprite is shown (matches asset suffix convention).
    pub facing: CardinalDir,
}

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

    /// Convert tile grid coords to isometric world-space Vec3.
    ///
    /// Projection:
    ///   world_x = (col - row) * ISO_STEP_X
    ///   world_y = -(col + row) * ISO_STEP_Y
    pub fn to_world(self, z: f32) -> Vec3 {
        Vec3::new(
            (self.x as f32 - self.y as f32) * crate::ISO_STEP_X,
            -(self.x as f32 + self.y as f32) * crate::ISO_STEP_Y,
            z,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_world_origin() {
        let pos = MapPosition::new(0, 0);
        let w = pos.to_world(0.0);
        assert_eq!(w.x, 0.0);
        assert_eq!(w.y, 0.0);
        assert_eq!(w.z, 0.0);
    }

    #[test]
    fn to_world_col_step() {
        // Moving one column right → right and down on screen.
        let pos = MapPosition::new(1, 0);
        let w = pos.to_world(0.0);
        assert_eq!(w.x, crate::ISO_STEP_X);
        assert_eq!(w.y, -crate::ISO_STEP_Y);
    }

    #[test]
    fn to_world_row_step() {
        // Moving one row down → left and down on screen.
        let pos = MapPosition::new(0, 1);
        let w = pos.to_world(0.0);
        assert_eq!(w.x, -crate::ISO_STEP_X);
        assert_eq!(w.y, -crate::ISO_STEP_Y);
    }

    #[test]
    fn to_world_diagonal_cancels_x() {
        // Equal col and row → world_x cancels to 0, world_y doubles.
        let pos = MapPosition::new(3, 3);
        let w = pos.to_world(0.0);
        assert_eq!(w.x, 0.0);
        assert_eq!(w.y, -6.0 * crate::ISO_STEP_Y);
    }
}
