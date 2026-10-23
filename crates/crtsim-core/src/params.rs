//! The shader's settings for one tick, and the uniform slots the renderer writes them to.

use crate::config::{ColorMode, Config, Phase};
use bytemuck::{Pod, Zeroable};

/// The shader's settings, laid out as `Params` in `crtsim.wgsl`.
#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub(crate) struct Params {
    size: [f32; 4],
    signal: [f32; 4],
    persistence: [f32; 4],
    geometry: [f32; 4],
    mask: [f32; 4],
    lighting: [f32; 4],
    surface: [f32; 4],
    bezel: [f32; 4],
    light: [f32; 4],
    camera: [f32; 4],
    bloom: [f32; 4],
    processing: [f32; 4],
    backdrop: [f32; 4],
    post: [f32; 4],
    finish: [f32; 4],
    picture: [f32; 4],
}

/// What a tube's aspect makes of the original's 4:3 one, worked out here so that for 4:3 they
/// are exactly the original's, whatever a GPU's division rounds to.
struct Tube {
    /// The glass across, over the original's.
    stretch: f32,
    /// How much wider each side is than the original's, or narrower where it is negative.
    widening: f32,
    /// How much more the view takes in, so a tube wider than an output of `output`'s shape is
    /// framed as a 4:3 one is in an output of the tube's shape; 1 for any other.
    framing: f32,
}

impl Tube {
    fn new(aspect: f32, output: (u32, u32)) -> Self {
        const ORIGINAL: f32 = 4. / 3.;
        let output = output.0 as f32 / output.1 as f32;
        Self {
            stretch: aspect / ORIGINAL,
            widening: aspect - ORIGINAL,
            framing: (aspect / output.max(ORIGINAL)).max(1.),
        }
    }
}

/// The size of `Params`, which each settings slot holds.
pub(crate) const PARAMS_SIZE: Option<std::num::NonZeroU64> =
    std::num::NonZeroU64::new(std::mem::size_of::<Params>() as u64);

impl Params {
    /// Everything but the composite phase and the interlaced field, which `tick` sets.
    pub(crate) fn new(c: &Config, signal: (u32, u32), output: (u32, u32)) -> Self {
        let distance = 1. / (c.fov.to_radians() * 0.5).tan();
        let camera = glam::Vec3::new(-distance, 0., 0.);
        let uv = c.uv_scale(signal);
        let mask = c.mask_repeats.resolve(signal);
        let tube = Tube::new(c.tube_aspect, output);
        let flag = |on: bool| if on { 1. } else { 0. };
        Self {
            size: [
                signal.0 as f32,
                signal.1 as f32,
                output.0 as f32,
                output.1 as f32,
            ],
            signal: [c.sharpness, c.bleed, c.artifacts, 0.5],
            persistence: [
                c.persistence[0],
                c.persistence[1],
                c.persistence[2],
                flag(c.flip_artifacts),
            ],
            geometry: [uv[0], uv[1], c.overscan, c.barrel],
            mask: [mask[0], mask[1], c.mask_brightness, c.mask_opacity],
            lighting: [c.diffuse, c.specular, c.specular_power, c.rim],
            surface: [
                c.dimming,
                c.reflection,
                c.saturation,
                flag(c.mask_antialias),
            ],
            bezel: [
                c.frame_color[0],
                c.frame_color[1],
                c.frame_color[2],
                flag(c.reflection_as_screen),
            ],
            light: [
                c.light_position[0],
                c.light_position[1],
                c.light_position[2],
                tube.stretch,
            ],
            camera: [camera.x, camera.y, camera.z, c.tube_aspect],
            // With the CRT off there is no bloom, as in the game.
            bloom: [
                if c.crt_off { 0. } else { c.bloom },
                c.bloom_power,
                c.bloom_spread,
                tube.widening,
            ],
            processing: [
                flag(c.color_mode == ColorMode::LinearLight),
                flag(c.interlace),
                0.,
                flag(c.screen_only),
            ],
            backdrop: [
                c.backdrop_color[0],
                c.backdrop_color[1],
                c.backdrop_color[2],
                tube.framing,
            ],
            post: [flag(c.bloom_from_mipmaps), flag(c.glow_floor), 0., 0.],
            finish: [
                2_f32.powf(-c.brightness),
                c.fxaa as u32 as f32,
                c.tate.quarters() as f32,
                c.bloom_spread_up,
            ],
            picture: [c.pixel_aspect, c.static_amount, c.static_cycle, 0.],
        }
    }

    /// The composite phase for `tick`, the field it scans when interlaced, and where TV static's
    /// clock stands, in sixtieths of a second, which also draws its noise anew each whole one.
    /// `blending` moves the two phases towards their average as the game's NTSC Blending does:
    /// its `NTSCLerp` is `p + (0.5 - p) * blending` for the tick's phase `p` of 0 or 1.
    pub(crate) fn tick(&mut self, phase: Phase, blending: f32, tick: u64, clock: f64) {
        let (a, b) = (0.5 * blending, 1. - 0.5 * blending);
        self.signal[3] = match phase {
            Phase::Stable => 0.5,
            Phase::A => a,
            Phase::B => b,
            Phase::Alternating if tick.is_multiple_of(2) => a,
            Phase::Alternating => b,
        };
        self.processing[2] = (tick % 2) as f32;
        // Whole sixtieths are exact up to 2^24, over 77 hours.
        self.picture[3] = (clock % f64::from(1 << 24)) as f32;
    }
}

/// Ticks encoded before each submission, which is also where a frame reports progress and
/// notices a cancel.
pub(crate) const BATCH: u32 = 8;

/// A workspace's settings, one slot per tick of a batch and one for the surface passes, each
/// reached by dynamic offset. `write_buffer` takes effect when the batch is submitted, so every
/// tick needs a slot of its own: sharing one would give them all the last tick's settings.
pub(crate) struct Uniforms {
    pub(crate) buffer: wgpu::Buffer,
    stride: u32,
}

/// The surface passes' slot, after the ticks'.
pub(crate) const SURFACE_SLOT: u32 = BATCH;

impl Uniforms {
    pub(crate) fn new(device: &wgpu::Device) -> Self {
        let alignment = device.limits().min_uniform_buffer_offset_alignment;
        let stride = (std::mem::size_of::<Params>() as u32).div_ceil(alignment) * alignment;
        Self {
            buffer: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("settings"),
                size: u64::from(stride * (SURFACE_SLOT + 1)),
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            stride,
        }
    }

    /// Puts `params` in `slot` for the next submission, and returns the slot's offset.
    pub(crate) fn write(&self, queue: &wgpu::Queue, slot: u32, params: &Params) -> u32 {
        let offset = slot * self.stride;
        queue.write_buffer(&self.buffer, offset.into(), bytemuck::bytes_of(params));
        offset
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ntsc_blending_moves_the_two_phases_towards_each_other() {
        let artifact_mix = |phase, ntsc_blending, tick| {
            let c = Config {
                phase,
                ntsc_blending,
                ..Config::default()
            };
            let mut params = Params::new(&c, (256, 224), (320, 240));
            params.tick(c.phase, c.ntsc_blending, tick, 0.);
            params.signal[3]
        };
        // Unblended, the public source's switch between the two patterns.
        assert_eq!(artifact_mix(Phase::A, 0., 0), 0.);
        assert_eq!(artifact_mix(Phase::B, 0., 0), 1.);
        assert_eq!(artifact_mix(Phase::Alternating, 0., 7), 1.);
        // Super Win the Game's 0.35: each tick mixes in half that of the other pattern, as the
        // game's NTSCLerp = p + (0.5 - p) * 0.35 does.
        assert_eq!(artifact_mix(Phase::A, 0.35, 0), 0.175);
        assert_eq!(artifact_mix(Phase::B, 0.35, 0), 0.825);
        assert_eq!(artifact_mix(Phase::Alternating, 0.35, 4), 0.175);
        assert_eq!(artifact_mix(Phase::Alternating, 0.35, 5), 0.825);
        // Fully blended, every tick shows the average, as stable phase always does.
        assert_eq!(artifact_mix(Phase::Alternating, 1., 1), 0.5);
        assert_eq!(artifact_mix(Phase::Stable, 0.35, 0), 0.5);
    }
}
