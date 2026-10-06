//! The screen's glass as the surface pass shades it. The pass ray-traces the sphere the
//! original's mesh was cut from, whose UVs and normals the mesh shares to within a rounding;
//! what the mesh adds is its outline, a polygon of 192 edges, and its vertex shading, 15 greys
//! blended across each triangle. Both are baked here into one image across the glass's UVs, so
//! the passes draw the mesh's outline and shading wherever the ray meets the sphere.
//!
//! Texel (i, j) holds the glass at UV (i, j) / (size − 1), so the image spans exactly 0 to 1.
//! Its two values each take two channels, high byte first:
//!
//! | R          | G         | B         | A        |
//! | ---------- | --------- | --------- | -------- |
//! | shade high | shade low | edge high | edge low |
//!
//! The edge is the distance to the outline in the mesh's units, positive inside and held within
//! ±`EDGE`; blended between texels it puts the outline where the polygon has it. The shade
//! outside the outline is its nearest covered texel's, so blending across the edge stays smooth.

use crate::bezel::fill;
use crate::mesh::{Mesh, SCREEN};
use anyhow::Result;
use image::{Rgba, RgbaImage};
use std::collections::HashMap;

/// About one texel for each pixel of the glass at 1280×960.
pub const SIZE: (u32, u32) = (1024, 768);
/// The edge distances the image holds, in the mesh's units: about six texels either side.
pub const EDGE: f32 = 1. / 64.;
/// How far one unit of UV is across the glass in the mesh's units, left to right and down.
const SPAN: [f32; 2] = [8. / 3., 2.];

/// Bakes the original glass.
pub fn map() -> Result<RgbaImage> {
    let mesh = Mesh::read(SCREEN)?;
    let (w, h) = SIZE;
    let to_texel = |uv: [f32; 2]| (uv[0] * (w - 1) as f32, uv[1] * (h - 1) as f32);
    // Each texel takes the face it is deepest inside. Those just outside the outline take the
    // nearest face's shading carried on past its edge, so blending across the edge stays true.
    let mut best: Vec<Option<(f32, f32)>> = vec![None; (w * h) as usize];
    let faces = mesh.indices.as_chunks::<3>().0;
    for face in faces {
        let [a, b, c] = face.map(|i| mesh.vertices[usize::from(i)]);
        let corners = [a, b, c].map(|v| to_texel(v.uv));
        let widen = |v: [f32; 3]| v.map(|x| x + 2.).into_iter().chain(v.map(|x| x - 2.));
        let (min_x, max_x) = span_of(widen(corners.map(|p| p.0)), w);
        let (min_y, max_y) = span_of(widen(corners.map(|p| p.1)), h);
        for ty in min_y..=max_y {
            for tx in min_x..=max_x {
                let Some(weights) = weights(corners, (tx as f32, ty as f32)) else {
                    continue;
                };
                let depth = weights.into_iter().fold(f32::INFINITY, f32::min);
                let value =
                    weights[0] * a.color[0] + weights[1] * b.color[0] + weights[2] * c.color[0];
                let slot = &mut best[(ty * w + tx) as usize];
                if slot.is_none_or(|(d, _)| depth > d) {
                    *slot = Some((depth, value));
                }
            }
        }
    }
    // Edges count as inside, so neighbouring faces leave no gaps between them.
    let covered: Vec<bool> = best
        .iter()
        .map(|b| b.is_some_and(|(d, _)| d >= -1e-5))
        .collect();
    let shade: Vec<Option<f32>> = best.iter().map(|b| b.map(|(_, v)| v)).collect();
    let edge = edge_distances(&mesh, &covered);
    let filled = fill(&shade, SIZE);
    let pair = |v: f32, low: f32, high: f32| {
        let steps = ((v - low) / (high - low)).clamp(0., 1.) * 65534.;
        let steps = steps.round() as u16;
        [(steps >> 8) as u8, steps as u8]
    };
    Ok(RgbaImage::from_fn(w, h, |x, y| {
        let i = (y * w + x) as usize;
        let [s0, s1] = pair(filled[i], 0., 1.);
        let [e0, e1] = pair(edge[i], -EDGE, EDGE);
        Rgba([s0, s1, e0, e1])
    }))
}

/// Each texel's distance to the outline, signed by whether the mesh covers it, within ±EDGE.
/// The outline is the edges only one face has; only texels near one are measured.
fn edge_distances(mesh: &Mesh, covered: &[bool]) -> Vec<f32> {
    let (w, h) = SIZE;
    let mut uses: HashMap<(u16, u16), u32> = HashMap::new();
    for face in mesh.indices.as_chunks::<3>().0 {
        for (a, b) in [(face[0], face[1]), (face[1], face[2]), (face[2], face[0])] {
            *uses.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    let at = |uv: [f32; 2]| [uv[0] * SPAN[0], uv[1] * SPAN[1]];
    let texel_size = [SPAN[0] / (w - 1) as f32, SPAN[1] / (h - 1) as f32];
    let mut nearest = vec![EDGE; (w * h) as usize];
    for (&(a, b), _) in uses.iter().filter(|(_, &n)| n == 1) {
        let [a, b] = [a, b].map(|i| at(mesh.vertices[usize::from(i)].uv));
        let range = |axis: usize, size: u32| {
            let low = (a[axis].min(b[axis]) - EDGE) / texel_size[axis];
            let high = (a[axis].max(b[axis]) + EDGE) / texel_size[axis];
            (
                low.floor().max(0.) as u32,
                (high.ceil() as u32).min(size - 1),
            )
        };
        let (x0, x1) = range(0, w);
        let (y0, y1) = range(1, h);
        for y in y0..=y1 {
            for x in x0..=x1 {
                let p = [x as f32 * texel_size[0], y as f32 * texel_size[1]];
                let i = (y * w + x) as usize;
                nearest[i] = nearest[i].min(to_segment(p, a, b));
            }
        }
    }
    nearest
        .into_iter()
        .zip(covered)
        .map(|(d, &c)| if c { d } else { -d })
        .collect()
}

/// The texels from the least to the greatest of `values`, within the grid.
fn span_of(values: impl Iterator<Item = f32> + Clone, size: u32) -> (u32, u32) {
    let low = values.clone().fold(f32::INFINITY, f32::min).floor().max(0.);
    let high = values.fold(f32::NEG_INFINITY, f32::max).ceil().max(0.);
    (low as u32, (high as u32).min(size - 1))
}

/// The weights of the three corners at `p`, inside the triangle or not.
fn weights(c: [(f32, f32); 3], p: (f32, f32)) -> Option<[f32; 3]> {
    let area = (c[1].0 - c[0].0) * (c[2].1 - c[0].1) - (c[2].0 - c[0].0) * (c[1].1 - c[0].1);
    if area.abs() < 1e-12 {
        return None;
    }
    let edge = |i: usize, j: usize| {
        ((c[j].0 - c[i].0) * (p.1 - c[i].1) - (p.0 - c[i].0) * (c[j].1 - c[i].1)) / area
    };
    Some([edge(1, 2), edge(2, 0), edge(0, 1)])
}

fn to_segment(p: [f32; 2], a: [f32; 2], b: [f32; 2]) -> f32 {
    let (ab, ap) = ([b[0] - a[0], b[1] - a[1]], [p[0] - a[0], p[1] - a[1]]);
    let t = ((ap[0] * ab[0] + ap[1] * ab[1]) / (ab[0] * ab[0] + ab[1] * ab[1])).clamp(0., 1.);
    (ap[0] - ab[0] * t).hypot(ap[1] - ab[1] * t)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The image's two values at a UV, blended between texels as the passes blend them.
    fn read(map: &RgbaImage, uv: [f32; 2]) -> (f32, f32) {
        let (w, h) = SIZE;
        let g = (uv[0] * (w - 1) as f32, uv[1] * (h - 1) as f32);
        let (x, y) = (
            g.0.floor().min((w - 2) as f32),
            g.1.floor().min((h - 2) as f32),
        );
        let f = (g.0 - x, g.1 - y);
        let decode = |dx: u32, dy: u32| {
            let p = map.get_pixel(x as u32 + dx, y as u32 + dy).0;
            let pair = |hi: u8, lo: u8| f32::from(u16::from_be_bytes([hi, lo])) / 65534.;
            (pair(p[0], p[1]), -EDGE + pair(p[2], p[3]) * 2. * EDGE)
        };
        let mix =
            |a: (f32, f32), b: (f32, f32), t: f32| (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t);
        mix(
            mix(decode(0, 0), decode(1, 0), f.0),
            mix(decode(0, 1), decode(1, 1), f.0),
            f.1,
        )
    }

    #[test]
    fn the_glass_has_the_meshs_shading_and_outline() {
        let map = map().unwrap();
        assert_eq!(map.dimensions(), SIZE);
        let mesh = Mesh::read(SCREEN).unwrap();
        let mut boundary = 0;
        for v in &mesh.vertices {
            let (shade, edge) = read(&map, v.uv);
            // Within a step of the vertex's own colour; the corners, where the outline turns
            // most sharply, are furthest from it.
            assert!(
                (shade - v.color[0]).abs() < 1. / 255.,
                "{:?}: {shade}",
                v.uv
            );
            // The outline runs through the mesh's outermost vertices, which lie on it.
            if edge.abs() < 0.0005 {
                boundary += 1;
            } else {
                assert!(edge > 0., "{:?} is inside the glass", v.uv);
            }
        }
        assert_eq!(boundary, 192);
        assert!(read(&map, [0.5, 0.5]).1 >= EDGE * 0.999);
        assert!(read(&map, [0., 0.]).1 <= -EDGE * 0.999);
        // Between two outline vertices, the edge is the straight line joining them.
        let [a, b] = [mesh.vertices[0].uv, mesh.vertices[1].uv];
        let middle = [(a[0] + b[0]) / 2., (a[1] + b[1]) / 2.];
        assert!(read(&map, middle).1.abs() < 0.0005);
    }
}
