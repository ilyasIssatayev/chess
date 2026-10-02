//! Versioned manifests for offline chess-camera evaluation data.
//!
//! The schema deliberately records acquisition time rather than decode or
//! inference time. Validation covers structural and temporal consistency; it
//! does not prove that a UCI sequence is legal from its initial position.

use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs;
use std::path::Path;

pub const SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatasetManifest {
    pub schema_version: u32,
    pub dataset_id: String,
    pub sessions: Vec<AnnotatedSession>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AnnotatedSession {
    /// Unique ID for this manifest entry or continuous clip.
    pub session_id: String,
    /// Stable grouping key shared by every clip derived from one recording or
    /// game. A partition key may occur in only one dataset split.
    pub partition_key: String,
    pub split: DatasetSplit,
    pub media: MediaReference,
    pub timestamp_source: TimestampSource,
    pub frames: Vec<FrameTimestamp>,
    pub board: BoardCalibration,
    pub reference_game: ReferenceGame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DatasetSplit {
    Training,
    Validation,
    Test,
    Qualification,
}

impl fmt::Display for DatasetSplit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = match self {
            Self::Training => "training",
            Self::Validation => "validation",
            Self::Test => "test",
            Self::Qualification => "qualification",
        };
        f.write_str(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MediaReference {
    /// Path or URI interpreted by the offline playback adapter.
    pub uri: String,
    /// SHA-256 of the immutable media bytes when the media has been sealed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    pub width_px: u32,
    pub height_px: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FrameTimestamp {
    pub frame_index: u64,
    /// Monotonic capture/acquisition timestamp normalized to the session epoch.
    pub capture_us: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TimestampSource {
    /// Presentation time supplied directly by the capture backend/device.
    DevicePresentation,
    /// Monotonic host time recorded when the application acquired the frame.
    HostAcquisition,
    /// Presentation time recovered from an imported media container.
    MediaPresentation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardCalibration {
    /// Corners are named from the camera side of the board. The order around
    /// the quadrilateral is near-left, near-right, far-right, far-left.
    pub corners: BoardCorners,
    pub orientation: BoardOrientation,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoardCorners {
    pub near_left: ImagePoint,
    pub near_right: ImagePoint,
    pub far_right: ImagePoint,
    pub far_left: ImagePoint,
}

impl BoardCorners {
    pub fn in_perimeter_order(&self) -> [ImagePoint; 4] {
        [
            self.near_left,
            self.near_right,
            self.far_right,
            self.far_left,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ImagePoint {
    pub x_px: f64,
    pub y_px: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardOrientation {
    A1NearLeft,
    A1NearRight,
    A1FarRight,
    A1FarLeft,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceGame {
    /// `startpos` for the standard initial position, otherwise a FEN string.
    pub initial_position: String,
    pub moves: Vec<ReferenceMove>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReferenceMove {
    /// One-based half-move number.
    pub ply: u32,
    /// Long algebraic UCI form, such as `e2e4` or `a7a8q`.
    pub uci: String,
    pub completion: CompletionAnnotation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CompletionAnnotation {
    /// The physical move completed at some acquisition time in this inclusive
    /// interval. It is not the later time at which software confirmed it.
    Bounded {
        earliest_capture_us: u64,
        latest_capture_us: u64,
    },
    /// Use when the recording cannot support a defensible completion interval.
    Unknown { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    UnsupportedSchema {
        found: u32,
        supported: u32,
    },
    EmptyDatasetId,
    NoSessions,
    EmptySessionId {
        session_index: usize,
    },
    DuplicateSessionId {
        session_id: String,
    },
    EmptyPartitionKey {
        session_id: String,
    },
    PartitionSplitLeakage {
        partition_key: String,
        first_split: DatasetSplit,
        second_split: DatasetSplit,
    },
    EmptyMediaUri {
        session_id: String,
    },
    InvalidMediaDimensions {
        session_id: String,
    },
    MissingQualificationSha256 {
        session_id: String,
    },
    InvalidSha256 {
        session_id: String,
    },
    NoFrames {
        session_id: String,
    },
    FrameIndexNotIncreasing {
        session_id: String,
        previous: u64,
        current: u64,
    },
    FrameTimestampNotIncreasing {
        session_id: String,
        previous: u64,
        current: u64,
    },
    CornerNotFinite {
        session_id: String,
        corner: &'static str,
    },
    CornerOutsideImage {
        session_id: String,
        corner: &'static str,
    },
    InvalidBoardQuadrilateral {
        session_id: String,
    },
    EmptyInitialPosition {
        session_id: String,
    },
    NonSequentialPly {
        session_id: String,
        expected: u32,
        found: u32,
    },
    InvalidUci {
        session_id: String,
        ply: u32,
        uci: String,
    },
    InvalidCompletionBounds {
        session_id: String,
        ply: u32,
        earliest: u64,
        latest: u64,
    },
    CompletionOutsideFrames {
        session_id: String,
        ply: u32,
    },
    NonChronologicalMoveBounds {
        session_id: String,
        ply: u32,
    },
    EmptyUnknownTimingReason {
        session_id: String,
        ply: u32,
    },
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedSchema { found, supported } => {
                write!(
                    f,
                    "unsupported schema version {found}; supported version is {supported}"
                )
            }
            Self::EmptyDatasetId => f.write_str("dataset_id must not be empty"),
            Self::NoSessions => f.write_str("manifest must contain at least one session"),
            Self::EmptySessionId { session_index } => {
                write!(f, "sessions[{session_index}].session_id must not be empty")
            }
            Self::DuplicateSessionId { session_id } => {
                write!(f, "duplicate session_id `{session_id}`")
            }
            Self::EmptyPartitionKey { session_id } => {
                write!(f, "session `{session_id}` has an empty partition_key")
            }
            Self::PartitionSplitLeakage {
                partition_key,
                first_split,
                second_split,
            } => write!(
                f,
                "partition_key `{partition_key}` occurs in both `{first_split}` and `{second_split}` splits"
            ),
            Self::EmptyMediaUri { session_id } => {
                write!(f, "session `{session_id}` has an empty media URI")
            }
            Self::InvalidMediaDimensions { session_id } => {
                write!(f, "session `{session_id}` has zero media width or height")
            }
            Self::MissingQualificationSha256 { session_id } => write!(
                f,
                "qualification session `{session_id}` must seal its media with sha256"
            ),
            Self::InvalidSha256 { session_id } => write!(
                f,
                "session `{session_id}` media sha256 must contain exactly 64 hexadecimal characters"
            ),
            Self::NoFrames { session_id } => {
                write!(
                    f,
                    "session `{session_id}` must contain at least one frame timestamp"
                )
            }
            Self::FrameIndexNotIncreasing {
                session_id,
                previous,
                current,
            } => write!(
                f,
                "session `{session_id}` frame indexes are not strictly increasing: {previous} then {current}"
            ),
            Self::FrameTimestampNotIncreasing {
                session_id,
                previous,
                current,
            } => write!(
                f,
                "session `{session_id}` capture timestamps are not strictly increasing: {previous} then {current}"
            ),
            Self::CornerNotFinite { session_id, corner } => {
                write!(
                    f,
                    "session `{session_id}` board corner `{corner}` is not finite"
                )
            }
            Self::CornerOutsideImage { session_id, corner } => write!(
                f,
                "session `{session_id}` board corner `{corner}` is outside the media dimensions"
            ),
            Self::InvalidBoardQuadrilateral { session_id } => write!(
                f,
                "session `{session_id}` board corners do not form a non-degenerate convex quadrilateral in perimeter order"
            ),
            Self::EmptyInitialPosition { session_id } => {
                write!(f, "session `{session_id}` has an empty initial_position")
            }
            Self::NonSequentialPly {
                session_id,
                expected,
                found,
            } => write!(
                f,
                "session `{session_id}` expected reference ply {expected}, found {found}"
            ),
            Self::InvalidUci {
                session_id,
                ply,
                uci,
            } => write!(
                f,
                "session `{session_id}` reference ply {ply} has invalid UCI `{uci}`"
            ),
            Self::InvalidCompletionBounds {
                session_id,
                ply,
                earliest,
                latest,
            } => write!(
                f,
                "session `{session_id}` reference ply {ply} has inverted completion bounds [{earliest}, {latest}]"
            ),
            Self::CompletionOutsideFrames { session_id, ply } => write!(
                f,
                "session `{session_id}` reference ply {ply} completion bounds extend outside its frame timestamps"
            ),
            Self::NonChronologicalMoveBounds { session_id, ply } => write!(
                f,
                "session `{session_id}` reference ply {ply} has no completion time consistent with prior ply order"
            ),
            Self::EmptyUnknownTimingReason { session_id, ply } => write!(
                f,
                "session `{session_id}` reference ply {ply} marks timing unknown without a reason"
            ),
        }
    }
}

impl DatasetManifest {
    /// Returns every detected validation error so annotation tools can fix a
    /// batch in one pass.
    pub fn validate(&self) -> Result<(), Vec<ValidationError>> {
        let mut errors = Vec::new();

        if self.schema_version != SCHEMA_VERSION {
            errors.push(ValidationError::UnsupportedSchema {
                found: self.schema_version,
                supported: SCHEMA_VERSION,
            });
        }
        if self.dataset_id.trim().is_empty() {
            errors.push(ValidationError::EmptyDatasetId);
        }
        if self.sessions.is_empty() {
            errors.push(ValidationError::NoSessions);
        }

        let mut session_ids = HashSet::new();
        let mut partition_splits: HashMap<&str, DatasetSplit> = HashMap::new();
        for (session_index, session) in self.sessions.iter().enumerate() {
            let display_id = if session.session_id.trim().is_empty() {
                format!("sessions[{session_index}]")
            } else {
                session.session_id.clone()
            };

            if session.session_id.trim().is_empty() {
                errors.push(ValidationError::EmptySessionId { session_index });
            } else if !session_ids.insert(session.session_id.as_str()) {
                errors.push(ValidationError::DuplicateSessionId {
                    session_id: session.session_id.clone(),
                });
            }

            if session.partition_key.trim().is_empty() {
                errors.push(ValidationError::EmptyPartitionKey {
                    session_id: display_id.clone(),
                });
            } else if let Some(first_split) = partition_splits.get(session.partition_key.as_str()) {
                if *first_split != session.split {
                    errors.push(ValidationError::PartitionSplitLeakage {
                        partition_key: session.partition_key.clone(),
                        first_split: *first_split,
                        second_split: session.split,
                    });
                }
            } else {
                partition_splits.insert(session.partition_key.as_str(), session.split);
            }

            validate_session(session, &display_id, &mut errors);
        }

        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors)
        }
    }
}

fn validate_session(
    session: &AnnotatedSession,
    session_id: &str,
    errors: &mut Vec<ValidationError>,
) {
    if session.media.uri.trim().is_empty() {
        errors.push(ValidationError::EmptyMediaUri {
            session_id: session_id.to_owned(),
        });
    }
    if session.media.width_px == 0 || session.media.height_px == 0 {
        errors.push(ValidationError::InvalidMediaDimensions {
            session_id: session_id.to_owned(),
        });
    }
    if session.split == DatasetSplit::Qualification && session.media.sha256.is_none() {
        errors.push(ValidationError::MissingQualificationSha256 {
            session_id: session_id.to_owned(),
        });
    }
    if let Some(hash) = &session.media.sha256
        && (hash.len() != 64 || !hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
    {
        errors.push(ValidationError::InvalidSha256 {
            session_id: session_id.to_owned(),
        });
    }

    let mut timestamp_sequence_valid = !session.frames.is_empty();
    if session.frames.is_empty() {
        errors.push(ValidationError::NoFrames {
            session_id: session_id.to_owned(),
        });
    }
    for pair in session.frames.windows(2) {
        if pair[0].frame_index >= pair[1].frame_index {
            errors.push(ValidationError::FrameIndexNotIncreasing {
                session_id: session_id.to_owned(),
                previous: pair[0].frame_index,
                current: pair[1].frame_index,
            });
        }
        if pair[0].capture_us >= pair[1].capture_us {
            timestamp_sequence_valid = false;
            errors.push(ValidationError::FrameTimestampNotIncreasing {
                session_id: session_id.to_owned(),
                previous: pair[0].capture_us,
                current: pair[1].capture_us,
            });
        }
    }

    validate_board(session, session_id, errors);

    if session.reference_game.initial_position.trim().is_empty() {
        errors.push(ValidationError::EmptyInitialPosition {
            session_id: session_id.to_owned(),
        });
    }

    let frame_range = if timestamp_sequence_valid {
        session
            .frames
            .first()
            .zip(session.frames.last())
            .map(|(first, last)| (first.capture_us, last.capture_us))
    } else {
        None
    };
    let mut earliest_feasible_completion = None;

    for (move_index, reference_move) in session.reference_game.moves.iter().enumerate() {
        let expected_ply = u32::try_from(move_index + 1).unwrap_or(u32::MAX);
        if reference_move.ply != expected_ply {
            errors.push(ValidationError::NonSequentialPly {
                session_id: session_id.to_owned(),
                expected: expected_ply,
                found: reference_move.ply,
            });
        }
        if !is_syntactic_uci(&reference_move.uci) {
            errors.push(ValidationError::InvalidUci {
                session_id: session_id.to_owned(),
                ply: reference_move.ply,
                uci: reference_move.uci.clone(),
            });
        }

        match &reference_move.completion {
            CompletionAnnotation::Bounded {
                earliest_capture_us,
                latest_capture_us,
            } => {
                if earliest_capture_us > latest_capture_us {
                    errors.push(ValidationError::InvalidCompletionBounds {
                        session_id: session_id.to_owned(),
                        ply: reference_move.ply,
                        earliest: *earliest_capture_us,
                        latest: *latest_capture_us,
                    });
                    continue;
                }
                if let Some((first_frame_us, last_frame_us)) = frame_range
                    && (*earliest_capture_us < first_frame_us || *latest_capture_us > last_frame_us)
                {
                    errors.push(ValidationError::CompletionOutsideFrames {
                        session_id: session_id.to_owned(),
                        ply: reference_move.ply,
                    });
                }

                let candidate = earliest_feasible_completion
                    .map_or(*earliest_capture_us, |prior: u64| {
                        prior.max(*earliest_capture_us)
                    });
                if candidate > *latest_capture_us {
                    errors.push(ValidationError::NonChronologicalMoveBounds {
                        session_id: session_id.to_owned(),
                        ply: reference_move.ply,
                    });
                } else {
                    earliest_feasible_completion = Some(candidate);
                }
            }
            CompletionAnnotation::Unknown { reason } => {
                if reason.trim().is_empty() {
                    errors.push(ValidationError::EmptyUnknownTimingReason {
                        session_id: session_id.to_owned(),
                        ply: reference_move.ply,
                    });
                }
                // An unknown time supplies no ordering constraint. Later known
                // bounds must still follow the latest preceding known bound.
            }
        }
    }
}

fn validate_board(session: &AnnotatedSession, session_id: &str, errors: &mut Vec<ValidationError>) {
    let corners = session.board.corners.in_perimeter_order();
    let names = ["near_left", "near_right", "far_right", "far_left"];
    let width = f64::from(session.media.width_px);
    let height = f64::from(session.media.height_px);
    let mut all_finite = true;

    for (corner, name) in corners.iter().zip(names) {
        if !corner.x_px.is_finite() || !corner.y_px.is_finite() {
            all_finite = false;
            errors.push(ValidationError::CornerNotFinite {
                session_id: session_id.to_owned(),
                corner: name,
            });
        } else if corner.x_px < 0.0
            || corner.y_px < 0.0
            || corner.x_px > width
            || corner.y_px > height
        {
            errors.push(ValidationError::CornerOutsideImage {
                session_id: session_id.to_owned(),
                corner: name,
            });
        }
    }

    if all_finite && !is_convex_quadrilateral(corners) {
        errors.push(ValidationError::InvalidBoardQuadrilateral {
            session_id: session_id.to_owned(),
        });
    }
}

fn is_convex_quadrilateral(points: [ImagePoint; 4]) -> bool {
    const EPSILON: f64 = 1.0e-6;
    let mut sign = 0.0_f64;
    for index in 0..4 {
        let a = points[index];
        let b = points[(index + 1) % 4];
        let c = points[(index + 2) % 4];
        let cross = (b.x_px - a.x_px) * (c.y_px - b.y_px) - (b.y_px - a.y_px) * (c.x_px - b.x_px);
        if cross.abs() <= EPSILON {
            return false;
        }
        if sign == 0.0 {
            sign = cross.signum();
        } else if cross.signum() != sign {
            return false;
        }
    }

    let twice_area = points
        .iter()
        .zip(points.iter().cycle().skip(1))
        .take(4)
        .map(|(a, b)| a.x_px * b.y_px - b.x_px * a.y_px)
        .sum::<f64>()
        .abs();
    twice_area > 1.0
}

fn is_syntactic_uci(value: &str) -> bool {
    let bytes = value.as_bytes();
    if bytes.len() != 4 && bytes.len() != 5 {
        return false;
    }
    let square =
        |file: u8, rank: u8| (b'a'..=b'h').contains(&file) && (b'1'..=b'8').contains(&rank);
    if !square(bytes[0], bytes[1]) || !square(bytes[2], bytes[3]) {
        return false;
    }
    if bytes[0..2] == bytes[2..4] {
        return false;
    }
    bytes.len() == 4 || matches!(bytes[4], b'q' | b'r' | b'b' | b'n')
}

#[derive(Debug)]
pub enum LoadError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Validation(Vec<ValidationError>),
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "could not read manifest: {error}"),
            Self::Json(error) => write!(f, "could not parse manifest JSON: {error}"),
            Self::Validation(errors) => {
                write!(f, "manifest has {} validation error(s)", errors.len())
            }
        }
    }
}

impl Error for LoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::Json(error) => Some(error),
            Self::Validation(_) => None,
        }
    }
}

pub fn parse_manifest(json: &str) -> Result<DatasetManifest, LoadError> {
    let manifest: DatasetManifest = serde_json::from_str(json).map_err(LoadError::Json)?;
    manifest.validate().map_err(LoadError::Validation)?;
    Ok(manifest)
}

/// Validates manifests together, including session and partition isolation
/// across files that declare the same dataset ID.
pub fn validate_manifest_collection(
    manifests: &[&DatasetManifest],
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    for manifest in manifests {
        if let Err(manifest_errors) = manifest.validate() {
            errors.extend(manifest_errors);
        }
    }

    let mut session_files: HashMap<(&str, &str), usize> = HashMap::new();
    let mut partition_files: HashMap<(&str, &str), (usize, DatasetSplit)> = HashMap::new();
    for (manifest_index, manifest) in manifests.iter().enumerate() {
        for session in &manifest.sessions {
            let session_key = (manifest.dataset_id.as_str(), session.session_id.as_str());
            if let Some(first_manifest_index) = session_files.get(&session_key) {
                if *first_manifest_index != manifest_index {
                    errors.push(ValidationError::DuplicateSessionId {
                        session_id: session.session_id.clone(),
                    });
                }
            } else {
                session_files.insert(session_key, manifest_index);
            }

            let partition_key = (manifest.dataset_id.as_str(), session.partition_key.as_str());
            if let Some((first_manifest_index, first_split)) = partition_files.get(&partition_key) {
                if *first_manifest_index != manifest_index && *first_split != session.split {
                    errors.push(ValidationError::PartitionSplitLeakage {
                        partition_key: session.partition_key.clone(),
                        first_split: *first_split,
                        second_split: session.split,
                    });
                }
            } else {
                partition_files.insert(partition_key, (manifest_index, session.split));
            }
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

pub fn load_manifest(path: impl AsRef<Path>) -> Result<DatasetManifest, LoadError> {
    let json = fs::read_to_string(path).map_err(LoadError::Io)?;
    parse_manifest(&json)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> DatasetManifest {
        DatasetManifest {
            schema_version: SCHEMA_VERSION,
            dataset_id: "unit-test".to_owned(),
            sessions: vec![AnnotatedSession {
                session_id: "session-1".to_owned(),
                partition_key: "game-1-recording".to_owned(),
                split: DatasetSplit::Training,
                media: MediaReference {
                    uri: "fixture.mp4".to_owned(),
                    sha256: None,
                    width_px: 1920,
                    height_px: 1080,
                },
                timestamp_source: TimestampSource::MediaPresentation,
                frames: vec![
                    FrameTimestamp {
                        frame_index: 0,
                        capture_us: 0,
                    },
                    FrameTimestamp {
                        frame_index: 1,
                        capture_us: 33_333,
                    },
                    FrameTimestamp {
                        frame_index: 2,
                        capture_us: 66_666,
                    },
                ],
                board: BoardCalibration {
                    corners: BoardCorners {
                        near_left: ImagePoint {
                            x_px: 200.0,
                            y_px: 900.0,
                        },
                        near_right: ImagePoint {
                            x_px: 1700.0,
                            y_px: 900.0,
                        },
                        far_right: ImagePoint {
                            x_px: 1400.0,
                            y_px: 200.0,
                        },
                        far_left: ImagePoint {
                            x_px: 500.0,
                            y_px: 200.0,
                        },
                    },
                    orientation: BoardOrientation::A1NearLeft,
                },
                reference_game: ReferenceGame {
                    initial_position: "startpos".to_owned(),
                    moves: vec![ReferenceMove {
                        ply: 1,
                        uci: "e2e4".to_owned(),
                        completion: CompletionAnnotation::Bounded {
                            earliest_capture_us: 33_333,
                            latest_capture_us: 66_666,
                        },
                    }],
                },
            }],
        }
    }

    #[test]
    fn valid_manifest_round_trips() {
        let manifest = valid_manifest();
        manifest.validate().expect("fixture must be valid");

        let json = serde_json::to_string_pretty(&manifest).expect("serialize manifest");
        let reparsed = parse_manifest(&json).expect("parse serialized manifest");
        assert_eq!(manifest, reparsed);
    }

    #[test]
    fn unknown_json_fields_are_rejected() {
        let mut value = serde_json::to_value(valid_manifest()).expect("serialize manifest");
        value
            .as_object_mut()
            .expect("manifest is an object")
            .insert("unexpected".to_owned(), serde_json::Value::Bool(true));
        let json = serde_json::to_string(&value).expect("serialize modified JSON");

        assert!(matches!(parse_manifest(&json), Err(LoadError::Json(_))));
    }

    #[test]
    fn detects_partition_leakage_across_splits() {
        let mut manifest = valid_manifest();
        let mut second = manifest.sessions[0].clone();
        second.session_id = "session-2".to_owned();
        second.split = DatasetSplit::Validation;
        manifest.sessions.push(second);

        let errors = manifest.validate().expect_err("split leakage must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::PartitionSplitLeakage { .. }))
        );
    }

    #[test]
    fn rejects_non_monotonic_capture_timestamps() {
        let mut manifest = valid_manifest();
        manifest.sessions[0].frames[2].capture_us = 30_000;

        let errors = manifest.validate().expect_err("timestamps must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::FrameTimestampNotIncreasing { .. }))
        );
    }

    #[test]
    fn frame_index_error_does_not_hide_out_of_range_completion() {
        let mut manifest = valid_manifest();
        manifest.sessions[0].frames[2].frame_index = 1;
        manifest.sessions[0].reference_game.moves[0].completion = CompletionAnnotation::Bounded {
            earliest_capture_us: 33_333,
            latest_capture_us: 90_000,
        };

        let errors = manifest.validate().expect_err("both errors must be found");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::FrameIndexNotIncreasing { .. }))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::CompletionOutsideFrames { .. }))
        );
    }

    #[test]
    fn rejects_invalid_uci_and_completion_bounds() {
        let mut manifest = valid_manifest();
        let reference_move = &mut manifest.sessions[0].reference_game.moves[0];
        reference_move.uci = "e2e9".to_owned();
        reference_move.completion = CompletionAnnotation::Bounded {
            earliest_capture_us: 70_000,
            latest_capture_us: 60_000,
        };

        let errors = manifest.validate().expect_err("move annotations must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::InvalidUci { .. }))
        );
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::InvalidCompletionBounds { .. }))
        );
    }

    #[test]
    fn allows_overlapping_bounds_when_an_ordered_timeline_exists() {
        let mut manifest = valid_manifest();
        manifest.sessions[0].reference_game.moves = vec![
            ReferenceMove {
                ply: 1,
                uci: "e2e4".to_owned(),
                completion: CompletionAnnotation::Bounded {
                    earliest_capture_us: 30_000,
                    latest_capture_us: 50_000,
                },
            },
            ReferenceMove {
                ply: 2,
                uci: "e7e5".to_owned(),
                completion: CompletionAnnotation::Bounded {
                    earliest_capture_us: 40_000,
                    latest_capture_us: 60_000,
                },
            },
        ];

        manifest
            .validate()
            .expect("overlapping uncertainty can still preserve move order");
    }

    #[test]
    fn rejects_bounds_that_cannot_follow_prior_moves() {
        let mut manifest = valid_manifest();
        manifest.sessions[0].reference_game.moves = vec![
            ReferenceMove {
                ply: 1,
                uci: "e2e4".to_owned(),
                completion: CompletionAnnotation::Bounded {
                    earliest_capture_us: 50_000,
                    latest_capture_us: 60_000,
                },
            },
            ReferenceMove {
                ply: 2,
                uci: "e7e5".to_owned(),
                completion: CompletionAnnotation::Bounded {
                    earliest_capture_us: 30_000,
                    latest_capture_us: 40_000,
                },
            },
        ];

        let errors = manifest.validate().expect_err("move order must fail");
        assert!(errors.iter().any(|error| matches!(
            error,
            ValidationError::NonChronologicalMoveBounds { ply: 2, .. }
        )));
    }

    #[test]
    fn qualification_requires_sealed_media() {
        let mut manifest = valid_manifest();
        manifest.sessions[0].split = DatasetSplit::Qualification;

        let errors = manifest
            .validate()
            .expect_err("qualification must be sealed");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::MissingQualificationSha256 { .. }))
        );
    }

    #[test]
    fn collection_detects_cross_file_partition_leakage() {
        let first = valid_manifest();
        let mut second = valid_manifest();
        second.sessions[0].session_id = "session-2".to_owned();
        second.sessions[0].split = DatasetSplit::Validation;

        let errors = validate_manifest_collection(&[&first, &second])
            .expect_err("partition leakage across files must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::PartitionSplitLeakage { .. }))
        );
    }

    #[test]
    fn rejects_non_convex_corner_order() {
        let mut manifest = valid_manifest();
        let corners = &mut manifest.sessions[0].board.corners;
        std::mem::swap(&mut corners.near_right, &mut corners.far_right);

        let errors = manifest.validate().expect_err("crossing corners must fail");
        assert!(
            errors
                .iter()
                .any(|error| matches!(error, ValidationError::InvalidBoardQuadrilateral { .. }))
        );
    }
}
