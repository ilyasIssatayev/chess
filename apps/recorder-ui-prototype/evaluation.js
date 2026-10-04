const $ = selector => document.querySelector(selector);
const vision = new VisionClient();
const display = $("#frame-canvas"), ctx = display.getContext("2d");
const motionCanvas = document.createElement("canvas"), motionCtx = motionCanvas.getContext("2d", {willReadFrequently:true});
let prepared, files, index = 0, trace, running = false, busy = false, epoch = 0, modelVersion;
vision.ready.then(result => { modelVersion = result.version; $("#run-status").textContent = prepared ? "Local models ready. Replay the loaded recording." : "Local models ready. Select a prepared recording."; controls(); })
  .catch(error => { $("#run-status").textContent = error.message; });
function controls() {
  const complete = Boolean(trace && trace.frames.length === prepared?.bundle.frames.length && !running && !busy);
  $("#trace-details").hidden = !complete;
  if (complete && !$("#trace-json").value) $("#trace-json").value = JSON.stringify(ChessEvaluationCore.exportTrace(trace));
  $("#run").disabled = !prepared || !modelVersion || running || busy;
  $("#cancel").disabled = !running;
  $("#download").disabled = !trace || trace.frames.length !== prepared?.bundle.frames.length || running || busy;
  $("#previous").disabled = !prepared || !index || running || busy;
  $("#next").disabled = !prepared || index >= prepared.bundle.frames.length - 1 || running || busy;
  $("#bundle-files").disabled = running || busy;
  $("#threshold").disabled = running || busy;
}
async function loadFrame(frame) {
  const file = files.get(frame.file);
  if (!file || file.size > 16 * 1024 * 1024) throw new Error(`Missing or oversized frame: ${frame.file}`);
  const bytes = await file.arrayBuffer();
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest("SHA-256", bytes)), n => n.toString(16).padStart(2,"0")).join("");
  if (digest !== frame.sha256.toLowerCase()) throw new Error(`Frame hash failed: ${frame.file}. Prepare it again from the sealed media.`);
  const image = await createImageBitmap(new Blob([bytes], {type:"image/png"}));
  const media = prepared.bundle.session.media;
  if (image.width !== media.width_px || image.height !== media.height_px) { image.close(); throw new Error("Decoded image dimensions do not match calibration."); }
  return image;
}
function showImage(image, frame, i) {
  display.width = image.width; display.height = image.height; ctx.drawImage(image, 0, 0);
  $("#frame-label").textContent = `Frame ${frame.frame_index} · ${i + 1}/${prepared.bundle.frames.length}`;
  $("#clock-label").textContent = `Capture ${(frame.capture_us / 1e6).toFixed(3)} s`;
  const overlay = $("#eval-overlay"); overlay.replaceChildren(); overlay.setAttribute("viewBox", `0 0 ${image.width} ${image.height}`);
  const corners = prepared.corners, project = ChessVisionCore.homography([corners.a8, corners.h8, corners.h1, corners.a1].map(p => ({x:p.x*image.width,y:p.y*image.height})));
  for (let n = 0; n <= 8; n += 1) for (const [a,b] of [[project(n/8,0),project(n/8,1)],[project(0,n/8),project(1,n/8)]]) {
    const line = document.createElementNS("http://www.w3.org/2000/svg","line");
    Object.entries({x1:a.x,y1:a.y,x2:b.x,y2:b.y,class:"grid-line"}).forEach(([k,v])=>line.setAttribute(k,v)); overlay.append(line);
  }
}
function readout(result) {
  const lookup = Object.fromEntries(result.squares.map(s => [s.square,s]));
  const symbols = ["♙","♘","♗","♖","♕","♔","♟","♞","♝","♜","♛","♚"];
  const board = $("#eval-board"); board.replaceChildren();
  CameraRecorder.names.forEach((name,i) => {
    const e=lookup[name], probabilities=[e.empty_probability,...e.piece_probabilities], best=probabilities.indexOf(Math.max(...probabilities));
    const cell=document.createElement("div");cell.className=`chess-square ${(Math.floor(i/8)+i%8)%2?"dark":"light"} ${e.visible_probability<.5?"uncertain":""}`;
    cell.setAttribute("aria-label",`${name} ${best ? ChessVisionCore.classes[best-1] : "empty"}`);
    const symbol=document.createElement("span");symbol.textContent=best?symbols[best-1]:"";
    const label=document.createElement("small");label.textContent=name;cell.append(symbol,label);board.append(cell);
  });
  $("#model-detail").textContent=`${result.latency_ms} ms inference · ${result.squares.filter(s=>s.visible_probability<.5).length} uncertain squares. ${result.quality.warning}`;
}
function patches(image) {
  motionCanvas.width=Math.min(image.width,640);motionCanvas.height=Math.round(image.height*motionCanvas.width/image.width);
  motionCtx.drawImage(image,0,0,motionCanvas.width,motionCanvas.height);
  const corners=prepared.corners;
  const project=ChessVisionCore.homography([corners.a8,corners.h8,corners.h1,corners.a1].map(p=>({x:p.x*motionCanvas.width,y:p.y*motionCanvas.height})));
  return CameraRecorder.sampleSquares(motionCtx.getImageData(0,0,motionCanvas.width,motionCanvas.height).data,motionCanvas.width,motionCanvas.height,project);
}
async function inspect(i) {
  if (running || busy || !prepared) return;
  busy=true;controls();let image;
  try {
    image=await loadFrame(prepared.bundle.frames[i]);
    const result=await vision.infer(image,prepared.corners);
    index=i;showImage(image,prepared.bundle.frames[i],i);readout(result);
    $("#frame-detail").textContent="Individual-frame inspection. Replay the recording to build a complete, timestamped trace.";
  } catch(error) { $("#run-status").textContent=error.message; }
  finally { image?.close();busy=false;controls(); }
}
async function acceptFiles(selected) {
  if (running || busy) return;
  busy=true;epoch+=1;trace=null;prepared=null;files=new Map();$("#trace-json").value="";controls();
  try {
    if (selected.length>2002 || selected.reduce((sum,f)=>sum+f.size,0)>514*1024*1024) throw new Error("Recording exceeds the 2000-frame/512 MiB preparation limit.");
    selected.forEach(file=>{
      const relative=file.webkitRelativePath.split("/").slice(1).join("/");
      if (files.has(relative)) throw new Error("Duplicate recording file paths.");
      files.set(relative,file);
    });
    const bundle=files.get("bundle.json");
    if (!bundle || bundle.size>2*1024*1024) throw new Error("Select the folder containing bundle.json and frames/.");
    prepared=ChessEvaluationCore.validateBundle(JSON.parse(await bundle.text()));
    if (prepared.bundle.frames.some(f=>!files.has(f.file))) throw new Error("The recording folder is missing required frame images.");
    const s=prepared.bundle.session;
    $("#bundle-status").textContent=`${s.session_id} · ${s.split} · ${prepared.bundle.frames.length} frames · ${s.timestamp_source.replaceAll("_"," ")} · base-model evaluation.`;
    $("#progress").value=0;$("#progress").max=prepared.bundle.frames.length;
    index=0;busy=false;await inspect(0);
    if (modelVersion) $("#run-status").textContent="Local models ready. Replay the loaded recording.";
  }catch(error){prepared=null;$("#bundle-status").textContent=error.message;}
  finally{busy=false;controls();}
}
$("#bundle-files").addEventListener("change", event => acceptFiles(Array.from(event.target.files)));
$("#previous").addEventListener("click",()=>inspect(index-1));
$("#next").addEventListener("click",()=>inspect(index+1));
$("#cancel").addEventListener("click",()=>{epoch+=1;$("#run-status").textContent="Cancelling replay. Partial traces cannot be scored; run again to complete it.";});
$("#run").addEventListener("click",async()=>{
  if (!prepared || running || busy || !modelVersion) return;
  const threshold=Number($("#threshold").value);
  if (!Number.isFinite(threshold) || threshold<6 || threshold>24) {$("#run-status").textContent="Choose a motion threshold from 6 to 24.";return;}
  running=true;$("#trace-json").value="";const run=++epoch;controls();let previous;
  trace=ChessEvaluationCore.begin(prepared.bundle,modelVersion,threshold,crypto.randomUUID());
  try {
    for (let i=0;i<prepared.bundle.frames.length;i+=1) {
      if (run!==epoch) break;
      const frame=prepared.bundle.frames[i];let image;
      try {
        image=await loadFrame(frame);
        const current=patches(image), moving=Boolean(previous && CameraRecorder.changes(previous,current,threshold/2).some(e=>e.changed));
        const result=await vision.infer(image,prepared.corners);
        if (run!==epoch) break;
        ChessEvaluationCore.append(trace,frame,result,moving);previous=current;index=i;
        showImage(image,frame,i);readout(result);
        $("#frame-detail").textContent=`${moving?"Motion":"No measured motion"} · acquisition timestamp ${frame.capture_us} µs · source frame index ${frame.frame_index}.`;
        $("#progress").value=i+1;$("#run-status").textContent=`Recognized ${i+1}/${prepared.bundle.frames.length} verified frames.`;
      }finally{image?.close();}
    }
    if (run===epoch) $("#run-status").textContent="Recognition replay complete. Download observations and score them against the independent annotations.";
  }catch(error){$("#run-status").textContent=error.message;trace=null;}
  finally{running=false;controls();}
});
$("#download").addEventListener("click",()=>{
  if (!trace || trace.frames.length!==prepared.bundle.frames.length) return;
  const blob=new Blob([JSON.stringify(ChessEvaluationCore.exportTrace(trace))],{type:"application/json"}),url=URL.createObjectURL(blob);
  const a=document.createElement("a");a.href=url;a.download=`observations-${trace.session_id.replaceAll(/[^a-zA-Z0-9_-]/g,"_")}.json`;a.click();
  setTimeout(()=>URL.revokeObjectURL(url),1000);
});
controls();

// The loopback test server alone serves this fixed, public regression fixture.
// Production never serves recording directories or accepts uploaded images.
if (new URLSearchParams(location.search).get("fixture") === "public") {
  (async () => {
    try {
      const response = await fetch("/evaluation-fixture/bundle.json");
      if (!response.ok) throw new Error("Public regression fixture requires the isolated test server.");
      const bundle = await response.json();
      ChessEvaluationCore.validateBundle(bundle);
      if (bundle.frames.length > 20) throw new Error("Regression fixture is too large.");
      const wrap = (name, blob) => ({webkitRelativePath:`fixture/${name}`,size:blob.size,arrayBuffer:()=>blob.arrayBuffer(),text:()=>blob.text()});
      const selected = [wrap("bundle.json",new Blob([JSON.stringify(bundle)]))];
      for (const frame of bundle.frames) {
        const image = await fetch(`/evaluation-fixture/${frame.file}`);
        if (!image.ok) throw new Error("Public fixture frame unavailable.");
        selected.push(wrap(frame.file,await image.blob()));
      }
      await acceptFiles(selected);
    } catch(error) {$("#bundle-status").textContent=error.message;}
  })();
}
