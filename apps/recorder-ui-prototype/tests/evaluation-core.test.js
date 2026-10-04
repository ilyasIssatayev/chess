let count=0;const assert=(condition,message="assertion failed")=>{if(!condition)throw new Error(message);};
function test(name,action){action();count+=1;print(`PASS ${name}`);}
function rejects(action){try{action();}catch(_){return;}throw new Error("Expected rejection");}
function fixture(){
 const session={session_id:"test",split:"validation",timestamp_source:"host_acquisition",media:{width_px:1000,height_px:800,sha256:"a".repeat(64)},frames:[{frame_index:0,capture_us:0},{frame_index:1,capture_us:33000}],board:{orientation:"a1_near_left",corners:{near_left:{x_px:100,y_px:700},near_right:{x_px:900,y_px:700},far_right:{x_px:800,y_px:100},far_left:{x_px:200,y_px:100}}}};
 return {schema_version:1,kind:"chess-frame-bundle",dataset_id:"test",source_sha256:"a".repeat(64),session,frames:session.frames.map((f,i)=>({...f,file:`frames/00000${i}.png`,sha256:"b".repeat(64)}))};
}
test("calibration maps all four a1 orientations to the model's labelled corners",()=>{
 const bundle=fixture(),perimeter=[bundle.session.board.corners.near_left,bundle.session.board.corners.near_right,bundle.session.board.corners.far_right,bundle.session.board.corners.far_left];
 ["a1_near_left","a1_near_right","a1_far_right","a1_far_left"].forEach((name,i)=>{bundle.session.board.orientation=name;const {corners}=ChessEvaluationCore.validateBundle(bundle);assert(corners.a1.x===perimeter[i].x_px/1000);assert(corners.h1.y===perimeter[(i+1)%4].y_px/800);});
});
test("missing frames, changed timestamps, unsafe paths and media hash mismatches are rejected",()=>{
 let bundle=fixture();bundle.frames.pop();rejects(()=>ChessEvaluationCore.validateBundle(bundle));
 bundle=fixture();bundle.frames[1].capture_us=33001;rejects(()=>ChessEvaluationCore.validateBundle(bundle));
 bundle=fixture();bundle.frames[1].file="../private.png";rejects(()=>ChessEvaluationCore.validateBundle(bundle));
 bundle=fixture();bundle.source_sha256="c".repeat(64);rejects(()=>ChessEvaluationCore.validateBundle(bundle));
});
test("qualification data cannot be opened by a development evaluator",()=>{const b=fixture();b.session.split="qualification";rejects(()=>ChessEvaluationCore.validateBundle(b));});
test("capture timestamps and source indexes are preserved despite inference latency and gaps",()=>{
 const bundle=fixture();bundle.frames[1].frame_index=4;bundle.session.frames[1].frame_index=4;ChessEvaluationCore.validateBundle(bundle);
 const trace=ChessEvaluationCore.begin(bundle,"base",10,"capture-id");
 const result={version:"base",latency_ms:10000,squares:[],personalized:false};
 ChessEvaluationCore.append(trace,bundle.frames[1],result,true);
 assert(trace.frames[0].observation.capture_time===33000);assert(trace.frames[0].observation.sequence===4);assert(trace.frames[0].inference_ms===10000);
 const exported=ChessEvaluationCore.exportTrace(trace);assert(!("captureId" in exported));assert(exported.frames[0].observation.session_id==="capture-id");
});
test("changed or personal models do not contaminate a base-model evaluation trace",()=>{
 const b=fixture(),t=ChessEvaluationCore.begin(b,"base",10,"id");
 rejects(()=>ChessEvaluationCore.append(t,b.frames[0],{version:"other",squares:[],latency_ms:1},false));
 rejects(()=>ChessEvaluationCore.append(t,b.frames[0],{version:"base",personalized:true,squares:[],latency_ms:1},false));
 assert(!t.frames.length);
});
print(`${count} offline evaluation tests passed`);
