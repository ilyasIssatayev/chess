use chess_evaluation::{BoardOrientation, DatasetSplit, TimestampSource, parse_manifest};

#[test]
fn documented_json_example_is_valid() {
    let json = include_str!("../examples/manifest.example.json");
    let manifest = parse_manifest(json).expect("documented example must remain valid");

    assert_eq!(manifest.sessions.len(), 1);
    assert_eq!(manifest.sessions[0].split, DatasetSplit::Training);
    assert_eq!(
        manifest.sessions[0].timestamp_source,
        TimestampSource::MediaPresentation
    );
    assert_eq!(
        manifest.sessions[0].board.orientation,
        BoardOrientation::A1NearLeft
    );
}
