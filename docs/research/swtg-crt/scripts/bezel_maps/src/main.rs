//! Writes the three bezel images `crtsim_core::bezel::maps` bakes -- shape, uv, normal -- as
//! PNGs into OUTDIR. usage: bezel_maps OUTDIR
fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).expect("an output folder"));
    std::fs::create_dir_all(&out)?;
    let maps = crtsim_core::bezel::maps()?;
    maps.shape.save(out.join("shape.png"))?;
    maps.uv.save(out.join("uv.png"))?;
    maps.normal.save(out.join("normal.png"))?;
    Ok(())
}
