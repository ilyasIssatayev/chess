//! Hardware-independent support for the camera feasibility probe.
//!
//! The executable's AVFoundation adapter is macOS-only. Argument parsing,
//! timestamp health calculations, JSONL output, and PPM encoding live here so
//! they can be tested without opening a camera.

use std::fmt::Write as _;
use std::io::{self, Write};
use std::path::PathBuf;

pub const TIMESTAMP_SOURCE: &str = "process_monotonic_after_blocking_capture";

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Command {
    Help,
    Permission,
    Devices { json: bool },
    Formats { device: u32, json: bool },
    Sample(SampleOptions),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FormatRequest {
    Default,
    HighestFrameRate,
    HighestResolution,
    Exact {
        width: u32,
        height: u32,
        fps: u32,
        fourcc: String,
    },
}

impl FormatRequest {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "default" => Ok(Self::Default),
            "highest-fps" => Ok(Self::HighestFrameRate),
            "highest-resolution" => Ok(Self::HighestResolution),
            _ => parse_exact_format(value),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SampleOptions {
    pub device: u32,
    pub frames: usize,
    pub interval_ms: u64,
    pub save_every: usize,
    pub output: PathBuf,
    pub format: FormatRequest,
}

impl Default for SampleOptions {
    fn default() -> Self {
        Self {
            device: 0,
            frames: 120,
            interval_ms: 0,
            save_every: 30,
            output: PathBuf::from("camera-probe-output"),
            format: FormatRequest::HighestFrameRate,
        }
    }
}

pub fn parse_cli<I>(args: I) -> Result<Command, String>
where
    I: IntoIterator<Item = String>,
{
    let mut args = args.into_iter();
    let Some(command) = args.next() else {
        return Ok(Command::Help);
    };

    match command.as_str() {
        "help" | "-h" | "--help" => {
            reject_extra(args)?;
            Ok(Command::Help)
        }
        "permission" => {
            reject_extra(args)?;
            Ok(Command::Permission)
        }
        "devices" => {
            let mut json = false;
            for arg in args {
                match arg.as_str() {
                    "--json" => json = true,
                    _ => return Err(format!("unknown devices option: {arg}")),
                }
            }
            Ok(Command::Devices { json })
        }
        "formats" => {
            let mut device = 0;
            let mut json = false;
            let mut rest = args.peekable();
            while let Some(arg) = rest.next() {
                match arg.as_str() {
                    "--device" => device = parse_value(&mut rest, "--device")?,
                    "--json" => json = true,
                    _ => return Err(format!("unknown formats option: {arg}")),
                }
            }
            Ok(Command::Formats { device, json })
        }
        "sample" => parse_sample(args).map(Command::Sample),
        _ => Err(format!("unknown command: {command}\n\n{}", help_text())),
    }
}

fn parse_sample<I>(args: I) -> Result<SampleOptions, String>
where
    I: IntoIterator<Item = String>,
{
    let mut options = SampleOptions::default();
    let mut rest = args.into_iter().peekable();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--device" => options.device = parse_value(&mut rest, "--device")?,
            "--frames" => {
                options.frames = parse_value(&mut rest, "--frames")?;
                if options.frames == 0 {
                    return Err("--frames must be at least 1".to_owned());
                }
            }
            "--interval-ms" => options.interval_ms = parse_value(&mut rest, "--interval-ms")?,
            "--save-every" => options.save_every = parse_value(&mut rest, "--save-every")?,
            "--output" => {
                options.output = PathBuf::from(next_value(&mut rest, "--output")?);
            }
            "--format" => {
                let value = next_value(&mut rest, "--format")?;
                options.format = FormatRequest::parse(&value)?;
            }
            _ => return Err(format!("unknown sample option: {arg}")),
        }
    }
    Ok(options)
}

fn parse_value<T, I>(args: &mut I, option: &str) -> Result<T, String>
where
    T: std::str::FromStr,
    T::Err: std::fmt::Display,
    I: Iterator<Item = String>,
{
    let value = next_value(args, option)?;
    value
        .parse()
        .map_err(|error| format!("invalid value for {option}: {value} ({error})"))
}

fn next_value<I>(args: &mut I, option: &str) -> Result<String, String>
where
    I: Iterator<Item = String>,
{
    args.next()
        .ok_or_else(|| format!("{option} requires a value"))
}

fn reject_extra<I>(mut args: I) -> Result<(), String>
where
    I: Iterator<Item = String>,
{
    match args.next() {
        Some(arg) => Err(format!("unexpected argument: {arg}")),
        None => Ok(()),
    }
}

fn parse_exact_format(value: &str) -> Result<FormatRequest, String> {
    let (geometry, fourcc) = value.rsplit_once(':').ok_or_else(|| {
        "format must be default, highest-fps, highest-resolution, or WIDTHxHEIGHT@FPS:FOURCC"
            .to_owned()
    })?;
    let (size, fps) = geometry
        .rsplit_once('@')
        .ok_or_else(|| "exact format is missing @FPS".to_owned())?;
    let (width, height) = size
        .split_once('x')
        .ok_or_else(|| "exact format is missing WIDTHxHEIGHT".to_owned())?;

    let width = positive_u32(width, "width")?;
    let height = positive_u32(height, "height")?;
    let fps = positive_u32(fps, "fps")?;
    if fourcc.is_empty() {
        return Err("exact format is missing FOURCC".to_owned());
    }

    Ok(FormatRequest::Exact {
        width,
        height,
        fps,
        fourcc: fourcc.to_ascii_uppercase(),
    })
}

fn positive_u32(value: &str, name: &str) -> Result<u32, String> {
    let parsed = value
        .parse::<u32>()
        .map_err(|error| format!("invalid {name}: {value} ({error})"))?;
    if parsed == 0 {
        Err(format!("{name} must be greater than zero"))
    } else {
        Ok(parsed)
    }
}

pub fn help_text() -> &'static str {
    r#"capture-probe — MacBook camera feasibility probe

USAGE:
  capture-probe permission
  capture-probe devices [--json]
  capture-probe formats [--device INDEX] [--json]
  capture-probe sample [OPTIONS]

SAMPLE OPTIONS:
  --device INDEX        Camera index (default: 0)
  --frames COUNT        Frames to capture (default: 120)
  --interval-ms MS      Minimum delay after each capture (default: 0)
  --save-every COUNT    Save every Nth RGB frame as PPM; 0 disables (default: 30)
  --output DIRECTORY    Metadata/frame directory (default: camera-probe-output)
  --format REQUEST      default | highest-fps | highest-resolution |
                        WIDTHxHEIGHT@FPS:FOURCC

The sample command writes manifest.json, frames.jsonl and optional frame-*.ppm files.
Frame times are process-side receipt timestamps; they are not AVFoundation sample PTS."#
}

#[derive(Clone, Debug, PartialEq)]
pub struct FrameSample {
    pub sequence: u64,
    pub capture_started_mono_ns: u64,
    pub received_mono_ns: u64,
    pub received_unix_ns: u128,
    pub capture_block_ns: u64,
    pub interarrival_ns: Option<u64>,
    pub raw_bytes: usize,
    pub decoded_rgb_bytes: usize,
    pub saved_path: Option<String>,
}

impl FrameSample {
    pub fn to_json_line(&self) -> String {
        let interarrival = self
            .interarrival_ns
            .map(|value| value.to_string())
            .unwrap_or_else(|| "null".to_owned());
        let saved_path = self
            .saved_path
            .as_deref()
            .map(|value| format!("\"{}\"", json_escape(value)))
            .unwrap_or_else(|| "null".to_owned());
        format!(
            concat!(
                "{{\"sequence\":{},",
                "\"capture_started_mono_ns\":{},",
                "\"received_mono_ns\":{},",
                "\"received_unix_ns\":{},",
                "\"capture_block_ns\":{},",
                "\"interarrival_ns\":{},",
                "\"raw_bytes\":{},",
                "\"decoded_rgb_bytes\":{},",
                "\"saved_path\":{}}}"
            ),
            self.sequence,
            self.capture_started_mono_ns,
            self.received_mono_ns,
            self.received_unix_ns,
            self.capture_block_ns,
            interarrival,
            self.raw_bytes,
            self.decoded_rgb_bytes,
            saved_path
        )
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct CaptureSummary {
    pub frame_count: u64,
    pub first_received_mono_ns: Option<u64>,
    pub last_received_mono_ns: Option<u64>,
    pub elapsed_ns: u64,
    pub observed_fps: Option<f64>,
    pub max_interarrival_ns: Option<u64>,
    pub mean_capture_block_ns: Option<f64>,
    pub gap_count: u64,
    pub gap_threshold_ns: Option<u64>,
}

impl CaptureSummary {
    pub fn to_json(&self) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"frame_count\": {},\n",
                "  \"first_received_mono_ns\": {},\n",
                "  \"last_received_mono_ns\": {},\n",
                "  \"elapsed_ns\": {},\n",
                "  \"observed_fps\": {},\n",
                "  \"max_interarrival_ns\": {},\n",
                "  \"mean_capture_block_ns\": {},\n",
                "  \"gap_count\": {},\n",
                "  \"gap_threshold_ns\": {}\n",
                "}}"
            ),
            self.frame_count,
            json_option_integer(self.first_received_mono_ns),
            json_option_integer(self.last_received_mono_ns),
            self.elapsed_ns,
            json_option_float(self.observed_fps),
            json_option_integer(self.max_interarrival_ns),
            json_option_float(self.mean_capture_block_ns),
            self.gap_count,
            json_option_integer(self.gap_threshold_ns),
        )
    }
}

#[derive(Clone, Debug)]
pub struct CaptureTracker {
    expected_frame_ns: Option<u64>,
    first_received_mono_ns: Option<u64>,
    previous_received_mono_ns: Option<u64>,
    last_received_mono_ns: Option<u64>,
    max_interarrival_ns: Option<u64>,
    capture_block_total_ns: u128,
    frame_count: u64,
    gap_count: u64,
}

impl CaptureTracker {
    pub fn new(expected_fps: u32) -> Self {
        Self {
            expected_frame_ns: (expected_fps > 0)
                .then(|| 1_000_000_000_u64 / u64::from(expected_fps)),
            first_received_mono_ns: None,
            previous_received_mono_ns: None,
            last_received_mono_ns: None,
            max_interarrival_ns: None,
            capture_block_total_ns: 0,
            frame_count: 0,
            gap_count: 0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record(
        &mut self,
        capture_started_mono_ns: u64,
        received_mono_ns: u64,
        received_unix_ns: u128,
        raw_bytes: usize,
        decoded_rgb_bytes: usize,
        saved_path: Option<String>,
    ) -> FrameSample {
        let capture_block_ns = received_mono_ns.saturating_sub(capture_started_mono_ns);
        let interarrival_ns = self
            .previous_received_mono_ns
            .map(|previous| received_mono_ns.saturating_sub(previous));
        self.first_received_mono_ns.get_or_insert(received_mono_ns);
        self.last_received_mono_ns = Some(received_mono_ns);
        self.previous_received_mono_ns = Some(received_mono_ns);
        self.max_interarrival_ns = match (self.max_interarrival_ns, interarrival_ns) {
            (Some(current), Some(next)) => Some(current.max(next)),
            (None, Some(next)) => Some(next),
            (current, None) => current,
        };
        self.capture_block_total_ns += u128::from(capture_block_ns);
        if let (Some(interval), Some(expected)) = (interarrival_ns, self.expected_frame_ns)
            && interval > gap_threshold(expected)
        {
            self.gap_count += 1;
        }

        let sample = FrameSample {
            sequence: self.frame_count,
            capture_started_mono_ns,
            received_mono_ns,
            received_unix_ns,
            capture_block_ns,
            interarrival_ns,
            raw_bytes,
            decoded_rgb_bytes,
            saved_path,
        };
        self.frame_count += 1;
        sample
    }

    pub fn summary(&self) -> CaptureSummary {
        let elapsed_ns = match (self.first_received_mono_ns, self.last_received_mono_ns) {
            (Some(first), Some(last)) => last.saturating_sub(first),
            _ => 0,
        };
        let observed_fps = (self.frame_count > 1 && elapsed_ns > 0)
            .then(|| (self.frame_count - 1) as f64 * 1_000_000_000.0 / elapsed_ns as f64);
        let mean_capture_block_ns = (self.frame_count > 0)
            .then(|| self.capture_block_total_ns as f64 / self.frame_count as f64);

        CaptureSummary {
            frame_count: self.frame_count,
            first_received_mono_ns: self.first_received_mono_ns,
            last_received_mono_ns: self.last_received_mono_ns,
            elapsed_ns,
            observed_fps,
            max_interarrival_ns: self.max_interarrival_ns,
            mean_capture_block_ns,
            gap_count: self.gap_count,
            gap_threshold_ns: self.expected_frame_ns.map(gap_threshold),
        }
    }
}

fn gap_threshold(expected_frame_ns: u64) -> u64 {
    expected_frame_ns.saturating_mul(5) / 2
}

pub fn write_ppm<W: Write>(mut writer: W, width: u32, height: u32, rgb: &[u8]) -> io::Result<()> {
    let expected = usize::try_from(width)
        .ok()
        .and_then(|width| {
            usize::try_from(height)
                .ok()
                .and_then(|height| width.checked_mul(height))
        })
        .and_then(|pixels| pixels.checked_mul(3))
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "image dimensions overflow"))?;
    if rgb.len() != expected {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("RGB buffer has {} bytes; expected {expected}", rgb.len()),
        ));
    }
    write!(writer, "P6\n{width} {height}\n255\n")?;
    writer.write_all(rgb)
}

pub fn json_escape(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '\"' => escaped.push_str("\\\""),
            '\\' => escaped.push_str("\\\\"),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            control if control <= '\u{1f}' => {
                let _ = write!(escaped, "\\u{:04x}", control as u32);
            }
            ordinary => escaped.push(ordinary),
        }
    }
    escaped
}

fn json_option_integer<T: ToString>(value: Option<T>) -> String {
    value
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_owned())
}

fn json_option_float(value: Option<f64>) -> String {
    value
        .filter(|value| value.is_finite())
        .map(|value| format!("{value:.6}"))
        .unwrap_or_else(|| "null".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn parses_sample_options() {
        let command = parse_cli(args(&[
            "sample",
            "--device",
            "2",
            "--frames",
            "3",
            "--interval-ms",
            "40",
            "--save-every",
            "1",
            "--output",
            "probe data",
            "--format",
            "1920x1080@30:NV12",
        ]))
        .unwrap();

        assert_eq!(
            command,
            Command::Sample(SampleOptions {
                device: 2,
                frames: 3,
                interval_ms: 40,
                save_every: 1,
                output: PathBuf::from("probe data"),
                format: FormatRequest::Exact {
                    width: 1920,
                    height: 1080,
                    fps: 30,
                    fourcc: "NV12".to_owned(),
                },
            })
        );
    }

    #[test]
    fn rejects_zero_frames() {
        let error = parse_cli(args(&["sample", "--frames", "0"])).unwrap_err();
        assert_eq!(error, "--frames must be at least 1");
    }

    #[test]
    fn rejects_malformed_exact_format() {
        let error = FormatRequest::parse("1920x1080:NV12").unwrap_err();
        assert_eq!(error, "exact format is missing @FPS");
    }

    #[test]
    fn tracker_calculates_rate_and_gaps() {
        let mut tracker = CaptureTracker::new(10);
        tracker.record(5, 10, 100, 20, 30, None);
        tracker.record(90_000_000, 100_000_010, 200, 20, 30, None);
        tracker.record(390_000_000, 400_000_010, 300, 20, 30, None);

        let summary = tracker.summary();
        assert_eq!(summary.frame_count, 3);
        assert_eq!(summary.elapsed_ns, 400_000_000);
        assert_eq!(summary.observed_fps, Some(5.0));
        assert_eq!(summary.max_interarrival_ns, Some(300_000_000));
        assert_eq!(summary.gap_threshold_ns, Some(250_000_000));
        assert_eq!(summary.gap_count, 1);
    }

    #[test]
    fn ppm_writer_validates_size_and_writes_header() {
        let mut output = Vec::new();
        write_ppm(&mut output, 1, 1, &[1, 2, 3]).unwrap();
        assert_eq!(output, b"P6\n1 1\n255\n\x01\x02\x03");

        let error = write_ppm(Vec::new(), 2, 1, &[1, 2, 3]).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn json_escaping_covers_metadata_paths() {
        assert_eq!(json_escape("a\"b\\c\n\t"), "a\\\"b\\\\c\\n\\t");
    }

    #[test]
    fn frame_sample_is_valid_json_shape() {
        let sample = FrameSample {
            sequence: 0,
            capture_started_mono_ns: 1,
            received_mono_ns: 2,
            received_unix_ns: 3,
            capture_block_ns: 1,
            interarrival_ns: None,
            raw_bytes: 4,
            decoded_rgb_bytes: 5,
            saved_path: Some("frames/frame-000000.ppm".to_owned()),
        };
        let json = sample.to_json_line();
        assert!(json.starts_with('{'));
        assert!(json.ends_with('}'));
        assert!(json.contains("\"interarrival_ns\":null"));
        assert!(json.contains("\"saved_path\":\"frames/frame-000000.ppm\""));
    }
}
