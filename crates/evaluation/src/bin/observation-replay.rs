use anyhow::{Context, Result, ensure};
use chess_evaluation::{
    load_manifest,
    replay::{ObservationTrace, ReplayConfig, replay},
};
fn main() -> Result<()> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    ensure!(
        (2..=3).contains(&args.len()),
        "usage: observation-replay <manifest.json> <observations.json> [config.json]"
    );
    let manifest = load_manifest(&args[0])?;
    let trace: ObservationTrace =
        serde_json::from_slice(&std::fs::read(&args[1])?).context("read observation trace")?;
    let config = args
        .get(2)
        .map(|path| -> Result<ReplayConfig> { Ok(serde_json::from_slice(&std::fs::read(path)?)?) })
        .transpose()?
        .unwrap_or_default();
    println!(
        "{}",
        serde_json::to_string_pretty(&replay(&manifest, &trace, config)?)?
    );
    Ok(())
}
