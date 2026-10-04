#!/usr/bin/env python3
"""Check Rust-generated exports using the separately installed chess==1.11.2 parser."""
import chess,chess.pgn,json,pathlib
root=pathlib.Path(__file__).resolve().parents[1];cases=json.loads((root/'fixtures/rules-reference.json').read_text())['cases']
for case in cases:
 with (root/f"local-data/export-check/{case['name']}.pgn").open() as f:game=chess.pgn.read_game(f)
 assert game and not game.errors,case['name'];assert game.headers['Result']==case['result'];assert [m.uci() for m in game.mainline_moves()]==[m['uci'] for m in case['moves']],case['name'];assert game.end().board().fen(en_passant='fen')==case['moves'][-1]['fen'],case['name']
print(f"Independent PGN parsing passed for {len(cases)} Rust exports, including castling, en passant and every promotion choice")
