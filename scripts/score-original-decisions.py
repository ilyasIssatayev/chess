#!/usr/bin/env python3
"""Development audit scoring. Never scores the corrected active history as automatic."""
import argparse,hashlib,json,math,pathlib

def score(reference,audit):
    decisions=audit['original_decisions']; automatic=[m for m in decisions if m['provenance']=='automatic'];seen=set();correct=0;errors=[]
    # Count every original decision, including duplicates and superseded revisions.
    for m in automatic:
        index=m['ply']-1
        if 0<=index<len(reference) and m['uci']==reference[index]['uci'] and index not in seen:
            seen.add(index);correct+=1
        else:errors.append({'ply':m['ply'],'uci':m['uci'],'revision':m['revision'],'reason':'wrong, extra or duplicate original decision'})
    total=len(automatic);count=len(reference)
    interval=None
    if total:
        z=1.959963984540054;p=correct/total;center=(p+z*z/(2*total))/(1+z*z/total);half=z*math.sqrt(p*(1-p)/total+z*z/(4*total*total))/(1+z*z/total);interval=[max(0,center-half),min(1,center+half)]
    exact=len(decisions)==count and all(m['provenance']=='automatic' and m['ply']==i+1 and m['uci']==reference[i]['uci'] for i,m in enumerate(decisions))
    return {'original_automatic_decisions':total,'reference_plies':count,'correct_automatic_plies':correct,'automatic_errors':len(errors),'precision':correct/total if total else None,'coverage':correct/count if count else None,'precision_wilson_95':interval,'unattended_exact_game':exact if count else None,'errors':errors,'scoring_policy':'ply/UCI original-decision matching; corrections never remove decisions; no sequence repair','release_qualified':False,'limitations':['No position-chain cross-check or acquisition-clock alignment in this development report','Timing and release qualification require the locked independent evaluator and annotated complete sessions']}

def main():
 p=argparse.ArgumentParser(description=__doc__);p.add_argument('manifest');p.add_argument('session_id');p.add_argument('audit');p.add_argument('output');a=p.parse_args()
 manifest=json.loads(pathlib.Path(a.manifest).read_text());sessions=[s for s in manifest['sessions'] if s['session_id']==a.session_id];assert len(sessions)==1,'Select exactly one annotated session';session=sessions[0]
 assert session['split']!='qualification','Use a frozen independent qualification evaluator for qualification data'
 audit=json.loads(pathlib.Path(a.audit).read_text());report=score(session['reference_game']['moves'],audit)
 report.update({'session_id':a.session_id,'manifest_sha256':hashlib.sha256(pathlib.Path(a.manifest).read_bytes()).hexdigest(),'audit_sha256':hashlib.sha256(pathlib.Path(a.audit).read_bytes()).hexdigest()})
 pathlib.Path(a.output).write_text(json.dumps(report,indent=2)+'\n');print(json.dumps(report))
if __name__=='__main__':main()
