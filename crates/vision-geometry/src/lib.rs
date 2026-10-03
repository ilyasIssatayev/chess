//! Board-plane geometry derived from a manual four-corner calibration.
//!
//! The calibration contract labels the outer board corners nearest `a1`, `h1`,
//! `h8`, and `a8`. Those labels, rather than image left/right, define the chess
//! coordinate system and allow the same mapping code to handle either player
//! nearest the camera.

use std::error::Error;
use std::fmt::{self, Display, Formatter};

use contracts::{BoardCalibration, BoardOrientation, ImagePoint};

const CORNER_COUNT: usize = 4;
const MATRIX_SIZE: usize = 3;
const SOLVE_SIZE: usize = 8;
const NORMALIZED_EPSILON: f64 = 1.0e-9;

/// A point in normalized board coordinates.
///
/// `(0, 0)` is the outer corner nearest `a1`; `(1, 1)` is the outer corner
/// nearest `h8`. File and rank increase in chess-coordinate order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoardPoint {
    pub file: f64,
    pub rank: f64,
}

impl BoardPoint {
    pub const fn new(file: f64, rank: f64) -> Self {
        Self { file, rank }
    }
}

/// A projective mapping between the normalized chess board and an image.
#[derive(Debug, Clone, PartialEq)]
pub struct BoardGeometry {
    calibration: BoardCalibration,
    board_to_image: [[f64; MATRIX_SIZE]; MATRIX_SIZE],
    image_to_board: [[f64; MATRIX_SIZE]; MATRIX_SIZE],
}

impl BoardGeometry {
    /// Validates the labeled corners and constructs forward and inverse
    /// projective transforms.
    pub fn from_calibration(calibration: BoardCalibration) -> Result<Self, GeometryError> {
        validate_calibration(&calibration)?;

        let source = [
            BoardPoint::new(0.0, 0.0),
            BoardPoint::new(1.0, 0.0),
            BoardPoint::new(1.0, 1.0),
            BoardPoint::new(0.0, 1.0),
        ];
        let board_to_image = solve_homography(source, calibration.corners)?;
        let image_to_board = invert_3x3(board_to_image)?;

        Ok(Self {
            calibration,
            board_to_image,
            image_to_board,
        })
    }

    pub const fn calibration(&self) -> &BoardCalibration {
        &self.calibration
    }

    pub const fn orientation(&self) -> BoardOrientation {
        self.calibration.orientation
    }

    /// Projects a normalized board point into image pixels.
    ///
    /// Coordinates outside `[0, 1]` are accepted so callers can project a
    /// small margin around the board when building image crops.
    pub fn board_to_image(&self, point: BoardPoint) -> Result<ImagePoint, GeometryError> {
        if !point.file.is_finite() || !point.rank.is_finite() {
            return Err(GeometryError::NonFiniteBoardPoint);
        }
        let (x, y) = transform_point(self.board_to_image, point.file, point.rank)?;
        let x = x as f32;
        let y = y as f32;
        if !x.is_finite() || !y.is_finite() {
            return Err(GeometryError::ProjectedPointOutOfRange);
        }
        Ok(ImagePoint { x, y })
    }

    /// Maps an image pixel into normalized chess board coordinates.
    pub fn image_to_board(&self, point: ImagePoint) -> Result<BoardPoint, GeometryError> {
        if !point.x.is_finite() || !point.y.is_finite() {
            return Err(GeometryError::NonFiniteImagePoint);
        }
        let (file, rank) =
            transform_point(self.image_to_board, f64::from(point.x), f64::from(point.y))?;
        Ok(BoardPoint::new(file, rank))
    }

    /// Returns the projected center of an algebraic square such as `e4`.
    pub fn square_center(&self, square: &str) -> Result<ImagePoint, GeometryError> {
        let (file, rank) = parse_square(square)?;
        self.board_to_image(BoardPoint::new(
            (f64::from(file) + 0.5) / 8.0,
            (f64::from(rank) + 0.5) / 8.0,
        ))
    }

    /// Returns a square's projected polygon in chess-coordinate order:
    /// lower-left, lower-right, upper-right, upper-left.
    pub fn square_polygon(&self, square: &str) -> Result<[ImagePoint; 4], GeometryError> {
        let (file, rank) = parse_square(square)?;
        let low_file = f64::from(file) / 8.0;
        let high_file = f64::from(file + 1) / 8.0;
        let low_rank = f64::from(rank) / 8.0;
        let high_rank = f64::from(rank + 1) / 8.0;

        Ok([
            self.board_to_image(BoardPoint::new(low_file, low_rank))?,
            self.board_to_image(BoardPoint::new(high_file, low_rank))?,
            self.board_to_image(BoardPoint::new(high_file, high_rank))?,
            self.board_to_image(BoardPoint::new(low_file, high_rank))?,
        ])
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum GeometryError {
    NonFiniteCorner { index: usize },
    InvalidReprojectionError,
    DegenerateCorners,
    NonConvexOrMisorderedCorners,
    SingularTransform,
    PointAtInfinity,
    NonFiniteBoardPoint,
    NonFiniteImagePoint,
    ProjectedPointOutOfRange,
    InvalidSquare(String),
}

impl Display for GeometryError {
    fn fmt(&self, formatter: &mut Formatter<'_>) -> fmt::Result {
        match self {
            Self::NonFiniteCorner { index } => {
                write!(formatter, "calibration corner {index} is not finite")
            }
            Self::InvalidReprojectionError => {
                formatter.write_str("reprojection error must be finite and non-negative")
            }
            Self::DegenerateCorners => {
                formatter.write_str("calibration corners do not span a usable board area")
            }
            Self::NonConvexOrMisorderedCorners => formatter
                .write_str("calibration corners must be a convex perimeter ordered a1, h1, h8, a8"),
            Self::SingularTransform => formatter
                .write_str("calibration does not define an invertible projective transform"),
            Self::PointAtInfinity => {
                formatter.write_str("point maps to infinity under the calibration transform")
            }
            Self::NonFiniteBoardPoint => {
                formatter.write_str("board point coordinates must be finite")
            }
            Self::NonFiniteImagePoint => {
                formatter.write_str("image point coordinates must be finite")
            }
            Self::ProjectedPointOutOfRange => {
                formatter.write_str("projected image point is outside the finite f32 range")
            }
            Self::InvalidSquare(square) => {
                write!(formatter, "invalid algebraic square `{square}`")
            }
        }
    }
}

impl Error for GeometryError {}

fn validate_calibration(calibration: &BoardCalibration) -> Result<(), GeometryError> {
    for (index, corner) in calibration.corners.iter().enumerate() {
        if !corner.x.is_finite() || !corner.y.is_finite() {
            return Err(GeometryError::NonFiniteCorner { index });
        }
    }
    if calibration
        .reprojection_error_px
        .is_some_and(|error| !error.is_finite() || error < 0.0)
    {
        return Err(GeometryError::InvalidReprojectionError);
    }

    let (min_x, max_x, min_y, max_y) = bounds(&calibration.corners);
    let scale_squared = (max_x - min_x).powi(2) + (max_y - min_y).powi(2);
    if scale_squared <= f64::EPSILON {
        return Err(GeometryError::DegenerateCorners);
    }
    let cross_tolerance = scale_squared * NORMALIZED_EPSILON;
    let mut winding = 0.0;
    for index in 0..CORNER_COUNT {
        let first = calibration.corners[index];
        let second = calibration.corners[(index + 1) % CORNER_COUNT];
        let third = calibration.corners[(index + 2) % CORNER_COUNT];
        let cross = cross_product(first, second, third);
        if cross.abs() <= cross_tolerance {
            return Err(GeometryError::DegenerateCorners);
        }
        if index == 0 {
            winding = cross.signum();
        } else if cross.signum() != winding {
            return Err(GeometryError::NonConvexOrMisorderedCorners);
        }
    }

    let doubled_area = signed_doubled_area(&calibration.corners).abs();
    if doubled_area <= cross_tolerance {
        return Err(GeometryError::DegenerateCorners);
    }
    Ok(())
}

fn bounds(points: &[ImagePoint; CORNER_COUNT]) -> (f64, f64, f64, f64) {
    points.iter().fold(
        (
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
        ),
        |(min_x, max_x, min_y, max_y), point| {
            let x = f64::from(point.x);
            let y = f64::from(point.y);
            (min_x.min(x), max_x.max(x), min_y.min(y), max_y.max(y))
        },
    )
}

fn cross_product(first: ImagePoint, second: ImagePoint, third: ImagePoint) -> f64 {
    let first_x = f64::from(first.x);
    let first_y = f64::from(first.y);
    let second_x = f64::from(second.x);
    let second_y = f64::from(second.y);
    let third_x = f64::from(third.x);
    let third_y = f64::from(third.y);
    (second_x - first_x) * (third_y - second_y) - (second_y - first_y) * (third_x - second_x)
}

fn signed_doubled_area(points: &[ImagePoint; CORNER_COUNT]) -> f64 {
    (0..CORNER_COUNT)
        .map(|index| {
            let point = points[index];
            let next = points[(index + 1) % CORNER_COUNT];
            f64::from(point.x) * f64::from(next.y) - f64::from(next.x) * f64::from(point.y)
        })
        .sum()
}

fn solve_homography(
    source: [BoardPoint; CORNER_COUNT],
    destination: [ImagePoint; CORNER_COUNT],
) -> Result<[[f64; MATRIX_SIZE]; MATRIX_SIZE], GeometryError> {
    let mut system = [[0.0; SOLVE_SIZE + 1]; SOLVE_SIZE];
    for index in 0..CORNER_COUNT {
        let row = index * 2;
        let x = source[index].file;
        let y = source[index].rank;
        let u = f64::from(destination[index].x);
        let v = f64::from(destination[index].y);
        system[row] = [x, y, 1.0, 0.0, 0.0, 0.0, -u * x, -u * y, u];
        system[row + 1] = [0.0, 0.0, 0.0, x, y, 1.0, -v * x, -v * y, v];
    }

    let solved = solve_linear_system(system)?;
    Ok([
        [solved[0], solved[1], solved[2]],
        [solved[3], solved[4], solved[5]],
        [solved[6], solved[7], 1.0],
    ])
}

fn solve_linear_system(
    mut system: [[f64; SOLVE_SIZE + 1]; SOLVE_SIZE],
) -> Result<[f64; SOLVE_SIZE], GeometryError> {
    for pivot_column in 0..SOLVE_SIZE {
        let pivot_row = (pivot_column..SOLVE_SIZE)
            .max_by(|left, right| {
                system[*left][pivot_column]
                    .abs()
                    .total_cmp(&system[*right][pivot_column].abs())
            })
            .expect("pivot range is never empty");
        let pivot = system[pivot_row][pivot_column];
        if pivot.abs() <= f64::EPSILON {
            return Err(GeometryError::SingularTransform);
        }
        system.swap(pivot_column, pivot_row);

        for value in &mut system[pivot_column][pivot_column..] {
            *value /= pivot;
        }
        let pivot_values = system[pivot_column];
        for (row, values) in system.iter_mut().enumerate() {
            if row == pivot_column {
                continue;
            }
            let factor = values[pivot_column];
            for (value, pivot_value) in values[pivot_column..]
                .iter_mut()
                .zip(&pivot_values[pivot_column..])
            {
                *value -= factor * pivot_value;
            }
        }
    }

    let mut solution = [0.0; SOLVE_SIZE];
    for index in 0..SOLVE_SIZE {
        solution[index] = system[index][SOLVE_SIZE];
    }
    Ok(solution)
}

fn invert_3x3(
    matrix: [[f64; MATRIX_SIZE]; MATRIX_SIZE],
) -> Result<[[f64; MATRIX_SIZE]; MATRIX_SIZE], GeometryError> {
    let [a, b, c] = matrix[0];
    let [d, e, f] = matrix[1];
    let [g, h, i] = matrix[2];
    let determinant = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if determinant.abs() <= f64::EPSILON {
        return Err(GeometryError::SingularTransform);
    }

    let inverse_determinant = 1.0 / determinant;
    Ok([
        [
            (e * i - f * h) * inverse_determinant,
            (c * h - b * i) * inverse_determinant,
            (b * f - c * e) * inverse_determinant,
        ],
        [
            (f * g - d * i) * inverse_determinant,
            (a * i - c * g) * inverse_determinant,
            (c * d - a * f) * inverse_determinant,
        ],
        [
            (d * h - e * g) * inverse_determinant,
            (b * g - a * h) * inverse_determinant,
            (a * e - b * d) * inverse_determinant,
        ],
    ])
}

fn transform_point(
    matrix: [[f64; MATRIX_SIZE]; MATRIX_SIZE],
    x: f64,
    y: f64,
) -> Result<(f64, f64), GeometryError> {
    let denominator = matrix[2][0] * x + matrix[2][1] * y + matrix[2][2];
    if denominator.abs() <= f64::EPSILON {
        return Err(GeometryError::PointAtInfinity);
    }
    let mapped_x = (matrix[0][0] * x + matrix[0][1] * y + matrix[0][2]) / denominator;
    let mapped_y = (matrix[1][0] * x + matrix[1][1] * y + matrix[1][2]) / denominator;
    if !mapped_x.is_finite() || !mapped_y.is_finite() {
        return Err(GeometryError::PointAtInfinity);
    }
    Ok((mapped_x, mapped_y))
}

fn parse_square(square: &str) -> Result<(u8, u8), GeometryError> {
    let bytes = square.as_bytes();
    if let [file @ b'a'..=b'h', rank @ b'1'..=b'8'] = bytes {
        Ok((file - b'a', rank - b'1'))
    } else {
        Err(GeometryError::InvalidSquare(square.to_owned()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn calibration(orientation: BoardOrientation, corners: [(f32, f32); 4]) -> BoardCalibration {
        BoardCalibration {
            orientation,
            corners: corners.map(|(x, y)| ImagePoint { x, y }),
            reprojection_error_px: None,
        }
    }

    fn assert_point_close(actual: ImagePoint, expected: (f64, f64), tolerance: f64) {
        assert!((f64::from(actual.x) - expected.0).abs() <= tolerance);
        assert!((f64::from(actual.y) - expected.1).abs() <= tolerance);
    }

    #[test]
    fn maps_skewed_board_corners_and_round_trips_internal_points() {
        let geometry = BoardGeometry::from_calibration(calibration(
            BoardOrientation::WhiteNear,
            [
                (120.0, 880.0),
                (930.0, 810.0),
                (790.0, 90.0),
                (210.0, 150.0),
            ],
        ))
        .unwrap();

        for (board, expected) in [
            (BoardPoint::new(0.0, 0.0), (120.0, 880.0)),
            (BoardPoint::new(1.0, 0.0), (930.0, 810.0)),
            (BoardPoint::new(1.0, 1.0), (790.0, 90.0)),
            (BoardPoint::new(0.0, 1.0), (210.0, 150.0)),
        ] {
            assert_point_close(geometry.board_to_image(board).unwrap(), expected, 1.0e-3);
        }

        for expected in [
            BoardPoint::new(0.13, 0.27),
            BoardPoint::new(0.5, 0.5),
            BoardPoint::new(0.91, 0.72),
        ] {
            let image = geometry.board_to_image(expected).unwrap();
            let actual = geometry.image_to_board(image).unwrap();
            assert!((actual.file - expected.file).abs() < 1.0e-6);
            assert!((actual.rank - expected.rank).abs() < 1.0e-6);
        }
    }

    #[test]
    fn white_near_labels_put_a1_at_bottom_left() {
        let geometry = BoardGeometry::from_calibration(calibration(
            BoardOrientation::WhiteNear,
            [
                (100.0, 900.0),
                (900.0, 900.0),
                (900.0, 100.0),
                (100.0, 100.0),
            ],
        ))
        .unwrap();

        assert_eq!(geometry.orientation(), BoardOrientation::WhiteNear);
        assert_point_close(
            geometry.square_center("a1").unwrap(),
            (150.0, 850.0),
            1.0e-4,
        );
        assert_point_close(
            geometry.square_center("h8").unwrap(),
            (850.0, 150.0),
            1.0e-4,
        );
        let polygon = geometry.square_polygon("a1").unwrap();
        assert_point_close(polygon[0], (100.0, 900.0), 1.0e-4);
    }

    #[test]
    fn black_near_labels_rotate_chess_coordinates_in_the_image() {
        let geometry = BoardGeometry::from_calibration(calibration(
            BoardOrientation::BlackNear,
            [
                (900.0, 100.0),
                (100.0, 100.0),
                (100.0, 900.0),
                (900.0, 900.0),
            ],
        ))
        .unwrap();

        assert_eq!(geometry.orientation(), BoardOrientation::BlackNear);
        assert_point_close(
            geometry.square_center("a1").unwrap(),
            (850.0, 150.0),
            1.0e-4,
        );
        assert_point_close(
            geometry.square_center("h8").unwrap(),
            (150.0, 850.0),
            1.0e-4,
        );
        let polygon = geometry.square_polygon("a1").unwrap();
        assert_point_close(polygon[0], (900.0, 100.0), 1.0e-4);
    }

    #[test]
    fn rejects_non_finite_degenerate_concave_and_misordered_corners() {
        let cases = [
            calibration(
                BoardOrientation::WhiteNear,
                [(f32::NAN, 0.0), (2.0, 0.0), (2.0, 2.0), (0.0, 2.0)],
            ),
            calibration(
                BoardOrientation::WhiteNear,
                [(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 0.0)],
            ),
            calibration(
                BoardOrientation::WhiteNear,
                [(0.0, 0.0), (2.0, 0.0), (0.8, 0.5), (0.0, 2.0)],
            ),
            calibration(
                BoardOrientation::WhiteNear,
                [(0.0, 0.0), (2.0, 2.0), (2.0, 0.0), (0.0, 2.0)],
            ),
        ];

        assert!(matches!(
            BoardGeometry::from_calibration(cases[0].clone()),
            Err(GeometryError::NonFiniteCorner { index: 0 })
        ));
        assert!(matches!(
            BoardGeometry::from_calibration(cases[1].clone()),
            Err(GeometryError::DegenerateCorners)
        ));
        assert!(matches!(
            BoardGeometry::from_calibration(cases[2].clone()),
            Err(GeometryError::NonConvexOrMisorderedCorners)
        ));
        assert!(BoardGeometry::from_calibration(cases[3].clone()).is_err());
    }

    #[test]
    fn rejects_bad_reprojection_error_and_square_names() {
        let mut invalid = calibration(
            BoardOrientation::WhiteNear,
            [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)],
        );
        invalid.reprojection_error_px = Some(-0.1);
        assert_eq!(
            BoardGeometry::from_calibration(invalid).unwrap_err(),
            GeometryError::InvalidReprojectionError
        );

        let geometry = BoardGeometry::from_calibration(calibration(
            BoardOrientation::WhiteNear,
            [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)],
        ))
        .unwrap();
        for invalid_square in ["", "a0", "i1", "A1", "a10", "é4"] {
            assert!(matches!(
                geometry.square_center(invalid_square),
                Err(GeometryError::InvalidSquare(_))
            ));
        }
    }

    #[test]
    fn rejects_non_finite_mapping_inputs() {
        let geometry = BoardGeometry::from_calibration(calibration(
            BoardOrientation::WhiteNear,
            [(0.0, 0.0), (8.0, 0.0), (8.0, 8.0), (0.0, 8.0)],
        ))
        .unwrap();
        assert_eq!(
            geometry
                .board_to_image(BoardPoint::new(f64::INFINITY, 0.0))
                .unwrap_err(),
            GeometryError::NonFiniteBoardPoint
        );
        assert_eq!(
            geometry
                .image_to_board(ImagePoint {
                    x: 0.0,
                    y: f32::NAN,
                })
                .unwrap_err(),
            GeometryError::NonFiniteImagePoint
        );
    }

    #[test]
    fn rejects_projection_outside_image_point_range() {
        let geometry = BoardGeometry::from_calibration(calibration(
            BoardOrientation::WhiteNear,
            [(0.0, 1.0), (1.0, 1.0), (1.0, 0.0), (0.0, 0.0)],
        ))
        .unwrap();
        assert_eq!(
            geometry.board_to_image(BoardPoint::new(1.0e39, 0.5)),
            Err(GeometryError::ProjectedPointOutOfRange)
        );
    }
}
