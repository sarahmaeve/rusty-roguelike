use bevy::prelude::*;
use bevy_light_2d::prelude::*;

use crate::components::{MainCamera, Player};

const CAMERA_LERP_SPEED: f32 = 0.1;

// ── Startup system: spawn the 2D camera ──────────────────────────────────────

fn spawn_camera(mut commands: Commands) {
    commands.spawn((
        MainCamera,
        Camera2d::default(),
        // Near-black ambient so the player's torch is the dominant light source.
        AmbientLight2d {
            brightness: 0.06,
            color: Color::srgb(0.6, 0.65, 1.0), // faint cool blue tint for darkness
        },
    ));
}

// ── Update system: smooth-follow the player ───────────────────────────────────

fn camera_follow(
    player_q: Query<&Transform, With<Player>>,
    mut camera_q: Query<&mut Transform, (With<MainCamera>, Without<Player>)>,
) {
    let Ok(player_tf) = player_q.get_single() else {
        return;
    };
    let Ok(mut camera_tf) = camera_q.get_single_mut() else {
        return;
    };

    let target = player_tf.translation.truncate().extend(camera_tf.translation.z);
    camera_tf.translation = camera_tf.translation.lerp(target, CAMERA_LERP_SPEED);
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_camera)
            .add_systems(Update, camera_follow);
    }
}
