//! Native camera + bounded latest-frame processing. No database writes occur here.
use anyhow::{Context, Result, ensure};
use contracts::{CaptureTimeUs, FrameObservation, SessionId};
use image::{ExtendedColorType, codecs::jpeg::JpegEncoder};
use serde::{Deserialize, Serialize};
use std::{
    collections::VecDeque,
    io::{BufRead, BufReader, Read},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use vision_inference::{
    preprocess::{Corners, Geometry, RgbaFrame},
    runtime::NativeRecognizer,
};

#[derive(Deserialize, Debug)]
#[serde(deny_unknown_fields)]
struct Header {
    version: u32,
    sequence: u64,
    capture_time_us: i64,
    width: usize,
    height: usize,
    bytes: usize,
    dropped: u64,
    timestamp_source: String,
}
#[derive(Clone)]
struct Frame {
    image: RgbaFrame,
    time: CaptureTimeUs,
    sequence: u64,
    dropped: u64,
    acquired: Instant,
}
fn read_frame(reader: &mut impl BufRead) -> Result<Frame> {
    let mut line = String::new();
    ensure!(
        reader.take(1024).read_line(&mut line)? > 0 && line.ends_with('\n'),
        "Camera stopped or sent an invalid frame header"
    );
    let h: Header = serde_json::from_str(&line)?;
    ensure!(
        h.version == 1 && h.timestamp_source == "avfoundation_presentation_time",
        "Unsupported camera timestamp protocol"
    );
    ensure!(
        (2..=1920).contains(&h.width)
            && (2..=1080).contains(&h.height)
            && h.bytes == h.width * h.height * 4,
        "Invalid camera frame size"
    );
    let mut bgra = vec![0; h.bytes];
    reader.read_exact(&mut bgra)?;
    for p in bgra.as_chunks_mut::<4>().0.iter_mut() {
        p.swap(0, 2);
    }
    Ok(Frame {
        image: RgbaFrame::new(h.width, h.height, bgra)?,
        time: CaptureTimeUs::new(h.capture_time_us)?,
        sequence: h.sequence,
        dropped: h.dropped,
        acquired: Instant::now(),
    })
}
#[derive(Clone)]
pub struct Reading {
    pub observation: FrameObservation,
    pub jpeg: Vec<u8>,
    pub latency_ms: u128,
    pub source_sequence: u64,
    pub dropped: u64,
    pub acquired: Instant,
}
#[derive(Clone, Serialize)]
pub struct Health {
    pub state: String,
    pub error: Option<String>,
    pub frames: u64,
    pub overwritten: u64,
    pub camera_dropped: u64,
    pub inference_ms: u128,
    pub capture_time_us: i64,
    pub timestamp_source: &'static str,
    pub memory_frames_limit: usize,
}
struct Shared {
    frame: Option<Frame>,
    readings: VecDeque<Reading>,
    preview: Vec<u8>,
    health: Health,
    overflow: bool,
    last_capture: Option<Instant>,
}
pub struct NativeCamera {
    shared: Arc<Mutex<Shared>>,
    stop: Arc<AtomicBool>,
    child: Child,
    threads: Vec<thread::JoinHandle<()>>,
    pub id: SessionId,
}
impl NativeCamera {
    pub fn start(helper: &Path, assets: &Path, corners: Corners, threshold: f64) -> Result<Self> {
        ensure!(
            threshold.is_finite() && (1.0..=40.0).contains(&threshold),
            "Invalid motion threshold"
        );
        Geometry::new(corners, 1280, 720)?;
        // Verify assets before opening the camera. Recognizer moves into the worker.
        let mut recognizer = NativeRecognizer::load(assets)?;
        let mut child = Command::new(helper)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .context("Build the native camera helper with scripts/build-native-camera.sh")?;
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();
        let id = SessionId::new();
        let stop = Arc::new(AtomicBool::new(false));
        let shared = Arc::new(Mutex::new(Shared {
            frame: None,
            readings: VecDeque::new(),
            preview: vec![],
            overflow: false,
            last_capture: None,
            health: Health {
                state: "starting".into(),
                error: None,
                frames: 0,
                overwritten: 0,
                camera_dropped: 0,
                inference_ms: 0,
                capture_time_us: 0,
                timestamp_source: "avfoundation_presentation_time",
                memory_frames_limit: 18,
            },
        }));
        let mut threads = Vec::new();
        let state = shared.clone();
        let quitting = stop.clone();
        threads.push(thread::spawn(move || {
            let mut reader = BufReader::new(stdout); let mut last: Option<(u64, i64)> = None;
            while !quitting.load(Ordering::Relaxed) {
                match read_frame(&mut reader) {
                    Ok(frame) => {
                        let mut s = state.lock().unwrap();
                        if last.is_some_and(|(seq,time)| frame.sequence <= seq || frame.time.get() <= time || frame.time.get() - time > 2_000_000) {
                            s.health.state = "interrupted".into(); s.health.error = Some("Capture clock discontinuity. Restart camera and verify the physical position.".into()); break;
                        }
                        last = Some((frame.sequence, frame.time.get()));
                        s.last_capture=Some(frame.acquired);
                        s.health.frames += 1; s.health.camera_dropped = frame.dropped;
                        s.health.capture_time_us = frame.time.get();
                        if s.frame.is_some() { s.health.overwritten += 1; }
                        s.frame = Some(frame);
                    }
                    Err(e) => { let mut s = state.lock().unwrap(); s.health.state = "interrupted".into(); s.health.error = Some(e.to_string()); break; }
                }
            }
        }));
        let state = shared.clone();
        threads.push(thread::spawn(move || {
            let mut message = String::new();
            let _ = stderr.take(4096).read_to_string(&mut message);
            if !message.trim().is_empty() {
                let mut s = state.lock().unwrap();
                s.health.error = Some(message.trim().into());
                s.health.state = "interrupted".into();
            }
        }));
        let state = shared.clone();
        let quitting = stop.clone();
        threads.push(thread::spawn(move || {
            let mut previous: Option<Vec<f64>> = None;
            let mut stable: Option<i64> = None;
            let mut index = 0;
            while !quitting.load(Ordering::Relaxed) {
                let frame = {
                    let mut s = state.lock().unwrap();
                    if s.health.state == "interrupted" {
                        None
                    } else {
                        s.frame.take()
                    }
                };
                let Some(frame) = frame else {
                    thread::sleep(Duration::from_millis(20));
                    continue;
                };
                let result = (|| -> Result<Reading> {
                    let geometry = Geometry::new(corners, frame.image.width, frame.image.height)?;
                    let patch = motion_patch(&frame.image, &geometry);
                    let moving = previous
                        .as_ref()
                        .is_some_and(|p| motion(p, &patch) > threshold);
                    previous = Some(patch);
                    if moving {
                        stable = None;
                    } else {
                        stable.get_or_insert(frame.time.get());
                    }
                    let moving = stable.is_none_or(|t| frame.time.get() - t < 900_000);
                    let result = recognizer.recognize(&frame.image, corners, false)?;
                    let jpeg = jpeg(&frame.image)?;
                    let observation = FrameObservation {
                        session_id: id,
                        sequence: index,
                        capture_time: frame.time,
                        moving,
                        calibration_version: serde_json::to_string(&corners)?,
                        model_version: result.model_version,
                        squares: result.squares,
                    };
                    index += 1;
                    Ok(Reading {
                        observation,
                        jpeg,
                        latency_ms: result.latency_ms,
                        source_sequence: frame.sequence,
                        dropped: frame.dropped,
                        acquired: frame.acquired,
                    })
                })();
                let mut s = state.lock().unwrap();
                match result {
                    Ok(reading) => {
                        if s.health.state == "interrupted" {
                            continue;
                        }
                        s.health.inference_ms = reading.latency_ms;
                        s.health.state = "live".into();
                        s.preview = reading.jpeg.clone();
                        if s.readings.len() == 16 {
                            s.readings.pop_front();
                            s.overflow = true;
                        }
                        s.readings.push_back(reading);
                    }
                    Err(e) => {
                        s.health.state = "interrupted".into();
                        s.health.error = Some(e.to_string());
                    }
                }
            }
        }));
        Ok(Self {
            shared,
            stop,
            child,
            threads,
            id,
        })
    }
    pub fn health(&self) -> Health {
        let s = self.shared.lock().unwrap();
        let mut health = s.health.clone();
        if s.last_capture
            .is_some_and(|t| t.elapsed() > Duration::from_secs(2))
            && health.state == "live"
        {
            health.state = "interrupted".into();
            health.error = Some(
                "No recent camera frames. Restart capture and verify a fresh reference.".into(),
            );
        }
        health
    }
    pub fn preview(&self) -> Vec<u8> {
        self.shared.lock().unwrap().preview.clone()
    }
    pub fn drain(&self) -> (Vec<Reading>, bool) {
        let mut s = self.shared.lock().unwrap();
        let overflow = std::mem::take(&mut s.overflow);
        (s.readings.drain(..).collect(), overflow)
    }
}
impl Drop for NativeCamera {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.child.kill();
        let _ = self.child.wait();
        for t in self.threads.drain(..) {
            let _ = t.join();
        }
    }
}
fn motion_patch(frame: &RgbaFrame, geometry: &Geometry) -> Vec<f64> {
    (0..64 * 16)
        .map(|i| {
            let square = i / 16;
            let point = geometry.project(
                ((square % 8) as f64 + ((i % 4) as f64 + 0.5) / 4.0) / 8.0,
                ((square / 8) as f64 + (((i % 16) / 4) as f64 + 0.5) / 4.0) / 8.0,
            );
            let x = (point.x as usize).min(frame.width - 1);
            let y = (point.y as usize).min(frame.height - 1);
            let p = &frame.rgba[(y * frame.width + x) * 4..];
            0.299 * p[0] as f64 + 0.587 * p[1] as f64 + 0.114 * p[2] as f64
        })
        .collect()
}
fn motion(a: &[f64], b: &[f64]) -> f64 {
    // Remove a global brightness shift; maximum square change catches localized hands.
    let offset = a.iter().zip(b).map(|(a, b)| b - a).sum::<f64>() / a.len() as f64;
    a.as_chunks::<16>()
        .0
        .iter()
        .zip(b.as_chunks::<16>().0.iter())
        .map(|(a, b)| {
            a.iter()
                .zip(b)
                .map(|(a, b)| (b - a - offset).abs())
                .sum::<f64>()
                / 16.0
        })
        .fold(0.0, f64::max)
}
pub fn jpeg(frame: &RgbaFrame) -> Result<Vec<u8>> {
    let rgb: Vec<_> = frame
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .flat_map(|p| p[..3].iter().copied())
        .collect();
    let mut data = vec![];
    JpegEncoder::new_with_quality(&mut data, 85).encode(
        &rgb,
        frame.width as u32,
        frame.height as u32,
        ExtendedColorType::Rgb8,
    )?;
    Ok(data)
}
#[cfg(test)]
impl NativeCamera {
    pub(crate) fn fixture(id: SessionId) -> Self {
        Self {
            shared: Arc::new(Mutex::new(Shared {
                frame: None,
                readings: VecDeque::new(),
                preview: vec![],
                overflow: false,
                last_capture: Some(Instant::now()),
                health: Health {
                    state: "live".into(),
                    error: None,
                    frames: 1,
                    overwritten: 0,
                    camera_dropped: 0,
                    inference_ms: 0,
                    capture_time_us: 0,
                    timestamp_source: "avfoundation_presentation_time",
                    memory_frames_limit: 18,
                },
            })),
            stop: Arc::new(AtomicBool::new(false)),
            child: Command::new("/usr/bin/true").spawn().unwrap(),
            threads: vec![],
            id,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn protocol_rejects_oversize_bad_clocks_and_truncation() {
        for line in [
            r#"{"version":1,"sequence":0,"capture_time_us":0,"width":99999,"height":720,"bytes":0,"dropped":0,"timestamp_source":"avfoundation_presentation_time"}"#,
            r#"{"version":1,"sequence":0,"capture_time_us":-1,"width":2,"height":2,"bytes":16,"dropped":0,"timestamp_source":"host"}"#,
        ] {
            assert!(read_frame(&mut std::io::Cursor::new(format!("{line}\n"))).is_err());
        }
        let mut data = b"{\"version\":1,\"sequence\":0,\"capture_time_us\":123,\"width\":2,\"height\":2,\"bytes\":16,\"dropped\":0,\"timestamp_source\":\"avfoundation_presentation_time\"}\n".to_vec();
        assert!(read_frame(&mut std::io::Cursor::new(&data)).is_err());
        data.extend([1, 2, 3, 255].repeat(4));
        let frame = read_frame(&mut std::io::Cursor::new(data)).unwrap();
        assert_eq!(&frame.image.rgba[..4], &[3, 2, 1, 255]);
        assert_eq!(frame.time.get(), 123);
    }
    #[test]
    fn global_lighting_is_not_local_motion() {
        let a = vec![50.0; 1024];
        let mut b = vec![65.0; 1024];
        assert_eq!(motion(&a, &b), 0.0);
        b[..16].fill(200.0);
        assert!(motion(&a, &b) > 100.0);
    }
}
