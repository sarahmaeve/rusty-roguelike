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

pub const TILE_SIZE: f32 = 16.0;
pub const MAP_WIDTH: i32 = 80;
pub const MAP_HEIGHT: i32 = 45;

/// Updates Z on every YSort entity each frame so that entities lower on screen
/// (smaller world Y) render in front of entities higher up (larger world Y).
fn y_sort(mut query: Query<&mut Transform, With<YSort>>) {
    for mut tf in query.iter_mut() {
        tf.translation.z = -tf.translation.y / 10_000.0;
    }
}

fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: "Impossible Mission II".to_string(),
                    resolution: (1280.0, 720.0).into(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
        )
        .add_plugins((Light2dPlugin, MapPlugin, PlayerPlugin, CameraPlugin))
        .add_systems(Update, y_sort)
        .run();
}
