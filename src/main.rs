mod camera;
mod components;
mod map;
mod player;

use bevy::prelude::*;
use bevy_light_2d::prelude::*;
use components::YSort;

use camera::CameraPlugin;
use map::MapPlugin;
use player::PlayerPlugin;

/// Isometric grid step in world units (half the tile diamond width/height).
/// Assumes tile images are ~256px wide at TILE_SCALE 0.5 → 128px rendered width,
/// diamond half-width = 64, diamond half-height = 32.
pub const ISO_STEP_X: f32 = 64.0;
pub const ISO_STEP_Y: f32 = 32.0;

/// Uniform scale applied to all tile sprites.
pub const TILE_SCALE: f32 = 0.5;

pub const MAP_WIDTH: i32 = 40;
pub const MAP_HEIGHT: i32 = 22;

/// Y-sort: entities lower on screen (more negative world Y) render in front.
/// In isometric layout, lower on screen = higher col+row = correct depth order.
fn y_sort(mut query: Query<&mut Transform, With<YSort>>) {
    for mut tf in query.iter_mut() {
        tf.translation.z = -tf.translation.y / 10_000.0;
    }
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Impossible Mission II".to_string(),
                        resolution: (1280.0, 720.0).into(),
                        ..Default::default()
                    }),
                    ..Default::default()
                })
                .set(ImagePlugin::default_nearest()),
        )
        .add_plugins((Light2dPlugin, MapPlugin, PlayerPlugin, CameraPlugin))
        .add_systems(Update, y_sort)
        .run();
}
