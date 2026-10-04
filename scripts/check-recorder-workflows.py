import json,subprocess,time,urllib.request,pathlib,tempfile
root=pathlib.Path(tempfile.mkdtemp(prefix='chess-http-smoke-'));env=__import__('os').environ.copy();env['CHESS_RECORDER_DATA_DIR']=str(root)
child=subprocess.Popen(['target/release/chess-camera-recorder','serve','8773'],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
def req(path,body=None):
 r=urllib.request.Request('http://127.0.0.1:8773'+path,data=json.dumps(body).encode() if body is not None else None,headers={'Content-Type':'application/json'} if body is not None else {})
 with urllib.request.urlopen(r,timeout=10) as f:return json.loads(f.read()) if 'application/json' in f.headers['Content-Type'] else f.read()
try:
 for _ in range(100):
  try:g=req('/api/game');break
  except Exception:time.sleep(.1)
 else:raise RuntimeError(child.stderr.read().decode())
 def pos():return {'game_id':g['game_id'],'revision':g['revision'],'expected_ply':len(g['moves'])}
 for uci in ['e2e4','e7e5','g1f3','b8c6']:g=req('/api/game/move',{**pos(),'uci':uci})
 import socket
 idle=socket.create_connection(('127.0.0.1',8773)); started=time.monotonic(); assert len(req('/api/games'))==1; assert time.monotonic()-started<1;idle.close()
 g=req('/api/game/correct',{**pos(),'replace_from_ply':1,'uci':'g1f3','reason':'Correct the opening move'})
 assert g['first_invalid_ply']==3 and len(g['moves'])==2
 assert len(req('/api/game/audit')['original_decisions'])==4
 assert b'Incomplete recording' in req('/api/game/pgn')
 assert req('/api/game/timing.json')['moves'][0]['timing']['quality']=='unknown'
 assert b'elapsed_earliest_us' in req('/api/game/timing.csv')
 backup=req('/api/backup',{})['path'];assert pathlib.Path(backup,'backup.json').exists()
 req('/api/game/new',{});assert len(req('/api/games'))==2
 req('/api/restore',{'backup_id':pathlib.Path(backup).name});assert len(req('/api/games'))==1
 assert root.joinpath('active-profile').is_file() and root.joinpath('games.sqlite').is_file()
 for path in ['/library.html','/library.js','/library-core.js']:assert len(req(path))>100
 child.terminate();child.wait(timeout=5)
 child=subprocess.Popen(['target/release/chess-camera-recorder','serve','8773'],env=env,stdout=subprocess.PIPE,stderr=subprocess.PIPE)
 for _ in range(50):
  try:g=req('/api/game');break
  except Exception:time.sleep(.1)
 assert len(g['moves'])==2 and g['revision']==1
 print('HTTP smoke passed: idle sockets, persistence, corrections, immutable originals, library assets, exports, backup, restore, restored-profile restart')
 print('Isolated test data:',root)
finally:
 child.terminate();child.wait(timeout=5)
