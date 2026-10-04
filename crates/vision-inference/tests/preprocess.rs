use vision_inference::preprocess::*;

fn corners() -> Corners {
    Corners {
        a8: Point { x: 0.1, y: 0.1 },
        h8: Point { x: 0.9, y: 0.1 },
        h1: Point { x: 0.9, y: 0.9 },
        a1: Point { x: 0.1, y: 0.9 },
    }
}
#[test]
fn calibration_json_preserves_the_nearest_double_value() {
    for text in [
        "0.8455124082255701",
        "0.9418870536475365",
        "0.12345678901234566",
        "1.2345678901234567e-16",
    ] {
        let point: Point = serde_json::from_str(&format!(r#"{{"x":{text},"y":0.5}}"#)).unwrap();
        assert_eq!(point.x.to_bits(), text.parse::<f64>().unwrap().to_bits());
    }
}
#[test]
fn labelled_rotations_and_reflections_are_distinct() {
    let order = [corners().a8, corners().h8, corners().h1, corners().a1];
    for shift in 0..4 {
        let c = Corners {
            a8: order[shift],
            h8: order[(shift + 1) % 4],
            h1: order[(shift + 2) % 4],
            a1: order[(shift + 3) % 4],
        };
        let g = Geometry::new(c, 640, 480).unwrap();
        let mut squares = g.squares.clone();
        squares.sort();
        squares.dedup();
        assert_eq!(squares.len(), 64);
        let expected = ["a8", "a1", "h1", "h8"][shift];
        assert_eq!(g.squares[0], expected);
    }
    let mut bad = corners();
    std::mem::swap(&mut bad.a8, &mut bad.h8);
    assert!(Geometry::new(bad, 640, 480).is_err());
}
#[test]
fn malformed_frames_and_corners_fail_before_pixel_access() {
    assert!(RgbaFrame::new(0, 100, vec![]).is_err());
    assert!(RgbaFrame::new(8192, 8192, vec![]).is_err());
    assert!(RgbaFrame::new(2, 2, vec![0; 15]).is_err());
    let mut bad = corners();
    bad.a8.x = f64::NAN;
    assert!(Geometry::new(bad, 640, 480).is_err());
    let mut bad = corners();
    bad.a8 = bad.h8;
    assert!(Geometry::new(bad, 640, 480).is_err());
    let g = Geometry::new(corners(), 640, 480).unwrap();
    assert!(
        warp(
            &RgbaFrame {
                width: 640,
                height: 480,
                rgba: vec![]
            },
            &g,
            50,
            500
        )
        .is_err()
    );
}
#[test]
fn crops_preserve_zero_padding_and_mark_missing_camera_coverage() {
    let frame = RgbaFrame::new(10, 10, vec![255; 400]).unwrap();
    let edge = Corners {
        a8: Point { x: 0.0, y: 0.0 },
        h8: Point { x: 1.0, y: 0.0 },
        h1: Point { x: 1.0, y: 1.0 },
        a1: Point { x: 0.0, y: 1.0 },
    };
    let g = Geometry::new(edge, 10, 10).unwrap();
    let w = warp(&frame, &g, 200, 800).unwrap();
    let c = piece_crop(&w, 0, 0).unwrap();
    assert!(c.coverage < 0.75);
    assert!(c.rgb.contains(&0));
    assert!(c.rgb.contains(&255));
    assert!(piece_crop(&w, 8, 0).is_err());
    assert!(occupancy_crop(&w, 0, 0).is_err());
}
#[test]
fn nchw_normalization_uses_double_arithmetic_then_float_storage() {
    let crop = Crop {
        rgb: (0..30000).map(|i| (i % 256) as u8).collect(),
        coverage: 1.0,
    };
    let out = tensor(&[crop.clone(), crop.clone()], 100, 100).unwrap();
    assert_eq!(out.len(), 60000);
    for c in 0..3 {
        for i in [0, 1, 127, 9999] {
            let expected = ((f64::from(crop.rgb[i * 3 + c]) / 255.0 - [0.485, 0.456, 0.406][c])
                / [0.229, 0.224, 0.225][c]) as f32;
            assert_eq!(out[c * 10000 + i].to_bits(), expected.to_bits());
            assert_eq!(out[c * 10000 + i], out[30000 + c * 10000 + i]);
        }
    }
    assert!(tensor(&[], 100, 100).is_err());
    assert!(
        tensor(
            &[Crop {
                rgb: vec![],
                coverage: 1.0
            }],
            100,
            200
        )
        .is_err()
    );
}
#[test]
fn class_mapping_and_visibility_do_not_invent_piece_identity() {
    let names = Geometry::new(corners(), 640, 480).unwrap().squares;
    for (model, target) in MODEL_TO_CONTRACT.into_iter().enumerate() {
        let mut p = vec![0.0; 12];
        p[model] = 1.0;
        let e = evidence(&names, &[0.8; 64], &vec![Some(p); 64], &[1.0; 64]).unwrap();
        assert_eq!(e[0].piece_probabilities[target], 0.8);
        assert_eq!(e[0].visible_probability, 1.0);
    }
    let e = evidence(&names, &[0.01; 64], &vec![None; 64], &[1.0; 64]).unwrap();
    assert_eq!(e[0].empty_probability, 0.99);
    let e = evidence(&names, &[0.9; 64], &vec![None; 64], &[1.0; 64]).unwrap();
    assert_eq!(e[0].visible_probability, 0.0);
    let e = evidence(&names, &[0.01; 64], &vec![None; 64], &[0.74; 64]).unwrap();
    assert_eq!(e[0].visible_probability, 0.0);
}
#[test]
fn invalid_logits_and_probabilities_are_rejected() {
    assert!(softmax(&[f32::NAN, 0.0]).is_err());
    assert!(softmax(&[]).is_err());
    let values = softmax(&[1000.0, 1001.0]).unwrap();
    assert!((values.iter().sum::<f64>() - 1.0).abs() < 1e-12);
    let names = Geometry::new(corners(), 640, 480).unwrap().squares;
    assert!(evidence(&names, &[f64::NAN; 64], &vec![None; 64], &[1.0; 64]).is_err());
    assert!(
        evidence(
            &names,
            &[0.5; 64],
            &vec![Some(vec![0.0; 12]); 64],
            &[1.0; 64]
        )
        .is_err()
    );
}
