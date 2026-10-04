use anyhow::{Result, ensure};
use std::path::PathBuf;
use vision_inference::{
    preprocess::Corners,
    read_image,
    runtime::{NativeRecognizer, default_assets},
};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    ensure!(
        args.len() == 2 || (args.len() == 3 && args[2] == "--all-pieces"),
        "Usage: vision-infer IMAGE CORNERS.json [--all-pieces]"
    );
    let corners: Corners = serde_json::from_slice(&std::fs::read(&args[1])?)?;
    let frame = read_image(&PathBuf::from(&args[0]))?;
    let mut recognizer = NativeRecognizer::load(&default_assets())?;
    println!(
        "{}",
        serde_json::to_string(&recognizer.recognize(&frame, corners, args.len() == 3)?)?
    );
    Ok(())
}
