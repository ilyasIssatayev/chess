use chess_core::ChessGame;
#[test]
fn notation_and_fen_match_independent_python_chess_histories() {
    let corpus: serde_json::Value =
        serde_json::from_str(include_str!("../fixtures/rules-reference.json")).unwrap();
    for case in corpus["cases"].as_array().unwrap() {
        let mut records = vec![];
        let game_id = contracts::GameId::new();
        let mut game = ChessGame::from_fen(case["initial_fen"].as_str().unwrap()).unwrap();
        for reference in case["moves"].as_array().unwrap() {
            let applied = game.play_uci(reference["uci"].as_str().unwrap()).unwrap();
            assert_eq!(
                applied.san,
                reference["san"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            assert_eq!(
                applied.fen_after,
                reference["fen"].as_str().unwrap(),
                "{}",
                case["name"]
            );
            records.push(contracts::MoveRecord {
                game_id,
                revision: 0,
                ply: applied.ply,
                uci: applied.uci.clone(),
                san: applied.san.clone(),
                fen_before: applied.fen_before.clone(),
                fen_after: applied.fen_after.clone(),
                provenance: contracts::MoveProvenance::Manual,
                confidence: None,
                timing: contracts::MoveTiming {
                    completion: None,
                    elapsed_since_previous: None,
                    confirmation_time: contracts::CaptureTimeUs::new(0).unwrap(),
                    quality: contracts::TimingQuality::Unknown,
                },
                evidence_ids: vec![],
            });
        }
        let metadata = export::PgnMetadata {
            result: case["result"].as_str().unwrap().into(),
            initial_fen: if case["name"] == "ordinary"
                || case["name"] == "queenside-both"
                || case["name"] == "en-passant"
            {
                None
            } else {
                Some(case["initial_fen"].as_str().unwrap().into())
            },
            ..export::PgnMetadata::default()
        };
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("local-data/export-check");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join(format!("{}.pgn", case["name"].as_str().unwrap())),
            export::to_pgn(&metadata, &records, true).unwrap(),
        )
        .unwrap();
    }
}
