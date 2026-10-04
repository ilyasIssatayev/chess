#!/usr/bin/env python3
"""Offline reference fixture generation with chess==1.11.2; not shipped in the app."""
import chess,chess.pgn,json,pathlib
cases=[('ordinary',chess.STARTING_FEN,'e2e4 e7e5 g1f3 b8c6 f1b5 a7a6 b5a4 g8f6 e1g1 f8e7 f1e1 b7b5 a4b3 e8g8'),('queenside-both',chess.STARTING_FEN,'d2d4 d7d5 b1c3 b8c6 c1f4 c8f5 d1d2 d8d7 e1c1 e8c8'),('en-passant',chess.STARTING_FEN,'e2e4 a7a6 e4e5 d7d5 e5d6')]
for color,fen,base in [('white','7k/P7/8/8/8/8/8/7K w - - 0 1','a7a8'),('black','7k/8/8/8/8/8/p7/7K b - - 0 1','a2a1'),('capture','1r5k/P7/8/8/8/8/8/7K w - - 0 1','a7b8')]:
 for piece in 'qrbn':cases.append((f'{color}-promotion-{piece}',fen,base+piece))
result=[]
for name,fen,moves in cases:
 board=chess.Board(fen);history=[]
 for uci in moves.split():
  move=chess.Move.from_uci(uci);assert move in board.legal_moves;san=board.san(move);board.push(move);history.append({'uci':uci,'san':san,'fen':board.fen(en_passant='fen')})
 result.append({'name':name,'initial_fen':fen,'moves':history,'result':board.result()})
root=pathlib.Path('fixtures');root.mkdir(exist_ok=True);(root/'rules-reference.json').write_text(json.dumps({'generator':'python chess 1.11.2','cases':result},indent=2)+'\n')
print(f'Generated {len(result)} independent rule/notation histories')
