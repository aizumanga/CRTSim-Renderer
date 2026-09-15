//! Reader for the two CC0 M3D 2.1 assets. No GPU is needed for inspection.
use anyhow::{bail, ensure, Context, Result};
use bytemuck::{Pod, Zeroable};
use serde::Serialize;
use std::io::{Cursor, Read};

pub const SCREEN: &[u8] = include_bytes!("../../../assets/original-crtsim/screen.m3d");
pub const FRAME: &[u8] = include_bytes!("../../../assets/original-crtsim/frame.m3d");

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod, Zeroable, Serialize)]
pub struct Vertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub color: [f32; 4],
    pub uv: [f32; 2],
    pub reflection: f32,
}

#[derive(Debug, Serialize)]
pub struct StreamInfo {
    pub usage: u32,
    pub stride: u32,
}

#[derive(Debug, Serialize)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u16>,
    pub streams: Vec<StreamInfo>,
}

fn u32le(c: &mut Cursor<&[u8]>) -> Result<u32> {
    let mut b = [0; 4];
    c.read_exact(&mut b)?;
    Ok(u32::from_le_bytes(b))
}
fn f32le(c: &mut Cursor<&[u8]>) -> Result<f32> {
    let v = f32::from_bits(u32le(c)?);
    ensure!(v.is_finite(), "non-finite mesh value");
    Ok(v)
}

impl Mesh {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(bytes);
        let mut header = [0; 8];
        c.read_exact(&mut header).context("truncated M3D header")?;
        ensure!(
            &header[..4] == b".m3d" && header[4..6] == [2, 1],
            "expected M3D 2.1"
        );
        // Bytes 6..8 are padding, not required to be zero in the original files.
        let count = u32le(&mut c)?;
        let nv = u32le(&mut c)? as usize;
        let ni = u32le(&mut c)? as usize;
        ensure!(
            (4..=5).contains(&count)
                && nv > 0
                && nv <= 65536
                && ni <= 1_000_000
                && ni.is_multiple_of(3),
            "invalid mesh counts"
        );
        let mut unused = [0];
        c.read_exact(&mut unused)?;
        ensure!(u32le(&mut c)? == 2, "only 16-bit indices are supported");
        let mut indices = Vec::with_capacity(ni);
        for _ in 0..ni {
            let mut b = [0; 2];
            c.read_exact(&mut b)?;
            let i = u16::from_le_bytes(b);
            ensure!((i as usize) < nv, "index outside vertex array");
            indices.push(i);
        }
        let mut vertices = vec![Vertex::zeroed(); nv];
        let mut streams = Vec::new();
        let mut seen = [false; 7];
        for _ in 0..count {
            let usage = u32le(&mut c)?;
            let stride = u32le(&mut c)?;
            let expected = match usage {
                0 | 1 => 12,
                4 | 6 => 4,
                5 => 8,
                _ => bail!("unsupported stream {usage}"),
            };
            ensure!(
                stride == expected && !seen[usage as usize],
                "invalid/duplicate stream"
            );
            seen[usage as usize] = true;
            streams.push(StreamInfo { usage, stride });
            for v in &mut vertices {
                match usage {
                    0 => v.position = [f32le(&mut c)?, f32le(&mut c)?, f32le(&mut c)?],
                    1 => v.normal = [f32le(&mut c)?, f32le(&mut c)?, f32le(&mut c)?],
                    4 => {
                        let argb = u32le(&mut c)?;
                        v.color = [
                            ((argb >> 16) & 255) as f32 / 255.,
                            ((argb >> 8) & 255) as f32 / 255.,
                            (argb & 255) as f32 / 255.,
                            (argb >> 24) as f32 / 255.,
                        ];
                    }
                    5 => v.uv = [f32le(&mut c)?, f32le(&mut c)?],
                    6 => v.reflection = f32le(&mut c)?,
                    _ => unreachable!(),
                }
            }
        }
        ensure!(
            seen[0] && seen[1] && seen[4] && seen[5],
            "missing required stream"
        );
        ensure!(
            c.position() as usize == bytes.len(),
            "unexpected trailing mesh data"
        );
        Ok(Self {
            vertices,
            indices,
            streams,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn embedded_meshes_keep_all_attributes() {
        let s = Mesh::read(SCREEN).unwrap();
        let f = Mesh::read(FRAME).unwrap();
        assert_eq!(s.vertices.len(), 2401);
        assert_eq!(f.vertices.len(), 1372);
        assert_eq!(s.streams.len(), 4);
        assert_eq!(f.streams.len(), 5);
        assert!(f.vertices.iter().any(|v| v.reflection > 0.1));
        assert!(f.vertices.iter().any(|v| v.reflection < 0.01));
    }
    #[test]
    fn malformed_meshes_are_rejected() {
        assert!(Mesh::read(&SCREEN[..100]).is_err());
        let mut b = SCREEN.to_vec();
        b[0] = 0;
        assert!(Mesh::read(&b).is_err());
        let mut b = SCREEN.to_vec();
        b.push(0);
        assert!(Mesh::read(&b).is_err());
    }
}
