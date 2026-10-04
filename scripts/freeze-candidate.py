#!/usr/bin/env python3
"""Hash a development candidate; physical qualification still controls release."""
import hashlib,json,pathlib,subprocess,sys
app=pathlib.Path(sys.argv[1]).resolve(); destination=pathlib.Path(sys.argv[2])
files=[{"file":str(p.relative_to(app)),"bytes":p.stat().st_size,"sha256":hashlib.sha256(p.read_bytes()).hexdigest()} for p in sorted(app.rglob('*')) if p.is_file()]
result={"version":1,"candidate_kind":"development","release_qualified":False,"source_commit":subprocess.check_output(['git','rev-parse','HEAD'],text=True).strip(),"source_has_changes":bool(subprocess.check_output(['git','status','--porcelain'],text=True).strip()),"platform":"macos-arm64","files":files,"remaining_gates":["physical placement","independent validation and threshold selection","60-minute representative live soak","held-out original-decision qualification","clean target Mac and release distribution"]}
destination.write_text(json.dumps(result,indent=2)+'\n'); print(f'Candidate hashes: {destination}')
