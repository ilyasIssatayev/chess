//! Export the already-decoded RGBA boundary and stages for cross-language checks.
use anyhow::{Result, ensure};
use serde_json::json;
use std::path::Path;
use vision_inference::{
    preprocess::{self, Corners, Geometry},
    read_image,
    runtime::sha256,
};

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 3,
        "Usage: vision-preprocess IMAGE CORNERS.json NEW_OUTPUT_FOLDER"
    );
    let corners: Corners = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let frame = read_image(Path::new(&args[0]))?;
    let geometry = Geometry::new(corners, frame.width, frame.height)?;
    let output = Path::new(&args[2]);
    std::fs::create_dir(output)?;
    std::fs::write(
        output.join("input.json"),
        serde_json::to_vec(
            &json!({"width":frame.width,"height":frame.height,"corners":corners,"rgba":frame.rgba}),
        )?,
    )?;
    let mut report = json!({"schema_version":1,"squares":geometry.squares,"rotation":geometry.rotation,"quality":geometry.quality()});
    for (name, margin, size, height) in [("occupancy", 50, 500, 100), ("pieces", 200, 800, 200)] {
        let warped = preprocess::warp(&frame, &geometry, margin, size)?;
        let crops: Vec<_> = (0..64)
            .map(|i| {
                if name == "occupancy" {
                    preprocess::occupancy_crop(&warped, i / 8, i % 8)
                } else {
                    preprocess::piece_crop(&warped, i / 8, i % 8)
                }
            })
            .collect::<Result<_>>()?;
        let tensors = preprocess::tensor(&crops, 100, height)?;
        let bytes: Vec<_> = tensors.iter().flat_map(|v| v.to_le_bytes()).collect();
        std::fs::write(output.join(format!("{name}.f32")), &bytes)?;
        report[name] = json!({"crop_sha256":crops.iter().map(|c|sha256(&c.rgb)).collect::<Vec<_>>(),"coverage":crops.iter().map(|c|c.coverage).collect::<Vec<_>>(),"tensor_sha256":sha256(&bytes)});
    }
    std::fs::write(
        output.join("stages.json"),
        serde_json::to_vec_pretty(&report)?,
    )?;
    eprintln!("Preprocessing stages written to {}", output.display());
    Ok(())
}
