//! The bezel as the passes draw it: its mesh baked, seen straight on, into three small images,
//! which a full-screen pass steps each pixel's ray across. Hosts that draw only full-screen
//! passes, as RetroArch does, draw it from the same images, so the renderer and its ports read
//! the same numbers.
//!
//! Every face of the bezel the camera can see points back along the view, so its front is one
//! depth for each point across it, and a grid of those depths holds it. Each texel keeps the
//! nearest face's depth and what the mesh interpolates there: normal, UV, shade and reflection.
//! Values that need more than 8 bits are split over two channels, high byte first:
//!
//! | Image   | R          | G          | B          | A          |
//! | ------- | ---------- | ---------- | ---------- | ---------- |
//! | shape   | depth high | depth low  | shade      | reflection |
//! | uv      | u high     | u low      | v high     | v low      |
//! | normal  | y high     | y low      | z high     | z low      |
//!
//! The normal's x is always toward the camera, so it follows from the other two. A texel the
//! bezel does not cover holds the greatest depth, which the passes read as nothing there; its
//! other values are its nearest covered neighbour's, so filtering across an edge stays smooth.

use crate::mesh::{Mesh, Vertex, BEZEL};
use anyhow::Result;
use image::{Rgba, RgbaImage};

/// The bezel's extent across the view, in the mesh's units: y to the left and right, z up.
pub const HALF_WIDTH: f32 = 1.6533334;
pub const HALF_HEIGHT: f32 = 1.32;
/// The depths its faces span, front to back. The shaders step each ray across this range.
pub const NEAR: f32 = -0.1;
pub const FAR: f32 = 0.33;
/// The grid's size: about one texel for each pixel of a 1600×900 render.
pub const SIZE: (u32, u32) = (1024, 818);
/// Half the size of a box around the middle of the view that the bezel never covers. A ray
/// that stays inside it for the bezel's whole depth goes through the opening, so the passes
/// need not step it.
pub const OPENING: (f32, f32) = (1.24, 0.95);

/// The three images, in the order the table above lists them.
pub struct Maps {
    pub shape: RgbaImage,
    pub uv: RgbaImage,
    pub normal: RgbaImage,
}

/// What the nearest face holds at one texel.
#[derive(Clone, Copy)]
struct Texel {
    depth: f32,
    normal: [f32; 3],
    uv: [f32; 2],
    shade: f32,
    reflection: f32,
}

/// Bakes the original bezel.
pub fn maps() -> Result<Maps> {
    Ok(encode(&bake(&Mesh::read(BEZEL)?)))
}

fn bake(mesh: &Mesh) -> Vec<Option<Texel>> {
    let (w, h) = SIZE;
    let mut grid: Vec<Option<Texel>> = vec![None; (w * h) as usize];
    let to_texel = |p: [f32; 3]| {
        (
            (HALF_WIDTH - p[1]) / (2. * HALF_WIDTH) * w as f32 - 0.5,
            (HALF_HEIGHT - p[2]) / (2. * HALF_HEIGHT) * h as f32 - 0.5,
        )
    };
    for face in mesh.indices.as_chunks::<3>().0 {
        let [a, b, c] = [0, 1, 2].map(|i| mesh.vertices[usize::from(face[i])]);
        // Faces the camera sees point back along the view; the others it never sees.
        let normal = cross(sub(b.position, a.position), sub(c.position, a.position));
        if normal[0] >= -1e-6 {
            continue;
        }
        let corners = [a, b, c].map(|v| to_texel(v.position));
        let (min_x, max_x) = span(corners.map(|p| p.0), w);
        let (min_y, max_y) = span(corners.map(|p| p.1), h);
        for ty in min_y..=max_y {
            for tx in min_x..=max_x {
                let Some(weights) = barycentric(corners, (tx as f32, ty as f32)) else {
                    continue;
                };
                let texel = interpolate([a, b, c], weights);
                let slot = &mut grid[(ty * w + tx) as usize];
                if slot.is_none_or(|nearest| texel.depth < nearest.depth) {
                    *slot = Some(texel);
                }
            }
        }
    }
    grid
}

/// The texels a triangle's corners cover along one axis, within the grid.
fn span(values: [f32; 3], size: u32) -> (u32, u32) {
    let low = values
        .iter()
        .copied()
        .fold(f32::INFINITY, f32::min)
        .floor()
        .max(0.);
    let high = values
        .iter()
        .copied()
        .fold(f32::NEG_INFINITY, f32::max)
        .ceil();
    (low as u32, (high as u32).min(size - 1))
}

/// The weights of the three corners at `p`, if it is inside the triangle. Edges count as
/// inside, so neighbouring faces leave no gaps between them.
fn barycentric(c: [(f32, f32); 3], p: (f32, f32)) -> Option<[f32; 3]> {
    let area = (c[1].0 - c[0].0) * (c[2].1 - c[0].1) - (c[2].0 - c[0].0) * (c[1].1 - c[0].1);
    if area.abs() < 1e-12 {
        return None;
    }
    let edge = |i: usize, j: usize| {
        ((c[j].0 - c[i].0) * (p.1 - c[i].1) - (p.0 - c[i].0) * (c[j].1 - c[i].1)) / area
    };
    let weights = [edge(1, 2), edge(2, 0), edge(0, 1)];
    weights.iter().all(|&w| w >= -1e-5).then_some(weights)
}

fn interpolate(v: [Vertex; 3], w: [f32; 3]) -> Texel {
    let mix = |f: fn(&Vertex) -> f32| v.iter().zip(w).map(|(v, w)| f(v) * w).sum::<f32>();
    let normal = normalize([
        mix(|v| v.normal[0]),
        mix(|v| v.normal[1]),
        mix(|v| v.normal[2]),
    ]);
    Texel {
        depth: mix(|v| v.position[0]),
        normal,
        uv: [mix(|v| v.uv[0]), mix(|v| v.uv[1])],
        shade: mix(|v| v.color[0]),
        reflection: mix(|v| v.reflection),
    }
}

/// The grid as the three images, with the uncovered texels given their nearest covered
/// neighbour's values and the greatest depth.
fn encode(grid: &[Option<Texel>]) -> Maps {
    let (w, h) = SIZE;
    let filled = fill(grid);
    let at = |x: u32, y: u32| {
        (
            grid[(y * w + x) as usize].is_some(),
            filled[(y * w + x) as usize],
        )
    };
    let pair = |v: f32, low: f32, high: f32| {
        let steps = ((v - low) / (high - low)).clamp(0., 1.) * 65534.;
        let steps = steps.round() as u16;
        [(steps >> 8) as u8, steps as u8]
    };
    let byte = |v: f32| (v.clamp(0., 1.) * 255.).round() as u8;
    let shape = RgbaImage::from_fn(w, h, |x, y| {
        let (covered, t) = at(x, y);
        let [hi, lo] = if covered {
            pair(t.depth, NEAR, FAR)
        } else {
            [255, 255]
        };
        Rgba([hi, lo, byte(t.shade), byte(t.reflection)])
    });
    let uv = RgbaImage::from_fn(w, h, |x, y| {
        let t = at(x, y).1;
        let [u0, u1] = pair(t.uv[0], 0., 1.);
        let [v0, v1] = pair(t.uv[1], 0., 1.);
        Rgba([u0, u1, v0, v1])
    });
    let normal = RgbaImage::from_fn(w, h, |x, y| {
        let t = at(x, y).1;
        let [y0, y1] = pair(t.normal[1], -1., 1.);
        let [z0, z1] = pair(t.normal[2], -1., 1.);
        Rgba([y0, y1, z0, z1])
    });
    Maps { shape, uv, normal }
}

/// Every texel's values: its own, or those of the nearest covered texel, found by growing
/// the covered area outwards a texel at a time.
fn fill(grid: &[Option<Texel>]) -> Vec<Texel> {
    let (w, h) = (SIZE.0 as usize, SIZE.1 as usize);
    let mut filled = grid.to_vec();
    let mut queue: std::collections::VecDeque<usize> =
        (0..filled.len()).filter(|&i| filled[i].is_some()).collect();
    while let Some(i) = queue.pop_front() {
        let (x, y) = (i % w, i / w);
        let neighbours = [
            (x > 0).then(|| i - 1),
            (x + 1 < w).then(|| i + 1),
            (y > 0).then(|| i - w),
            (y + 1 < h).then(|| i + w),
        ];
        for n in neighbours.into_iter().flatten() {
            if filled[n].is_none() {
                filled[n] = filled[i];
                queue.push_back(n);
            }
        }
    }
    filled
        .into_iter()
        .map(|t| t.expect("the bezel covers some texel"))
        .collect()
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> [f32; 3] {
    let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    if length < 1e-12 {
        return [-1., 0., 0.];
    }
    v.map(|c| c / length)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(pair: [u8; 2], low: f32, high: f32) -> f32 {
        low + f32::from(u16::from_be_bytes(pair)) / 65534. * (high - low)
    }

    #[test]
    fn the_bezel_surrounds_an_opening_and_faces_the_camera() {
        let maps = maps().unwrap();
        for image in [&maps.shape, &maps.uv, &maps.normal] {
            assert_eq!(image.dimensions(), SIZE);
        }
        let (w, h) = SIZE;
        // The middle of the view is the opening the screen shows through.
        assert_eq!(&maps.shape.get_pixel(w / 2, h / 2).0[..2], &[255, 255]);
        // Near the outer edge is the bezel's front, at the front of its depth range.
        let rim = maps.shape.get_pixel(4, h / 2).0;
        let depth = decode([rim[0], rim[1]], NEAR, FAR);
        assert!((NEAR..NEAR + 0.06).contains(&depth), "{depth}");
        let normal = maps.normal.get_pixel(4, h / 2).0;
        let (ny, nz) = (
            decode([normal[0], normal[1]], -1., 1.),
            decode([normal[2], normal[3]], -1., 1.),
        );
        assert!(ny * ny + nz * nz <= 1.0001);
        // The opening's box is clear of the bezel: no covered texel lies within it.
        for (x, y, p) in maps.shape.enumerate_pixels() {
            let (cy, cz) = (
                HALF_WIDTH - (x as f32 + 0.5) / w as f32 * 2. * HALF_WIDTH,
                HALF_HEIGHT - (y as f32 + 0.5) / h as f32 * 2. * HALF_HEIGHT,
            );
            // A texel's neighbours are blended in too, so the box keeps one texel clear.
            let margin = 2. * HALF_WIDTH / w as f32;
            if cy.abs() < OPENING.0 + margin && cz.abs() < OPENING.1 + margin {
                assert_eq!(
                    &p.0[..2],
                    &[255, 255],
                    "texel {x}, {y} is inside the opening"
                );
            }
        }
        let covered = maps.shape.pixels().filter(|p| p[0] != 255 || p[1] != 255);
        let share = covered.count() as f32 / (w * h) as f32;
        assert!((0.2..0.8).contains(&share), "{share}");
    }
}
