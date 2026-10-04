//! Crop geometry and byte/f32 arithmetic ported from vision-core.js.
//! See models/NOTICE.md for the original MIT preprocessing attribution.
use anyhow::{Result, ensure};
use contracts::SquareEvidence;
use serde::{Deserialize, Serialize};

pub const MODEL_TO_CONTRACT: [usize; 12] = [8, 11, 7, 6, 10, 9, 2, 5, 1, 0, 4, 3];
const MEAN: [f64; 3] = [0.485, 0.456, 0.406];
const STD: [f64; 3] = [0.229, 0.224, 0.225];

#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Corners {
    pub a8: Point,
    pub h8: Point,
    pub h1: Point,
    pub a1: Point,
}
#[derive(Clone, Debug)]
pub struct RgbaFrame {
    pub width: usize,
    pub height: usize,
    pub rgba: Vec<u8>,
}
impl RgbaFrame {
    pub fn new(width: usize, height: usize, rgba: Vec<u8>) -> Result<Self> {
        let frame = Self {
            width,
            height,
            rgba,
        };
        frame.validate()?;
        Ok(frame)
    }
    pub fn validate(&self) -> Result<()> {
        ensure!(
            (2..=8192).contains(&self.width) && (2..=8192).contains(&self.height),
            "Unsupported image dimensions."
        );
        let pixels = self
            .width
            .checked_mul(self.height)
            .ok_or_else(|| anyhow::anyhow!("Image dimension overflow."))?;
        ensure!(
            pixels <= 16 * 1024 * 1024 && self.rgba.len() == pixels * 4,
            "Invalid or oversized RGBA frame."
        );
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct Geometry {
    coefficients: [f64; 8],
    pub squares: Vec<String>,
    pub rotation: usize,
}
#[derive(Debug, Serialize)]
pub struct ViewQuality {
    pub min_square_pixels: usize,
    pub compressed: bool,
    pub warning: &'static str,
}
impl Geometry {
    pub fn new(corners: Corners, width: usize, height: usize) -> Result<Self> {
        ensure!(
            (2..=8192).contains(&width) && (2..=8192).contains(&height),
            "Invalid geometry dimensions."
        );
        let mut points: Vec<_> = [corners.a8, corners.h8, corners.h1, corners.a1]
            .into_iter()
            .enumerate()
            .map(|(i, p)| {
                (
                    i,
                    Point {
                        x: p.x * width as f64,
                        y: p.y * height as f64,
                    },
                )
            })
            .collect();
        ensure!(
            points.iter().all(|(_, p)| p.x.is_finite()
                && p.y.is_finite()
                && p.x >= 0.0
                && p.y >= 0.0
                && p.x <= width as f64
                && p.y <= height as f64),
            "Invalid board corners."
        );
        // Stable sorting preserves browser order when y coordinates are tied.
        points.sort_by(|a, b| a.1.y.total_cmp(&b.1.y));
        points[..2].sort_by(|a, b| a.1.x.total_cmp(&b.1.x));
        points[2..].sort_by(|a, b| b.1.x.total_cmp(&a.1.x));
        for i in 0..4 {
            let (a, b, c) = (points[i].1, points[(i + 1) % 4].1, points[(i + 2) % 4].1);
            ensure!(
                (b.x - a.x) * (c.y - b.y) - (b.y - a.y) * (c.x - b.x) > 1e-6,
                "Board must be convex and uncrossed."
            );
        }
        let rotation = points[0].0;
        ensure!(
            points
                .iter()
                .enumerate()
                .all(|(i, (label, _))| *label == (rotation + i) % 4),
            "Board corner labels are reflected."
        );
        let coefficients = solve(
            points
                .iter()
                .map(|(_, p)| *p)
                .collect::<Vec<_>>()
                .try_into()
                .unwrap(),
        )?;
        let squares = (0..64)
            .map(|i| {
                let (row, col) = (i / 8, i % 8);
                let (file, rank) = match rotation {
                    0 => (col, row),
                    1 => (7 - row, col),
                    2 => (7 - col, 7 - row),
                    _ => (row, 7 - col),
                };
                format!("{}{}", b"abcdefgh"[file] as char, 8 - rank)
            })
            .collect();
        Ok(Self {
            coefficients,
            squares,
            rotation,
        })
    }
    pub fn project(&self, u: f64, v: f64) -> Point {
        let h = self.coefficients;
        let z = h[6] * u + h[7] * v + 1.0;
        Point {
            x: (h[0] * u + h[1] * v + h[2]) / z,
            y: (h[3] * u + h[4] * v + h[5]) / z,
        }
    }
    pub fn quality(&self) -> ViewQuality {
        let mut min_height = f64::INFINITY;
        let mut min_width = f64::INFINITY;
        let distance = |a: Point, b: Point| (a.x - b.x).hypot(a.y - b.y);
        for row in 0..8 {
            for col in 0..8 {
                min_height = min_height.min(distance(
                    self.project((col as f64 + 0.5) / 8.0, row as f64 / 8.0),
                    self.project((col as f64 + 0.5) / 8.0, (row + 1) as f64 / 8.0),
                ));
                min_width = min_width.min(distance(
                    self.project(col as f64 / 8.0, (row as f64 + 0.5) / 8.0),
                    self.project((col + 1) as f64 / 8.0, (row as f64 + 0.5) / 8.0),
                ));
            }
        }
        let minimum = min_height.min(min_width);
        let compressed = minimum / min_height.max(min_width) < 0.35;
        ViewQuality {
            min_square_pixels: minimum.round() as usize,
            compressed,
            warning: if minimum < 8.0 {
                "Too few camera pixels per square. Move closer or raise the camera."
            } else if compressed {
                "Shallow side view: neighboring pieces may block the crops. Raise the camera until every piece base is visible."
            } else {
                ""
            },
        }
    }
}
fn solve(points: [Point; 4]) -> Result<[f64; 8]> {
    let mut matrix = [[0.0; 8]; 8];
    let mut values = [0.0; 8];
    for (i, (u, v)) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
        .into_iter()
        .enumerate()
    {
        let Point { x, y } = points[i];
        matrix[i * 2] = [u, v, 1.0, 0.0, 0.0, 0.0, -x * u, -x * v];
        values[i * 2] = x;
        matrix[i * 2 + 1] = [0.0, 0.0, 0.0, u, v, 1.0, -y * u, -y * v];
        values[i * 2 + 1] = y;
    }
    for col in 0..8 {
        let mut pivot = col;
        for row in col + 1..8 {
            if matrix[row][col].abs() > matrix[pivot][col].abs() {
                pivot = row;
            }
        }
        matrix.swap(col, pivot);
        values.swap(col, pivot);
        let divisor = matrix[col][col];
        ensure!(divisor.abs() >= 1e-9, "Degenerate board corners.");
        for value in matrix[col].iter_mut().skip(col) {
            *value /= divisor;
        }
        values[col] /= divisor;
        let pivot_row = matrix[col];
        for (row, entries) in matrix.iter_mut().enumerate() {
            if row != col {
                let factor = entries[col];
                for (value, pivot) in entries.iter_mut().zip(&pivot_row).skip(col) {
                    *value -= factor * pivot;
                }
                values[row] -= factor * values[col];
            }
        }
    }
    ensure!(values.iter().all(|v| v.is_finite()), "Invalid homography.");
    Ok(values)
}

pub struct Warped {
    rgb: Vec<u8>,
    coverage: Vec<u8>,
    size: usize,
}
#[derive(Debug, Clone)]
pub struct Crop {
    pub rgb: Vec<u8>,
    pub coverage: f64,
}
pub fn warp(frame: &RgbaFrame, geometry: &Geometry, margin: usize, size: usize) -> Result<Warped> {
    frame.validate()?;
    ensure!(
        (margin, size) == (50, 500) || (margin, size) == (200, 800),
        "Unsupported crop warp."
    );
    let mut rgb = vec![0; size * size * 3];
    let mut coverage = vec![0; size * size];
    for y in 0..size {
        for x in 0..size {
            let p = geometry.project(
                (x as f64 - margin as f64) / 400.0,
                (y as f64 - margin as f64) / 400.0,
            );
            if !p.x.is_finite()
                || !p.y.is_finite()
                || p.x < 0.0
                || p.y < 0.0
                || p.x >= (frame.width - 1) as f64
                || p.y >= (frame.height - 1) as f64
            {
                continue;
            }
            let (x0, y0) = (p.x.floor() as usize, p.y.floor() as usize);
            let (dx, dy) = (p.x - x0 as f64, p.y - y0 as f64);
            let s = (y0 * frame.width + x0) * 4;
            let o = (y * size + x) * 3;
            for c in 0..3 {
                rgb[o + c] = (f64::from(frame.rgba[s + c]) * (1.0 - dx) * (1.0 - dy)
                    + f64::from(frame.rgba[s + 4 + c]) * dx * (1.0 - dy)
                    + f64::from(frame.rgba[s + frame.width * 4 + c]) * (1.0 - dx) * dy
                    + f64::from(frame.rgba[s + (frame.width + 1) * 4 + c]) * dx * dy)
                    .round() as u8;
            }
            coverage[y * size + x] = 1;
        }
    }
    Ok(Warped {
        rgb,
        coverage,
        size,
    })
}
pub fn occupancy_crop(w: &Warped, row: usize, col: usize) -> Result<Crop> {
    ensure!(
        w.size == 500 && row < 8 && col < 8,
        "Invalid occupancy crop index/warp."
    );
    let (x0, y0) = (
        (50.0 * (col as f64 + 0.5)).trunc() as usize,
        (50.0 * (row as f64 + 0.5)).trunc() as usize,
    );
    let mut rgb = vec![0; 30000];
    let mut visible = 0u32;
    for y in 0..100 {
        let offset = ((y0 + y) * w.size + x0) * 3;
        rgb[y * 300..(y + 1) * 300].copy_from_slice(&w.rgb[offset..offset + 300]);
        if (25..75).contains(&y) {
            for x in 25..75 {
                visible += u32::from(w.coverage[(y0 + y) * w.size + x0 + x]);
            }
        }
    }
    Ok(Crop {
        rgb,
        coverage: f64::from(visible) / 2500.0,
    })
}
pub fn piece_crop(w: &Warped, row: usize, col: usize) -> Result<Crop> {
    ensure!(
        w.size == 800 && row < 8 && col < 8,
        "Invalid piece crop index/warp."
    );
    let height = 1.0 + 2.0 * (7 - row) as f64 / 7.0;
    let left = if col >= 4 {
        0.0
    } else {
        0.25 + 0.75 * (3 - col) as f64 / 3.0
    };
    let right = if col < 4 {
        0.0
    } else {
        0.25 + 0.75 * (col - 4) as f64 / 3.0
    };
    let x1 = (200.0 + 50.0 * (col as f64 - left)).trunc() as usize;
    let x2 = (200.0 + 50.0 * (col as f64 + 1.0 + right)).trunc() as usize;
    let y1 = (200.0 + 50.0 * (row as f64 - height)).trunc() as usize;
    let y2 = (200.0 + 50.0 * (row as f64 + 1.0)).trunc() as usize;
    let mut rgb = vec![0; 60000];
    let mut visible = 0u32;
    for y in 0..y2 - y1 {
        for x in 0..x2 - x1 {
            let sx = if col < 4 { x2 - 1 - x } else { x1 + x };
            let source = ((y1 + y) * w.size + sx) * 3;
            let target = ((200 - (y2 - y1) + y) * 100 + x) * 3;
            rgb[target..target + 3].copy_from_slice(&w.rgb[source..source + 3]);
            visible += u32::from(w.coverage[(y1 + y) * w.size + sx]);
        }
    }
    Ok(Crop {
        rgb,
        coverage: f64::from(visible) / ((x2 - x1) * (y2 - y1)) as f64,
    })
}
pub fn tensor(crops: &[Crop], width: usize, height: usize) -> Result<Vec<f32>> {
    ensure!(
        !crops.is_empty() && crops.len() <= 64 && width == 100 && (height == 100 || height == 200),
        "Invalid tensor batch/shape."
    );
    let n = width * height;
    ensure!(
        crops.iter().all(|c| c.rgb.len() == n * 3),
        "Crop byte count differs from tensor shape."
    );
    let mut out = vec![0.0; crops.len() * 3 * n];
    for (batch, crop) in crops.iter().enumerate() {
        for c in 0..3 {
            for i in 0..n {
                out[batch * 3 * n + c * n + i] =
                    ((f64::from(crop.rgb[i * 3 + c]) / 255.0 - MEAN[c]) / STD[c]) as f32;
            }
        }
    }
    Ok(out)
}
pub fn softmax(logits: &[f32]) -> Result<Vec<f64>> {
    ensure!(
        !logits.is_empty() && logits.iter().all(|n| n.is_finite()),
        "Invalid model logits."
    );
    let peak = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
    let values: Vec<_> = logits
        .iter()
        .map(|v| (f64::from(*v) - peak).exp())
        .collect();
    let sum: f64 = values.iter().sum();
    Ok(values.into_iter().map(|v| v / sum).collect())
}
pub fn evidence(
    squares: &[String],
    occupied: &[f64],
    pieces: &[Option<Vec<f64>>],
    coverage: &[f64],
) -> Result<Vec<SquareEvidence>> {
    ensure!(
        [squares.len(), occupied.len(), pieces.len(), coverage.len()]
            .into_iter()
            .all(|n| n == 64),
        "Evidence must contain 64 squares."
    );
    let mut output = Vec::with_capacity(64);
    for i in 0..64 {
        ensure!(
            occupied[i].is_finite()
                && (0.0..=1.0).contains(&occupied[i])
                && coverage[i].is_finite()
                && (0.0..=1.0).contains(&coverage[i]),
            "Invalid occupancy/coverage."
        );
        let conditional = pieces[i].clone().unwrap_or_else(|| vec![1.0 / 12.0; 12]);
        ensure!(
            conditional.len() == 12
                && conditional
                    .iter()
                    .all(|n| n.is_finite() && (0.0..=1.0).contains(n))
                && (conditional.iter().sum::<f64>() - 1.0).abs() < 1e-6,
            "Invalid conditional piece probabilities."
        );
        let mut probabilities = [0.0f32; 12];
        let empty = 1.0 - occupied[i];
        let mut confidence = empty;
        for (index, target) in MODEL_TO_CONTRACT.iter().enumerate() {
            let probability = occupied[i] * conditional[index];
            probabilities[*target] = probability as f32;
            confidence = confidence.max(probability);
        }
        output.push(SquareEvidence {
            square: squares[i].clone(),
            visible_probability: if coverage[i] >= 0.75 && confidence >= 0.45 {
                1.0
            } else {
                0.0
            },
            empty_probability: empty as f32,
            piece_probabilities: probabilities,
        });
    }
    Ok(output)
}
