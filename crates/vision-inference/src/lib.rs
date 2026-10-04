//! Native CPU photo recognition. Preprocessing matches the browser worker's
//! already-decoded RGBA boundary; image decoding/scaling is a caller concern.
pub mod preprocess;
pub mod runtime;

use anyhow::{Result, ensure};
use image::{ImageReader, Limits};
use std::path::Path;

pub fn read_image(path: &Path) -> Result<preprocess::RgbaFrame> {
    ensure!(
        std::fs::metadata(path)?.len() <= 16 * 1024 * 1024,
        "Image file exceeds 16 MiB."
    );
    let mut reader = ImageReader::open(path)?.with_guessed_format()?;
    let mut limits = Limits::default();
    limits.max_image_width = Some(8192);
    limits.max_image_height = Some(8192);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode()?.to_rgba8();
    preprocess::RgbaFrame::new(
        image.width() as usize,
        image.height() as usize,
        image.into_raw(),
    )
}
