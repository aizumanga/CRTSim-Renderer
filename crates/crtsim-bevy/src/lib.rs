//! J. Kyle Pittman's CRTSim in Bevy: each camera holding a [`Crt`] has its picture drawn
//! through the CRT, by `crtsim-core`'s own renderer on Bevy's own device, every frame.
//!
//! ```no_run
//! use bevy::prelude::*;
//! use crtsim_bevy::{Crt, CrtPlugin};
//!
//! App::new()
//!     .add_plugins((DefaultPlugins, CrtPlugin))
//!     .add_systems(Startup, |mut commands: Commands| {
//!         commands.spawn((Camera2d, Crt::preset("NTSC 480i").unwrap()));
//!     })
//!     .run();
//! ```
//!
//! The CRT draws after Bevy's own post-processing, and after the camera's interface with the
//! `ui` feature, so a game's HUD is on the tube too. Each camera is a host sequence: its glow
//! decays by how long each frame lasted, a new window size goes on with the glow, and a new
//! signal size or colour mode starts it again. The look's output size is the camera's.

use bevy::{
    app::{App, Plugin},
    camera::Camera,
    core_pipeline::{
        schedule::{Core2d, Core2dSystems, Core3d, Core3dSystems},
        upscaling::upscaling,
    },
    ecs::{
        component::Component,
        query::{QueryItem, With},
        resource::Resource,
        schedule::IntoScheduleConfigs as _,
        system::{lifetimeless::Read, Commands, Query, Res, ResMut},
    },
    log::error,
    render::{
        extract_component::{ExtractComponent, ExtractComponentPlugin},
        renderer::{RenderAdapterInfo, RenderContext, RenderDevice, RenderQueue, ViewQuery},
        sync_component::SyncComponent,
        sync_world::{MainEntity, MainEntityHashMap, MainEntityHashSet},
        view::ViewTarget,
        Extract, ExtractSchedule, Render, RenderApp, RenderStartup, RenderSystems,
    },
    time::Time,
};
use crtsim_core::{config::Config, HostFrame, Renderer, Sequence};
use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
};

/// Draws the picture of every camera holding a [`Crt`] through the CRT.
pub struct CrtPlugin;

/// The look a camera's picture is drawn through the CRT with: an included preset, or any
/// settings the app or the command line saved.
#[derive(Component, Clone, Debug)]
pub struct Crt {
    pub config: Config,
}

impl Crt {
    /// The included preset called `name`, whatever its case, as the app's gallery and
    /// `crtsim render --preset` name them.
    pub fn preset(name: &str) -> Option<Self> {
        crtsim_core::presets::named(name).map(|preset| Self::new(preset.config))
    }

    pub fn new(config: Config) -> Self {
        Self { config }
    }

    /// A look saved as JSON, as the app or the command line saves it.
    pub fn from_json(json: &[u8]) -> anyhow::Result<Self> {
        Config::from_json_slice(json).map(Self::new)
    }
}

impl SyncComponent<RenderApp> for Crt {
    type Target = Self;
}

impl ExtractComponent<RenderApp> for Crt {
    type QueryData = Read<Crt>;
    type QueryFilter = With<Camera>;
    type Out = Self;

    fn extract_component(crt: QueryItem<'_, '_, Self::QueryData>) -> Option<Self> {
        Some(crt.clone())
    }
}

impl Plugin for CrtPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractComponentPlugin::<Crt>::default());
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<Sequences>()
            .init_resource::<FrameTime>()
            .add_systems(RenderStartup, start_renderer)
            .add_systems(ExtractSchedule, extract_frame_time)
            .add_systems(
                Render,
                (finish_renderer, forget_cameras).in_set(RenderSystems::Prepare),
            );
        #[cfg(feature = "ui")]
        render_app
            .add_systems(
                Core2d,
                crt.after(Core2dSystems::PostProcess)
                    .after(bevy::ui_render::ui_pass)
                    .before(upscaling),
            )
            .add_systems(
                Core3d,
                crt.after(Core3dSystems::PostProcess)
                    .after(bevy::ui_render::ui_pass)
                    .before(upscaling),
            );
        #[cfg(not(feature = "ui"))]
        render_app
            .add_systems(
                Core2d,
                crt.after(Core2dSystems::PostProcess).before(upscaling),
            )
            .add_systems(
                Core3d,
                crt.after(Core3dSystems::PostProcess).before(upscaling),
            );
    }
}

type Starting = Pin<Box<dyn Future<Output = anyhow::Result<Renderer>> + Send>>;

/// The renderer being made on Bevy's device, which validates its pipelines and, in a browser,
/// waits for the device to say so.
#[derive(Resource)]
struct StartingRenderer(Mutex<Starting>);

/// The renderer, once made.
#[derive(Resource)]
struct CrtRenderer(Renderer);

/// Each camera's sequence, by its entity in the main world.
#[derive(Resource, Default)]
struct Sequences(MainEntityHashMap<Drawn>);

/// A camera's sequence, and whether it has reported the CRT failing to draw it, so a run of
/// failures is reported once.
struct Drawn {
    sequence: Sequence,
    reported: bool,
}

/// How long the frame before this one lasted, in seconds, which this one's glow decays by.
#[derive(Resource)]
struct FrameTime(f64);

impl Default for FrameTime {
    fn default() -> Self {
        Self(1. / 60.)
    }
}

fn start_renderer(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    adapter: Res<RenderAdapterInfo>,
) {
    let device = Arc::new(device.wgpu_device().clone());
    let queue = Arc::new((**queue).clone());
    let info = (**adapter).clone();
    commands.insert_resource(StartingRenderer(Mutex::new(Box::pin(
        Renderer::with_device(device, queue, info),
    ))));
}

/// Takes the renderer once it is made, polling for it each frame until then.
fn finish_renderer(mut commands: Commands, starting: Option<Res<StartingRenderer>>) {
    let Some(starting) = starting else {
        return;
    };
    let mut future = starting.0.lock().unwrap_or_else(|e| e.into_inner());
    let Some(made) = futures_lite::future::block_on(futures_lite::future::poll_once(&mut *future))
    else {
        return;
    };
    commands.remove_resource::<StartingRenderer>();
    match made {
        Ok(renderer) => commands.insert_resource(CrtRenderer(renderer)),
        Err(e) => error!("The CRT cannot draw on this device: {e:#}"),
    }
}

fn extract_frame_time(mut frame: ResMut<FrameTime>, time: Extract<Res<Time>>) {
    frame.0 = time.delta_secs_f64();
}

/// Forgets the sequences of cameras that no longer have a CRT.
fn forget_cameras(mut sequences: ResMut<Sequences>, cameras: Query<&MainEntity, With<Crt>>) {
    if sequences.0.len() > cameras.iter().len() {
        let kept: MainEntityHashSet = cameras.iter().copied().collect();
        sequences.0.retain(|camera, _| kept.contains(camera));
    }
}

/// Draws the camera's picture through the CRT, from its main texture into the other one,
/// which becomes the main texture. A picture the CRT cannot draw is left as it was.
fn crt(
    view: ViewQuery<(&ViewTarget, &Crt, &MainEntity)>,
    renderer: Option<Res<CrtRenderer>>,
    mut sequences: ResMut<Sequences>,
    time: Res<FrameTime>,
    mut ctx: RenderContext,
) {
    let Some(renderer) = renderer else {
        return;
    };
    let (target, crt, camera) = view.into_inner();
    let drawn = sequences.0.entry(*camera).or_insert_with(|| Drawn {
        sequence: Sequence::host(),
        reported: false,
    });
    let frame = HostFrame {
        source: target.main_texture(),
        target: target.main_texture_other(),
        seconds: time.0,
    };
    match renderer.0.encode(
        ctx.command_encoder(),
        &mut drawn.sequence,
        &frame,
        &crt.config,
    ) {
        // The CRT drew into the other texture, which is now the picture.
        Ok(()) => {
            target.post_process_write();
            drawn.reported = false;
        }
        Err(e) if !drawn.reported => {
            error!("The CRT cannot draw this camera: {e:#}");
            drawn.reported = true;
        }
        Err(_) => {}
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_asks_for_the_renderer_of_its_own_release() {
        let manifest = include_str!("../Cargo.toml");
        let wanted = format!(
            "crtsim-core = {{ path = \"../crtsim-core\", version = \"{}\" }}",
            env!("CARGO_PKG_VERSION")
        );
        assert!(
            manifest.contains(&wanted),
            "crtsim-bevy draws with the crtsim-core of its own release: {wanted}"
        );
    }

    #[test]
    fn a_look_is_an_included_preset_or_saved_settings() {
        let crt = super::Crt::preset("general IMAGE").unwrap();
        let json = serde_json::to_vec(&crt.config).unwrap();
        assert_eq!(super::Crt::from_json(&json).unwrap().config, crt.config);
        assert!(super::Crt::preset("no such look").is_none());
        assert!(super::Crt::from_json(b"{\"persistence\": [2, 0, 0]}").is_err());
    }
}
