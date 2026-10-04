/* Execute the existing browser preprocessing on the native-decoded RGBA bytes.
   This file is a development reference runner, not an inference implementation. */
function sha256(bytes) {
  const K = [0x428a2f98,0x71374491,0xb5c0fbcf,0xe9b5dba5,0x3956c25b,0x59f111f1,0x923f82a4,0xab1c5ed5,0xd807aa98,0x12835b01,0x243185be,0x550c7dc3,0x72be5d74,0x80deb1fe,0x9bdc06a7,0xc19bf174,0xe49b69c1,0xefbe4786,0x0fc19dc6,0x240ca1cc,0x2de92c6f,0x4a7484aa,0x5cb0a9dc,0x76f988da,0x983e5152,0xa831c66d,0xb00327c8,0xbf597fc7,0xc6e00bf3,0xd5a79147,0x06ca6351,0x14292967,0x27b70a85,0x2e1b2138,0x4d2c6dfc,0x53380d13,0x650a7354,0x766a0abb,0x81c2c92e,0x92722c85,0xa2bfe8a1,0xa81a664b,0xc24b8b70,0xc76c51a3,0xd192e819,0xd6990624,0xf40e3585,0x106aa070,0x19a4c116,0x1e376c08,0x2748774c,0x34b0bcb5,0x391c0cb3,0x4ed8aa4a,0x5b9cca4f,0x682e6ff3,0x748f82ee,0x78a5636f,0x84c87814,0x8cc70208,0x90befffa,0xa4506ceb,0xbef9a3f7,0xc67178f2];
  const H=[0x6a09e667,0xbb67ae85,0x3c6ef372,0xa54ff53a,0x510e527f,0x9b05688c,0x1f83d9ab,0x5be0cd19];
  const data=new Uint8Array(Math.ceil((bytes.length+9)/64)*64);data.set(bytes);data[bytes.length]=128;
  const view=new DataView(data.buffer);view.setUint32(data.length-8,Math.floor(bytes.length*8/4294967296));view.setUint32(data.length-4,bytes.length*8>>>0);
  const W=new Int32Array(64),rotate=(v,n)=>(v>>>n)|(v<<(32-n));
  for(let start=0;start<data.length;start+=64){
    for(let i=0;i<16;i++)W[i]=view.getInt32(start+i*4);
    for(let i=16;i<64;i++){const a=W[i-15],b=W[i-2];W[i]=(W[i-16]+(rotate(a,7)^rotate(a,18)^(a>>>3))+W[i-7]+(rotate(b,17)^rotate(b,19)^(b>>>10)))|0;}
    let [a,b,c,d,e,f,g,h]=H;
    for(let i=0;i<64;i++){const t1=(h+(rotate(e,6)^rotate(e,11)^rotate(e,25))+((e&f)^(~e&g))+K[i]+W[i])|0,t2=((rotate(a,2)^rotate(a,13)^rotate(a,22))+((a&b)^(a&c)^(b&c)))|0;h=g;g=f;f=e;e=(d+t1)|0;d=c;c=b;b=a;a=(t1+t2)|0;}
    [a,b,c,d,e,f,g,h].forEach((v,i)=>H[i]=(H[i]+v)|0);
  }
  return H.map(v=>(v>>>0).toString(16).padStart(8,'0')).join('');
}
if(sha256(new Uint8Array())!=='e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855'||sha256(new Uint8Array([97,98,99]))!=='ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad')throw new Error('Reference hash self-check failed');
const input=JSON.parse(readFile(arguments[0]));
const core=ChessVisionCore,rgba=new Uint8Array(input.rgba),geometry=core.geometry(input.corners,input.width,input.height);
const report={schema_version:1,squares:geometry.squares,rotation:geometry.rotation,quality:core.viewQuality(geometry.project)};
for(const [name,margin,size,height] of [['occupancy',50,500,100],['pieces',200,800,200]]){
  const warped=core.warp(rgba,input.width,input.height,geometry.project,margin,size);
  const crops=Array.from({length:64},(_,i)=>name==='occupancy'?core.occupancyCrop(warped,Math.floor(i/8),i%8):core.pieceCrop(warped,Math.floor(i/8),i%8));
  const tensor=core.tensor(crops,100,height),bytes=new Uint8Array(tensor.length*4),view=new DataView(bytes.buffer);
  tensor.forEach((v,i)=>view.setFloat32(i*4,v,true));
  report[name]={crop_sha256:crops.map(c=>sha256(c.rgb)),coverage:crops.map(c=>c.coverage),tensor_sha256:sha256(bytes)};
}
print(JSON.stringify(report));
