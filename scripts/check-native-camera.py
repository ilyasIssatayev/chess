#!/usr/bin/env python3
"""Measure native timestamp delivery without saving camera images."""
import argparse,json,pathlib,select,subprocess,time
p=argparse.ArgumentParser();p.add_argument('--helper',default='target/native/chess-camera');p.add_argument('--frames',type=int,default=120);p.add_argument('--output',default='local-data/native-camera-check/report.json');a=p.parse_args()
assert 1<=a.frames<=1800
child=subprocess.Popen([a.helper],stdout=subprocess.PIPE,stderr=subprocess.PIPE);frames=[];deadline=time.monotonic()+55
try:
 for i in range(a.frames):
  if not select.select([child.stdout],[],[],max(0,deadline-time.monotonic()))[0]:raise RuntimeError('Camera sample timed out')
  line=child.stdout.readline(1024)
  if not line:raise RuntimeError(child.stderr.read(4096).decode(errors='replace') or 'Camera closed')
  h=json.loads(line);assert h['timestamp_source']=='avfoundation_presentation_time' and h['version']==1
  assert 2<=h['width']<=1920 and 2<=h['height']<=1080 and h['bytes']==h['width']*h['height']*4
  remaining=h['bytes']
  while remaining:
   chunk=child.stdout.read(min(remaining,262144))
   if not chunk:raise RuntimeError('Truncated frame')
   remaining-=len(chunk)
  if frames:assert h['capture_time_us']>frames[-1]['capture_time_us'] and h['sequence']>frames[-1]['sequence']
  frames.append(h)
 gaps=[b['capture_time_us']-a['capture_time_us'] for a,b in zip(frames,frames[1:])]
 result={'frames':len(frames),'width':frames[0]['width'],'height':frames[0]['height'],'timestamp_source':'avfoundation_presentation_time','observed_fps':(len(frames)-1)*1e6/(frames[-1]['capture_time_us']-frames[0]['capture_time_us']) if len(frames)>1 else None,'max_interval_us':max(gaps) if gaps else None,'camera_reported_drops':frames[-1]['dropped'],'images_saved':False,'physical_board_qualified':False}
 output=pathlib.Path(a.output);output.parent.mkdir(parents=True,exist_ok=True);output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result))
finally:
 child.terminate()
 try:child.wait(timeout=3)
 except subprocess.TimeoutExpired:child.kill();child.wait()
