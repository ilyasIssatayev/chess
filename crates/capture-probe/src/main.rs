use capture_probe::{Command, help_text, parse_cli, verify_capture_run};

fn main() {
    let command = match parse_cli(std::env::args().skip(1)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("error: {error}");
            std::process::exit(2);
        }
    };

    if command == Command::Help {
        println!("{}", help_text());
        return;
    }

    if let Command::Verify { input } = &command {
        match verify_capture_run(input) {
            Ok(report) => {
                println!(
                    "verified {} frames ({} saved images); timestamp source: {}",
                    report.frame_count, report.saved_frame_count, report.timestamp_source
                );
                return;
            }
            Err(error) => {
                eprintln!("error: {error}");
                std::process::exit(1);
            }
        }
    }

    if let Err(error) = platform::run(command) {
        eprintln!("error: {error}");
        std::process::exit(1);
    }
}

#[cfg(not(target_os = "macos"))]
mod platform {
    use capture_probe::Command;

    pub fn run(_command: Command) -> Result<(), String> {
        Err(format!(
            "camera capture is supported only on macOS; this build targets {}",
            std::env::consts::OS
        ))
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use capture_probe::{
        CaptureTracker, Command, FormatRequest, SampleOptions, TIMESTAMP_SOURCE, json_escape,
        write_ppm,
    };
    use nokhwa::pixel_format::RgbFormat;
    use nokhwa::utils::{
        ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
        Resolution,
    };
    use nokhwa::{Camera, nokhwa_initialize, query};
    use std::fs::{self, File};
    use std::io::{BufWriter, Write};
    use std::path::Path;
    use std::sync::mpsc;
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

    pub fn run(command: Command) -> Result<(), String> {
        match command {
            Command::Help => unreachable!("help is handled before platform dispatch"),
            Command::Verify { .. } => unreachable!("verify is handled before platform dispatch"),
            Command::Permission => {
                request_permission()?;
                println!("camera permission: granted");
                Ok(())
            }
            Command::Devices { json } => {
                request_permission()?;
                list_devices(json)
            }
            Command::Formats { device, json } => {
                request_permission()?;
                list_formats(device, json)
            }
            Command::Sample(options) => {
                request_permission()?;
                sample(options)
            }
        }
    }

    fn request_permission() -> Result<(), String> {
        let (sender, receiver) = mpsc::sync_channel(1);
        nokhwa_initialize(move |granted| {
            let _ = sender.send(granted);
        });
        match receiver.recv_timeout(Duration::from_secs(60)) {
            Ok(true) => Ok(()),
            Ok(false) => Err(concat!(
                "camera permission was denied. Enable it in System Settings > ",
                "Privacy & Security > Camera, then rerun the probe"
            )
            .to_owned()),
            Err(error) => Err(format!(
                "timed out waiting for the macOS camera permission result: {error}"
            )),
        }
    }

    fn list_devices(json: bool) -> Result<(), String> {
        let devices = query(ApiBackend::AVFoundation)
            .map_err(|error| format!("could not enumerate AVFoundation cameras: {error}"))?;
        if json {
            print!("[");
            for (offset, device) in devices.iter().enumerate() {
                if offset > 0 {
                    print!(",");
                }
                print!(
                    concat!(
                        "{{\"index\":\"{}\",\"name\":\"{}\",",
                        "\"description\":\"{}\",\"misc\":\"{}\"}}"
                    ),
                    json_escape(&device.index().as_string()),
                    json_escape(&device.human_name()),
                    json_escape(device.description()),
                    json_escape(&device.misc()),
                );
            }
            println!("]");
        } else if devices.is_empty() {
            println!("No AVFoundation cameras found.");
        } else {
            for device in devices {
                println!("{}: {}", device.index(), device.human_name());
                println!("  {}", device.description());
            }
        }
        Ok(())
    }

    fn list_formats(device: u32, json: bool) -> Result<(), String> {
        let mut camera = create_camera(device, &FormatRequest::Default)?;
        let mut formats = camera
            .compatible_camera_formats()
            .map_err(|error| format!("could not enumerate camera formats: {error}"))?;
        let enumeration_complete = !formats.is_empty();
        if !enumeration_complete {
            // Nokhwa 0.10.11 can return an empty compatibility list through
            // its AVFoundation adapter even after it negotiated a valid mode.
            // Preserve that distinction while still reporting the usable mode.
            formats.push(camera.camera_format());
        }
        formats.sort_by_key(|format| {
            (
                format.width(),
                format.height(),
                format.frame_rate(),
                format.format(),
            )
        });

        if json {
            print!("[");
            for (offset, format) in formats.iter().enumerate() {
                if offset > 0 {
                    print!(",");
                }
                print!(
                    concat!(
                        "{{\"width\":{},\"height\":{},\"fps\":{},",
                        "\"fourcc\":\"{}\",\"source\":\"{}\"}}"
                    ),
                    format.width(),
                    format.height(),
                    format.frame_rate(),
                    json_escape(&format.format().to_string()),
                    if enumeration_complete {
                        "backend_enumeration"
                    } else {
                        "negotiated_fallback"
                    },
                );
            }
            println!("]");
        } else {
            if !enumeration_complete {
                println!(
                    "Camera {device} returned no compatibility list; showing its negotiated fallback mode:"
                );
            }
            println!("Formats for camera {device}:");
            for format in formats {
                println!(
                    "  {}x{} @ {} fps ({})",
                    format.width(),
                    format.height(),
                    format.frame_rate(),
                    format.format()
                );
            }
        }
        Ok(())
    }

    fn sample(options: SampleOptions) -> Result<(), String> {
        prepare_output_directory(&options.output)?;
        let frames_dir = options.output.join("frames");
        if options.save_every > 0 {
            fs::create_dir_all(&frames_dir).map_err(|error| {
                format!(
                    "could not create frame directory {}: {error}",
                    frames_dir.display()
                )
            })?;
        }

        let mut camera = create_camera(options.device, &options.format)?;
        let device_name = camera.info().human_name();
        let actual_format = camera.camera_format();
        camera
            .open_stream()
            .map_err(|error| format!("could not open camera stream: {error}"))?;

        let started_wall_ns = unix_time_ns()?;
        let origin = Instant::now();
        let mut tracker = CaptureTracker::new(actual_format.frame_rate());
        let jsonl_path = options.output.join("frames.jsonl");
        let jsonl_file = File::create(&jsonl_path)
            .map_err(|error| format!("could not create {}: {error}", jsonl_path.display()))?;
        let mut jsonl = BufWriter::new(jsonl_file);

        println!(
            "capturing {} frames from {} at {}x{} @ {} fps ({})",
            options.frames,
            device_name,
            actual_format.width(),
            actual_format.height(),
            actual_format.frame_rate(),
            actual_format.format()
        );

        for sequence in 0..options.frames {
            let call_started = origin.elapsed();
            let frame = camera
                .frame()
                .map_err(|error| format!("frame {sequence} capture failed: {error}"))?;
            let received = origin.elapsed();
            let received_wall_ns = unix_time_ns()?;
            let raw_bytes = frame.buffer().len();

            let should_save = options.save_every > 0 && sequence % options.save_every == 0;
            let (decoded_bytes, saved_path) = if should_save {
                let decoded = frame
                    .decode_image::<RgbFormat>()
                    .map_err(|error| format!("frame {sequence} RGB decode failed: {error}"))?;
                let name = format!("frame-{sequence:06}.ppm");
                let path = frames_dir.join(&name);
                let file = File::create(&path)
                    .map_err(|error| format!("could not create {}: {error}", path.display()))?;
                write_ppm(
                    BufWriter::new(file),
                    decoded.width(),
                    decoded.height(),
                    decoded.as_raw(),
                )
                .map_err(|error| format!("could not write {}: {error}", path.display()))?;
                (decoded.len(), Some(format!("frames/{name}")))
            } else {
                (0, None)
            };

            let sample = tracker.record(
                duration_ns_u64(call_started),
                duration_ns_u64(received),
                received_wall_ns,
                raw_bytes,
                decoded_bytes,
                saved_path,
            );
            writeln!(jsonl, "{}", sample.to_json_line())
                .map_err(|error| format!("could not write {}: {error}", jsonl_path.display()))?;

            if options.interval_ms > 0 {
                std::thread::sleep(Duration::from_millis(options.interval_ms));
            }
        }

        camera.stop_stream().map_err(|error| {
            format!("captured frames but could not stop camera stream: {error}")
        })?;
        jsonl
            .flush()
            .map_err(|error| format!("could not flush {}: {error}", jsonl_path.display()))?;

        let summary = tracker.summary();
        let manifest = manifest_json(
            options.device,
            &device_name,
            &actual_format,
            started_wall_ns,
            &summary.to_json(),
        );
        let manifest_path = options.output.join("manifest.json");
        fs::write(&manifest_path, manifest)
            .map_err(|error| format!("could not write {}: {error}", manifest_path.display()))?;

        println!(
            "wrote {} frames to {}",
            summary.frame_count,
            jsonl_path.display()
        );
        println!("report: {}", manifest_path.display());
        if let Some(fps) = summary.observed_fps {
            println!("observed receipt rate: {fps:.2} fps");
        }
        println!("detected receipt gaps: {}", summary.gap_count);
        Ok(())
    }

    fn create_camera(device: u32, request: &FormatRequest) -> Result<Camera, String> {
        Camera::with_backend(
            CameraIndex::Index(device),
            requested_format(request)?,
            ApiBackend::AVFoundation,
        )
        .map_err(|error| format!("could not initialize camera {device}: {error}"))
    }

    fn requested_format(request: &FormatRequest) -> Result<RequestedFormat<'static>, String> {
        let request_type = match request {
            // Nokhwa's `None` request currently falls back to 640x480 YUYV on
            // AVFoundation. The built-in MacBook camera may not expose that
            // exact mode, so select a format advertised by the device.
            FormatRequest::Default => RequestedFormatType::AbsoluteHighestFrameRate,
            FormatRequest::HighestFrameRate => RequestedFormatType::AbsoluteHighestFrameRate,
            FormatRequest::HighestResolution => RequestedFormatType::AbsoluteHighestResolution,
            FormatRequest::Exact {
                width,
                height,
                fps,
                fourcc,
            } => {
                let format = fourcc.parse::<FrameFormat>().map_err(|error| {
                    format!("unsupported FOURCC {fourcc}: {error}; run `capture-probe formats`")
                })?;
                RequestedFormatType::Exact(CameraFormat::new(
                    Resolution::new(*width, *height),
                    format,
                    *fps,
                ))
            }
        };
        Ok(RequestedFormat::new::<RgbFormat>(request_type))
    }

    fn manifest_json(
        device: u32,
        device_name: &str,
        format: &CameraFormat,
        started_wall_ns: u128,
        summary: &str,
    ) -> String {
        format!(
            concat!(
                "{{\n",
                "  \"schema_version\": 1,\n",
                "  \"probe_version\": \"{}\",\n",
                "  \"backend\": \"AVFoundation via nokhwa 0.10.11\",\n",
                "  \"target_arch\": \"{}\",\n",
                "  \"device_index\": {},\n",
                "  \"device_name\": \"{}\",\n",
                "  \"started_unix_ns\": {},\n",
                "  \"timestamp_source\": \"{}\",\n",
                "  \"timestamp_note\": \"Receipt time after Camera::frame; not native sample PTS\",\n",
                "  \"format\": {{\"width\":{},\"height\":{},\"fps\":{},\"fourcc\":\"{}\"}},\n",
                "  \"summary\": {}\n",
                "}}\n"
            ),
            env!("CARGO_PKG_VERSION"),
            std::env::consts::ARCH,
            device,
            json_escape(device_name),
            started_wall_ns,
            TIMESTAMP_SOURCE,
            format.width(),
            format.height(),
            format.frame_rate(),
            json_escape(&format.format().to_string()),
            indent_json(summary, 2),
        )
    }

    fn prepare_output_directory(path: &Path) -> Result<(), String> {
        if path.exists() {
            let mut entries = fs::read_dir(path).map_err(|error| {
                format!(
                    "could not inspect output directory {}: {error}",
                    path.display()
                )
            })?;
            if entries.next().is_some() {
                return Err(format!(
                    "output directory {} is not empty; choose a new run directory to preserve prior evidence",
                    path.display()
                ));
            }
        }
        fs::create_dir_all(path).map_err(|error| {
            format!(
                "could not create output directory {}: {error}",
                path.display()
            )
        })
    }

    fn indent_json(value: &str, spaces: usize) -> String {
        let prefix = " ".repeat(spaces);
        value.replace('\n', &format!("\n{prefix}"))
    }

    fn unix_time_ns() -> Result<u128, String> {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .map_err(|error| format!("system clock is before Unix epoch: {error}"))
    }

    fn duration_ns_u64(duration: Duration) -> u64 {
        u64::try_from(duration.as_nanos()).unwrap_or(u64::MAX)
    }
}
