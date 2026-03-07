use bevy::prelude::*;
use bevy_light_2d::prelude::*;

use crate::{
    components::{MapPosition, Player, YSort},
    map::Map,
    ISO_STEP_X, ISO_STEP_Y,
};

// ── Startup system: spawn the player ─────────────────────────────────────────

fn spawn_player(mut commands: Commands, map: Res<Map>) {
    let (cx, cy) = map.rooms[0].center();
    let pos = MapPosition::new(cx, cy);
    let world = pos.to_world(0.0);

    commands
        .spawn((
            Player,
            YSort,
            pos,
            Sprite {
                color: Color::srgb(0.9, 0.8, 0.1),
                custom_size: Some(Vec2::splat(ISO_STEP_X * 0.6)),
                ..Default::default()
            },
            Transform::from_xyz(world.x, world.y, 0.0),
            PointLight2d {
                radius: 350.0,
                intensity: 3.5,
                color: Color::srgb(1.0, 0.82, 0.45),
                cast_shadows: false,
                ..Default::default()
            },
        ))
        .with_children(|parent| {
            // Isometric drop shadow: a flattened ellipse below the player sprite.
            parent.spawn((
                Sprite {
                    color: Color::srgba(0.0, 0.0, 0.0, 0.45),
                    custom_size: Some(Vec2::new(ISO_STEP_X * 0.6, ISO_STEP_Y * 0.3)),
                    ..Default::default()
                },
                Transform::from_xyz(0.0, -ISO_STEP_Y * 0.5, -0.01),
            ));
        });
}

// ── Update system: tile-based movement ───────────────────────────────────────

fn player_movement(
    keyboard: Res<ButtonInput<KeyCode>>,
    map: Res<Map>,
    mut query: Query<(&mut MapPosition, &mut Transform), With<Player>>,
) {
    let Ok((mut pos, mut transform)) = query.get_single_mut() else {
        return;
    };

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

    let new_x = pos.x + dx;
    let new_y = pos.y + dy;

    if map.is_walkable(new_x, new_y) {
        pos.x = new_x;
        pos.y = new_y;
        let world = pos.to_world(0.0);
        // Z is managed by y_sort each frame; only update X/Y here.
        transform.translation.x = world.x;
        transform.translation.y = world.y;
    }
}

// ── Plugin ────────────────────────────────────────────────────────────────────

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_player)
            .add_systems(Update, player_movement);
    }
}
