# Manual board geometry

`vision-geometry` provides the deterministic geometry layer for manual calibration. It does not find the board in an image. A caller supplies the four **outer** board corners labeled in chess coordinates:

1. corner nearest `a1`;
2. corner nearest `h1`;
3. corner nearest `h8`;
4. corner nearest `a8`.

The labels stay in this order when the physical board rotates. `BoardOrientation` records which player is near the camera, while the labeled corners define how files and ranks map to pixels.

```rust
use contracts::{BoardCalibration, BoardOrientation, ImagePoint};
use vision_geometry::BoardGeometry;

let geometry = BoardGeometry::from_calibration(BoardCalibration {
    orientation: BoardOrientation::WhiteNear,
    corners: [
        ImagePoint { x: 120.0, y: 880.0 }, // a1
        ImagePoint { x: 930.0, y: 810.0 }, // h1
        ImagePoint { x: 790.0, y: 90.0 },  // h8
        ImagePoint { x: 210.0, y: 150.0 }, // a8
    ],
    reprojection_error_px: None,
})?;

let e4_center = geometry.square_center("e4")?;
let e4_polygon = geometry.square_polygon("e4")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

The crate validates finite metadata and rejects degenerate, concave, or misordered corner polygons. It computes an invertible projective transform for board-to-image and image-to-board mapping, and its synthetic tests cover skew and both labeled orientations.

This transform corrects board-plane perspective only. It does not correct lens distortion or recover a square hidden by a piece or hand. Frame 6 still needs a calibration UI, real-camera corner measurements, persisted calibration versions, overlays, and measured reprojection error on the supported physical setup.
