//! CPU graph execution and pinned asset loading are kept behind this adapter.
use crate::preprocess::{self, Corners, Crop, Geometry, RgbaFrame};
use anyhow::{Context, Result, ensure};
use contracts::SquareEvidence;
use ort::{
    session::{Session, builder::GraphOptimizationLevel},
    value::Tensor,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::OnceLock,
    time::Instant,
};

#[derive(Deserialize)]
struct Asset {
    file: String,
    sha256: String,
    bytes: u64,
}
#[derive(Deserialize)]
struct Models {
    version: String,
    models: Vec<Asset>,
    features: Features,
}
#[derive(Deserialize)]
struct Features {
    tensor: String,
    dimension: usize,
}
#[derive(Deserialize)]
struct Native {
    version: String,
    platform: String,
    files: Vec<Asset>,
}
static INITIALIZED: OnceLock<Result<(), String>> = OnceLock::new();

pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn read_verified(path: &Path, asset: &Asset) -> Result<Vec<u8>> {
    ensure!(
        std::fs::metadata(path)
            .with_context(|| format!(
                "Missing asset {}. Run the vision installers.",
                path.display()
            ))?
            .len()
            == asset.bytes,
        "Asset size mismatch: {}",
        asset.file
    );
    let bytes = std::fs::read(path)?;
    ensure!(
        sha256(&bytes) == asset.sha256,
        "Asset hash mismatch: {}",
        asset.file
    );
    Ok(bytes)
}

pub struct NativeRecognizer {
    occupancy: Session,
    pieces: Session,
    model_version: String,
    runtime_version: String,
    feature_tensor: String,
}
#[derive(Debug, Serialize)]
pub struct GraphResult {
    pub logits: Vec<Vec<f32>>,
    pub probabilities: Vec<Vec<f64>>,
    pub features: Vec<Vec<f32>>,
}
#[derive(Debug, Serialize)]
pub struct Recognition {
    pub model_version: String,
    pub runtime_version: String,
    pub provider: &'static str,
    pub width: usize,
    pub height: usize,
    pub latency_ms: u128,
    pub squares: Vec<SquareEvidence>,
    pub coverage: Vec<f64>,
    pub quality: preprocess::ViewQuality,
    pub piece_indices: Vec<usize>,
    pub occupancy: GraphResult,
    pub pieces: GraphResult,
}
impl NativeRecognizer {
    pub fn load(assets: &Path) -> Result<Self> {
        let models: Models =
            serde_json::from_str(include_str!("../../../models/vision-manifest.json"))?;
        let native: Native =
            serde_json::from_str(include_str!("../../../models/native-runtime.json"))?;
        ensure!(
            cfg!(all(target_os = "macos", target_arch = "aarch64"))
                && native.platform == "macos-arm64",
            "Native runtime has only been pinned for macOS arm64."
        );
        ensure!(
            models.models.len() == 2 && models.features.dimension == 1024,
            "Unsupported model manifest."
        );
        let occupancy = read_verified(
            &assets.join("models").join(&models.models[0].file),
            &models.models[0],
        )?;
        let pieces = read_verified(
            &assets.join("models").join(&models.models[1].file),
            &models.models[1],
        )?;
        let library = native
            .files
            .first()
            .context("Native runtime manifest contains no library.")?;
        let path = assets.join("native").join(&library.file);
        read_verified(&path, library)?;
        let path = path.canonicalize()?;
        INITIALIZED
            .get_or_init(|| {
                ort::init_from(path.to_string_lossy())
                    .with_name("chess-native-cpu")
                    .commit()
                    .map_err(|e| e.to_string())
                    .and_then(|new| {
                        if new {
                            Ok(())
                        } else {
                            Err("ONNX Runtime was initialized outside the pinned adapter.".into())
                        }
                    })
            })
            .as_ref()
            .map_err(|e| anyhow::anyhow!(e.clone()))?;
        let session = |bytes: &[u8]| -> Result<Session> {
            Ok(Session::builder()?
                .with_intra_threads(1)?
                .with_inter_threads(1)?
                .with_optimization_level(GraphOptimizationLevel::Level3)?
                .commit_from_memory(bytes)?)
        };
        Ok(Self {
            occupancy: session(&occupancy)?,
            pieces: session(&pieces)?,
            model_version: models.version,
            runtime_version: native.version,
            feature_tensor: models.features.tensor,
        })
    }
    pub fn recognize(
        &mut self,
        frame: &RgbaFrame,
        corners: Corners,
        inspect_all: bool,
    ) -> Result<Recognition> {
        let start = Instant::now();
        let geometry = Geometry::new(corners, frame.width, frame.height)?;
        let warped = preprocess::warp(frame, &geometry, 50, 500)?;
        let crops: Vec<_> = (0..64)
            .map(|i| preprocess::occupancy_crop(&warped, i / 8, i % 8))
            .collect::<Result<_>>()?;
        let occupancy = run(&mut self.occupancy, &crops, 100, 2, &self.feature_tensor)?;
        let occupied: Vec<_> = occupancy.probabilities.iter().map(|p| p[1]).collect();
        let piece_indices: Vec<_> = (0..64)
            .filter(|&i| inspect_all || occupied[i] >= 0.10)
            .collect();
        let mut probabilities = vec![None; 64];
        let mut coverage: Vec<_> = crops.iter().map(|c| c.coverage).collect();
        let warped = preprocess::warp(frame, &geometry, 200, 800)?;
        let crops: Vec<_> = piece_indices
            .iter()
            .map(|&i| preprocess::piece_crop(&warped, i / 8, i % 8))
            .collect::<Result<_>>()?;
        let pieces = run(&mut self.pieces, &crops, 200, 12, &self.feature_tensor)?;
        for (i, &index) in piece_indices.iter().enumerate() {
            probabilities[index] = Some(pieces.probabilities[i].clone());
            coverage[index] = coverage[index].min(crops[i].coverage);
        }
        let squares =
            preprocess::evidence(&geometry.squares, &occupied, &probabilities, &coverage)?;
        Ok(Recognition {
            model_version: self.model_version.clone(),
            runtime_version: self.runtime_version.clone(),
            provider: "CPUExecutionProvider",
            width: frame.width,
            height: frame.height,
            latency_ms: start.elapsed().as_millis(),
            squares,
            coverage,
            quality: geometry.quality(),
            piece_indices,
            occupancy,
            pieces,
        })
    }
}
pub fn default_assets() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../local-data/vision")
}
fn run(
    session: &mut Session,
    crops: &[Crop],
    height: usize,
    count: usize,
    feature_name: &str,
) -> Result<GraphResult> {
    let mut result = GraphResult {
        logits: Vec::new(),
        probabilities: Vec::new(),
        features: Vec::new(),
    };
    for batch in crops.chunks(8) {
        let input = Tensor::from_array((
            [batch.len(), 3, height, 100],
            preprocess::tensor(batch, 100, height)?.into_boxed_slice(),
        ))?;
        let outputs = session.run(ort::inputs! {"input"=>input})?;
        let (shape, logits) = outputs
            .get("logits")
            .context("Missing logits output.")?
            .try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [batch.len() as i64, count as i64],
            "Unexpected logits shape."
        );
        let (shape, features) = outputs
            .get(feature_name)
            .context("Missing neural features.")?
            .try_extract_tensor::<f32>()?;
        ensure!(
            shape.as_ref() == [batch.len() as i64, 1024] && features.iter().all(|v| v.is_finite()),
            "Invalid feature output."
        );
        for i in 0..batch.len() {
            let logits = logits[i * count..(i + 1) * count].to_vec();
            result.probabilities.push(preprocess::softmax(&logits)?);
            result.logits.push(logits);
            result
                .features
                .push(features[i * 1024..(i + 1) * 1024].to_vec());
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn changed_or_truncated_assets_fail_before_runtime_loading() {
        let path = std::env::temp_dir().join(format!(
            "chess-native-hash-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        struct Cleanup(PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _cleanup = Cleanup(path.clone());
        let asset = Asset {
            file: "test.onnx".into(),
            bytes: 3,
            sha256: "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad".into(),
        };
        std::fs::write(&path, b"abc").unwrap();
        assert_eq!(read_verified(&path, &asset).unwrap(), b"abc");
        std::fs::write(&path, b"abd").unwrap();
        assert!(
            read_verified(&path, &asset)
                .unwrap_err()
                .to_string()
                .contains("hash mismatch")
        );
        std::fs::write(&path, b"a").unwrap();
        assert!(
            read_verified(&path, &asset)
                .unwrap_err()
                .to_string()
                .contains("size mismatch")
        );
    }
}
