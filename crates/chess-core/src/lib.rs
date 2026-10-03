use contracts::PieceClass;
use cozy_chess::util::{display_san_move, display_uci_move, parse_uci_move};
use cozy_chess::{Board, GameStatus, Move};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppliedMove {
    pub ply: u32,
    pub uci: String,
    pub san: String,
    pub fen_before: String,
    pub fen_after: String,
}

#[derive(Debug, Clone)]
pub struct ChessGame {
    initial_fen: String,
    board: Board,
    history: Vec<AppliedMove>,
}

impl Default for ChessGame {
    fn default() -> Self {
        Self::standard()
    }
}

impl ChessGame {
    pub fn standard() -> Self {
        let board = Board::default();
        Self {
            initial_fen: board.to_string(),
            board,
            history: Vec::new(),
        }
    }

    pub fn from_fen(fen: &str) -> Result<Self, ChessError> {
        let board = fen
            .parse::<Board>()
            .map_err(|error| ChessError::InvalidFen(error.to_string()))?;
        Ok(Self {
            initial_fen: board.to_string(),
            board,
            history: Vec::new(),
        })
    }

    pub fn initial_fen(&self) -> &str {
        &self.initial_fen
    }

    pub fn fen(&self) -> String {
        self.board.to_string()
    }

    pub fn history(&self) -> &[AppliedMove] {
        &self.history
    }

    /// Returns the position-only status reported by `cozy-chess`.
    ///
    /// This reports a draw when the FEN halfmove clock reaches 100 plies, but it
    /// does not distinguish a claim from an automatic result. It also does not
    /// scan this game's history for threefold repetition. Higher-level game
    /// adjudication must track repetition and draw claims separately.
    pub fn status(&self) -> GameStatus {
        self.board.status()
    }

    pub fn legal_uci_moves(&self) -> Vec<String> {
        legal_moves(&self.board)
            .into_iter()
            .map(|mv| display_uci_move(&self.board, mv).to_string())
            .collect()
    }

    pub fn piece_classes(&self) -> Vec<(String, Option<PieceClass>)> {
        const FILES: &[u8; 8] = b"abcdefgh";
        let mut squares = Vec::with_capacity(64);
        for rank in 1..=8 {
            for file in FILES {
                let name = format!("{}{rank}", *file as char);
                let square = name.parse().expect("generated square is valid");
                let piece = self.board.piece_on(square);
                let color = self.board.color_on(square);
                let class = match (color, piece) {
                    (Some(cozy_chess::Color::White), Some(piece)) => Some(white_class(piece)),
                    (Some(cozy_chess::Color::Black), Some(piece)) => Some(black_class(piece)),
                    (None, None) => None,
                    _ => unreachable!("board piece/color occupancy must agree"),
                };
                squares.push((name, class));
            }
        }
        squares
    }

    pub fn play_uci(&mut self, uci: &str) -> Result<&AppliedMove, ChessError> {
        let mv = parse_uci_move(&self.board, uci)
            .map_err(|error| ChessError::InvalidUci(error.to_string()))?;
        if !self.board.is_legal(mv) {
            return Err(ChessError::IllegalMove(uci.to_owned()));
        }

        let fen_before = self.board.to_string();
        let canonical_uci = display_uci_move(&self.board, mv).to_string();
        let san = display_san_move(&self.board, mv).to_string();
        self.board.play(mv);
        let applied = AppliedMove {
            ply: self.history.len() as u32 + 1,
            uci: canonical_uci,
            san,
            fen_before,
            fen_after: self.board.to_string(),
        };
        self.history.push(applied);
        Ok(self.history.last().expect("move was just appended"))
    }

    pub fn rebuild(&mut self, uci_moves: &[String]) -> Result<(), ChessError> {
        let initial = self.initial_fen.clone();
        let mut rebuilt = Self::from_fen(&initial)?;
        for uci in uci_moves {
            rebuilt.play_uci(uci)?;
        }
        *self = rebuilt;
        Ok(())
    }
}

fn legal_moves(board: &Board) -> Vec<Move> {
    let mut moves = Vec::new();
    board.generate_moves(|piece_moves| {
        moves.extend(piece_moves);
        false
    });
    moves
}

fn white_class(piece: cozy_chess::Piece) -> PieceClass {
    match piece {
        cozy_chess::Piece::Pawn => PieceClass::WhitePawn,
        cozy_chess::Piece::Knight => PieceClass::WhiteKnight,
        cozy_chess::Piece::Bishop => PieceClass::WhiteBishop,
        cozy_chess::Piece::Rook => PieceClass::WhiteRook,
        cozy_chess::Piece::Queen => PieceClass::WhiteQueen,
        cozy_chess::Piece::King => PieceClass::WhiteKing,
    }
}

fn black_class(piece: cozy_chess::Piece) -> PieceClass {
    match piece {
        cozy_chess::Piece::Pawn => PieceClass::BlackPawn,
        cozy_chess::Piece::Knight => PieceClass::BlackKnight,
        cozy_chess::Piece::Bishop => PieceClass::BlackBishop,
        cozy_chess::Piece::Rook => PieceClass::BlackRook,
        cozy_chess::Piece::Queen => PieceClass::BlackQueen,
        cozy_chess::Piece::King => PieceClass::BlackKing,
    }
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum ChessError {
    #[error("invalid FEN: {0}")]
    InvalidFen(String),
    #[error("invalid UCI move: {0}")]
    InvalidUci(String),
    #[error("illegal move: {0}")]
    IllegalMove(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn play(game: &mut ChessGame, moves: &[&str]) -> Vec<AppliedMove> {
        moves
            .iter()
            .map(|uci| game.play_uci(uci).unwrap().clone())
            .collect()
    }

    fn class_at(game: &ChessGame, square: &str) -> Option<PieceClass> {
        game.piece_classes()
            .into_iter()
            .find_map(|(name, class)| (name == square).then_some(class))
            .expect("every valid square is exposed")
    }

    fn repetition_key(fen: &str) -> String {
        fen.split_whitespace().take(4).collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn records_san_and_fen_chain() {
        let mut game = ChessGame::standard();
        let first = game.play_uci("e2e4").unwrap();
        assert_eq!(first.san, "e4");
        assert_eq!(first.ply, 1);
        assert_eq!(game.play_uci("e7e5").unwrap().san, "e5");
        assert_eq!(game.play_uci("g1f3").unwrap().san, "Nf3");
        assert_eq!(game.history()[0].fen_after, game.history()[1].fen_before);
    }

    #[test]
    fn handles_standard_uci_castling() {
        let mut game = ChessGame::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let applied = game.play_uci("e1g1").unwrap();
        assert_eq!(applied.san, "O-O");
        assert_eq!(class_at(&game, "g1"), Some(PieceClass::WhiteKing));
        assert_eq!(class_at(&game, "f1"), Some(PieceClass::WhiteRook));
        assert_eq!(class_at(&game, "e1"), None);
        assert_eq!(class_at(&game, "h1"), None);
    }

    #[test]
    fn handles_queenside_castling() {
        let mut game = ChessGame::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let applied = game.play_uci("e1c1").unwrap();
        assert_eq!(applied.san, "O-O-O");
        assert_eq!(class_at(&game, "c1"), Some(PieceClass::WhiteKing));
        assert_eq!(class_at(&game, "d1"), Some(PieceClass::WhiteRook));
        assert_eq!(class_at(&game, "e1"), None);
        assert_eq!(class_at(&game, "a1"), None);
    }

    #[test]
    fn handles_both_black_castling_sides() {
        let fen = "r3k2r/8/8/8/8/8/8/R3K2R b KQkq - 0 1";
        let mut kingside = ChessGame::from_fen(fen).unwrap();
        let mut queenside = ChessGame::from_fen(fen).unwrap();

        assert_eq!(kingside.play_uci("e8g8").unwrap().san, "O-O");
        assert_eq!(class_at(&kingside, "g8"), Some(PieceClass::BlackKing));
        assert_eq!(class_at(&kingside, "f8"), Some(PieceClass::BlackRook));
        assert_eq!(class_at(&kingside, "e8"), None);
        assert_eq!(class_at(&kingside, "h8"), None);

        assert_eq!(queenside.play_uci("e8c8").unwrap().san, "O-O-O");
        assert_eq!(class_at(&queenside, "c8"), Some(PieceClass::BlackKing));
        assert_eq!(class_at(&queenside, "d8"), Some(PieceClass::BlackRook));
        assert_eq!(class_at(&queenside, "e8"), None);
        assert_eq!(class_at(&queenside, "a8"), None);
    }

    #[test]
    fn records_captures_and_removes_the_captured_piece() {
        let mut game = ChessGame::standard();
        let applied = play(&mut game, &["e2e4", "d7d5", "e4d5", "d8d5"]);

        assert_eq!(
            applied.iter().map(|mv| mv.san.as_str()).collect::<Vec<_>>(),
            ["e4", "d5", "exd5", "Qxd5"]
        );
        assert_eq!(class_at(&game, "d5"), Some(PieceClass::BlackQueen));
        assert_eq!(class_at(&game, "d8"), None);
    }

    #[test]
    fn records_checkmate_marker_and_terminal_status() {
        let mut game = ChessGame::standard();
        let applied = play(&mut game, &["f2f3", "e7e5", "g2g4", "d8h4"]);

        assert_eq!(applied.last().unwrap().san, "Qh4#");
        assert_eq!(game.status(), GameStatus::Won);
        assert!(game.legal_uci_moves().is_empty());
    }

    #[test]
    fn records_check_marker_without_ending_the_game() {
        let mut game = ChessGame::standard();
        let applied = play(&mut game, &["e2e4", "f7f6", "d1h5"]);

        assert_eq!(applied.last().unwrap().san, "Qh5+");
        assert_eq!(game.status(), GameStatus::Ongoing);
        assert!(!game.legal_uci_moves().is_empty());
    }

    #[test]
    fn disambiguates_knights_that_can_reach_the_same_square() {
        let fen = "4k3/8/8/8/8/8/8/1N2KN2 w - - 0 1";
        let mut queen_knight = ChessGame::from_fen(fen).unwrap();
        let mut king_knight = ChessGame::from_fen(fen).unwrap();

        assert_eq!(queen_knight.play_uci("b1d2").unwrap().san, "Nbd2");
        assert_eq!(king_knight.play_uci("f1d2").unwrap().san, "Nfd2");
    }

    #[test]
    fn disambiguates_by_rank_when_same_file_rooks_can_move() {
        let fen = "7k/8/8/8/8/R7/8/R6K w - - 0 1";
        let mut first_rank = ChessGame::from_fen(fen).unwrap();
        let mut third_rank = ChessGame::from_fen(fen).unwrap();

        assert_eq!(first_rank.play_uci("a1a2").unwrap().san, "R1a2");
        assert_eq!(third_rank.play_uci("a3a2").unwrap().san, "R3a2");
    }

    #[test]
    fn disambiguates_by_full_origin_square_when_required() {
        let fen = "7k/8/8/1N6/8/1N3N2/8/7K w - - 0 1";
        let mut game = ChessGame::from_fen(fen).unwrap();

        assert_eq!(game.play_uci("b3d4").unwrap().san, "Nb3d4");
    }

    #[test]
    fn handles_en_passant_capture() {
        let mut game = ChessGame::standard();
        let applied = play(&mut game, &["e2e4", "a7a6", "e4e5", "d7d5", "e5d6"]);

        assert_eq!(applied.last().unwrap().san, "exd6");
        assert_eq!(class_at(&game, "d6"), Some(PieceClass::WhitePawn));
        assert_eq!(class_at(&game, "d5"), None);
        assert_eq!(class_at(&game, "e5"), None);
    }

    #[test]
    fn handles_all_four_promotion_choices() {
        let cases = [
            ("a7a8q", "a8=Q+", PieceClass::WhiteQueen),
            ("a7a8r", "a8=R+", PieceClass::WhiteRook),
            ("a7a8b", "a8=B", PieceClass::WhiteBishop),
            ("a7a8n", "a8=N", PieceClass::WhiteKnight),
        ];

        for (uci, expected_san, expected_class) in cases {
            let mut game = ChessGame::from_fen("7k/P7/8/8/8/8/8/7K w - - 0 1").unwrap();
            assert_eq!(game.play_uci(uci).unwrap().san, expected_san);
            assert_eq!(class_at(&game, "a8"), Some(expected_class));
            assert_eq!(class_at(&game, "a7"), None);
        }
    }

    #[test]
    fn records_a_promotion_capture() {
        let mut game = ChessGame::from_fen("1r5k/P7/8/8/8/8/8/7K w - - 0 1").unwrap();

        assert_eq!(game.play_uci("a7b8q").unwrap().san, "axb8=Q+");
        assert_eq!(class_at(&game, "b8"), Some(PieceClass::WhiteQueen));
        assert_eq!(class_at(&game, "a7"), None);
    }

    #[test]
    fn handles_black_promotion() {
        let mut game = ChessGame::from_fen("7k/8/8/8/8/8/p7/7K b - - 0 1").unwrap();

        assert_eq!(game.play_uci("a2a1q").unwrap().san, "a1=Q+");
        assert_eq!(class_at(&game, "a1"), Some(PieceClass::BlackQueen));
        assert_eq!(class_at(&game, "a2"), None);
    }

    #[test]
    fn identifies_stalemate_as_drawn() {
        let game = ChessGame::from_fen("k7/2Q5/2K5/8/8/8/8/8 b - - 0 1").unwrap();

        assert_eq!(game.status(), GameStatus::Drawn);
        assert!(game.legal_uci_moves().is_empty());
    }

    #[test]
    fn status_uses_the_fen_halfmove_clock_for_the_fifty_move_threshold() {
        let before_threshold = ChessGame::from_fen("7k/8/8/8/8/8/R7/7K w - - 99 51").unwrap();
        let at_threshold = ChessGame::from_fen("7k/8/8/8/8/8/R7/7K w - - 100 51").unwrap();

        assert_eq!(before_threshold.status(), GameStatus::Ongoing);
        assert_eq!(at_threshold.status(), GameStatus::Drawn);
        assert!(!at_threshold.legal_uci_moves().is_empty());
    }

    #[test]
    fn rejects_illegal_move() {
        let mut game = ChessGame::standard();
        let before = game.fen();
        assert!(matches!(
            game.play_uci("e2e5"),
            Err(ChessError::IllegalMove(_))
        ));
        assert_eq!(game.fen(), before);
        assert!(game.history().is_empty());
    }

    #[test]
    fn rejects_malformed_uci_and_fen() {
        let mut game = ChessGame::standard();
        assert!(matches!(
            game.play_uci("not-a-move"),
            Err(ChessError::InvalidUci(_))
        ));
        assert!(matches!(
            ChessGame::from_fen("not a FEN"),
            Err(ChessError::InvalidFen(_))
        ));
        assert!(game.history().is_empty());
    }

    #[test]
    fn rebuild_is_atomic_on_error() {
        let mut game = ChessGame::standard();
        game.play_uci("d2d4").unwrap();
        let before_fen = game.fen();
        let before_history = game.history().to_vec();
        assert!(game.rebuild(&["e2e4".into(), "e7e4".into()]).is_err());
        assert_eq!(game.fen(), before_fen);
        assert_eq!(game.history(), before_history);

        assert!(game.rebuild(&["e2e4".into(), "not-a-move".into()]).is_err());
        assert_eq!(game.fen(), before_fen);
        assert_eq!(game.history(), before_history);
    }

    #[test]
    fn rebuilds_a_threefold_position_without_adjudicating_repetition() {
        let moves = [
            "g1f3", "g8f6", "f3g1", "f6g8", "g1f3", "g8f6", "f3g1", "f6g8",
        ];
        let expected_san = ["Nf3", "Nf6", "Ng1", "Ng8", "Nf3", "Nf6", "Ng1", "Ng8"];
        let initial = ChessGame::standard();
        let initial_key = repetition_key(initial.initial_fen());
        let mut game = ChessGame::standard();

        game.rebuild(&moves.map(String::from)).unwrap();

        assert_eq!(
            game.history()
                .iter()
                .map(|mv| mv.san.as_str())
                .collect::<Vec<_>>(),
            expected_san
        );
        assert_eq!(repetition_key(&game.history()[3].fen_after), initial_key);
        assert_eq!(repetition_key(&game.history()[7].fen_after), initial_key);
        assert_eq!(repetition_key(&game.fen()), initial_key);
        // `Board::status` has no prior-position history, so repetition is a
        // separate higher-level adjudication concern.
        assert_eq!(game.status(), GameStatus::Ongoing);
    }

    #[test]
    fn exposes_standard_position_piece_classes() {
        let game = ChessGame::standard();
        let classes = game.piece_classes();
        assert_eq!(classes.len(), 64);
        assert_eq!(classes[0], ("a1".into(), Some(PieceClass::WhiteRook)));
        assert_eq!(classes[63], ("h8".into(), Some(PieceClass::BlackRook)));
    }
}
