//! A few coloured blocks moving under a CRT. Left and right go through the included looks.
//!
//! ```sh
//! cargo run -p crtsim-bevy --example tube
//! ```

use bevy::prelude::*;
use crtsim_bevy::{Crt, CrtPlugin};

#[derive(Resource)]
struct Looks {
    names: Vec<&'static str>,
    shown: usize,
}

#[derive(Component)]
struct Bounce(Vec2);

fn main() {
    let names = crtsim_core::presets::included()
        .into_iter()
        .map(|preset| preset.name)
        .collect();
    App::new()
        .add_plugins((DefaultPlugins, CrtPlugin))
        .insert_resource(Looks { names, shown: 0 })
        .add_systems(Startup, setup)
        .add_systems(Update, (bounce, choose_look))
        .run();
}

fn setup(mut commands: Commands, looks: Res<Looks>) {
    commands.spawn((Camera2d, Crt::preset(looks.names[0]).unwrap()));
    let colours = [
        Color::srgb(0.9, 0.2, 0.2),
        Color::srgb(0.2, 0.8, 0.3),
        Color::srgb(0.2, 0.4, 0.9),
        Color::srgb(0.95, 0.85, 0.2),
    ];
    for (i, colour) in colours.into_iter().enumerate() {
        let i = i as f32;
        commands.spawn((
            Sprite::from_color(colour, Vec2::splat(80.)),
            Transform::from_xyz(-240. + 160. * i, 60. * i - 90., 0.),
            Bounce(Vec2::new(140. + 30. * i, 110. - 20. * i)),
        ));
    }
}

fn bounce(time: Res<Time>, mut blocks: Query<(&mut Transform, &mut Bounce)>) {
    for (mut transform, mut bounce) in &mut blocks {
        transform.translation += (bounce.0 * time.delta_secs()).extend(0.);
        for axis in 0..2 {
            let limit = [360., 220.][axis];
            if transform.translation[axis].abs() > limit {
                bounce.0[axis] = -bounce.0[axis].abs() * transform.translation[axis].signum();
            }
        }
    }
}

fn choose_look(
    keys: Res<ButtonInput<KeyCode>>,
    mut looks: ResMut<Looks>,
    mut cameras: Query<&mut Crt>,
) {
    let count = looks.names.len();
    let shown = if keys.just_pressed(KeyCode::ArrowRight) {
        (looks.shown + 1) % count
    } else if keys.just_pressed(KeyCode::ArrowLeft) {
        (looks.shown + count - 1) % count
    } else {
        return;
    };
    looks.shown = shown;
    info!("{}", looks.names[shown]);
    for mut crt in &mut cameras {
        *crt = Crt::preset(looks.names[shown]).unwrap();
    }
}
