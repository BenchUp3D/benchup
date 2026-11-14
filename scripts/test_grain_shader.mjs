#!/usr/bin/env node
// Validate the real grain shader on a hardware WebGPU adapter; never build.
import {readFile, readdir, mkdtemp, rm} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import {tmpdir, homedir} from 'node:os';
import {join} from 'node:path';
const cache=join(homedir(),'.cache/ms-playwright');
const version=(await readdir(cache)).filter(n=>/^chromium-\d+$/.test(n)).sort().reverse()[0];
const executable=process.env.CHROME_BIN??join(cache,version,'chrome-linux64/chrome');
const profile=await mkdtemp(join(tmpdir(),'benchup-grain-shader-'));
const browser=spawn(executable,['--headless=new','--no-sandbox','--disable-dev-shm-usage',
 '--enable-unsafe-webgpu','--ignore-gpu-blocklist','--use-angle=vulkan','--enable-features=Vulkan',
 '--remote-debugging-port=0',`--user-data-dir=${profile}`,'about:blank'],{stdio:['ignore','ignore','ignore']});
let socket;
try {
 const deadline=Date.now()+15000;
 let port;
 while(!port&&Date.now()<deadline){try{port=(await readFile(join(profile,'DevToolsActivePort'),'utf8')).split('\n')[0]}catch{await new Promise(r=>setTimeout(r,100))}}
 if(!port)throw Error('Chromium did not start');
 const tabs=await(await fetch(`http://127.0.0.1:${port}/json/list`)).json();
 socket=new WebSocket(tabs.find(t=>t.type==='page').webSocketDebuggerUrl);
 await new Promise((resolve,reject)=>{socket.onopen=resolve;socket.onerror=reject});
 let sequence=0;
 const pending=new Map();
 socket.onmessage=event=>{const m=JSON.parse(event.data);const p=pending.get(m.id);if(p){pending.delete(m.id);m.error?p.reject(Error(JSON.stringify(m.error))):p.resolve(m.result)}};
 const call=(method,params={})=>new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});socket.send(JSON.stringify({id,method,params}))});
 await call('Page.navigate',{url:'http://127.0.0.1:8080/'});
 const source=(await Promise.all(['surface_appearance.wgsl','edges.wgsl'].map(n=>readFile(new URL('../crates/benchup-app/src/'+n,import.meta.url),'utf8')))).join('\n');
 const pairs=Array.from({length:32},(_,i)=>[i%2?-i*31:i*47,i%3?i-17:-i*113]);
 const expected=pairs.map(([column,row])=>{
  const mask=(1n<<64n)-1n;
  const key=(BigInt(column)*0x9e3779b97f4a7c15n&mask)^(BigInt(row)*0xbf58476d1ce4e5b9n&mask);
  const mixed=(key^(key>>30n))*0x94d049bb133111ebn&mask;
  return Number((mixed^(mixed>>31n))>>48n);
 });
 const probe=`
 @group(2) @binding(0) var<storage,read_write> grain_results: array<vec4<u32>>;
 @compute @workgroup_size(32) fn grain_probe(@builtin(global_invocation_id) id: vec3<u32>) {
  let pairs = array<vec2<i32>,32>(${pairs.map(([c,r])=>`vec2<i32>(${c},${r})`).join(',')});
  let samples = array<vec2<f32>,8>(vec2<f32>(0.0,0.0),vec2<f32>(0.2,0.0),vec2<f32>(0.6,0.0),vec2<f32>(1.35,0.0),
   vec2<f32>(0.0,0.015),vec2<f32>(0.0,0.025),vec2<f32>(0.0,0.1),vec2<f32>(0.0,1.0));
  let index=id.x;
  let end=f32((index/8u)%2u);
  let uv=samples[index%8u];
  let coverage=grain_direction_coverage(vec4<f32>(uv,end,1.0),vec4<f32>(0.01,0.0,0.0,0.0),vec4<f32>(0.0,0.01,0.0,0.0),0.85);
  grain_results[index]=vec4<u32>(u32(round((grain_variation(pairs[index].x,pairs[index].y)+1.0)*32767.5)),u32(coverage>0.01),0u,0u);
 }`;
 const expression=`(${async function(source,expected){
  if(!navigator.gpu)throw Error('WebGPU is unavailable');
  const adapter=await navigator.gpu.requestAdapter();
  if(!adapter||adapter.info.isFallbackAdapter)throw Error('A hardware GPU is required');
  const device=await adapter.requestDevice();
  try{
   const module=device.createShaderModule({code:source});
   const info=await module.getCompilationInfo();
   const errors=info.messages.filter(m=>m.type==='error');
   if(errors.length)throw Error(errors.map(m=>m.message).join('\n'));
   const pipeline=await device.createComputePipelineAsync({layout:'auto',compute:{module,entryPoint:'grain_probe'}});
   const output=device.createBuffer({size:512,usage:GPUBufferUsage.STORAGE|GPUBufferUsage.COPY_SRC});
   const readback=device.createBuffer({size:512,usage:GPUBufferUsage.COPY_DST|GPUBufferUsage.MAP_READ});
   const bind=device.createBindGroup({layout:pipeline.getBindGroupLayout(2),entries:[{binding:0,resource:{buffer:output}}]});
   const encoder=device.createCommandEncoder();const pass=encoder.beginComputePass();
   pass.setPipeline(pipeline);pass.setBindGroup(2,bind);pass.dispatchWorkgroups(1);pass.end();
   encoder.copyBufferToBuffer(output,0,readback,0,512);device.queue.submit([encoder.finish()]);
   await readback.mapAsync(GPUMapMode.READ);const values=new Uint32Array(readback.getMappedRange());
   for(let i=0;i<32;i++){
    if(values[i*4]!==expected[i])throw Error(`Grain hash differs at ${i}: ${values[i*4]} != ${expected[i]}`);
    const marks=i/8%2<1?[1,1,0,1,0,0,0,0]:[1,0,0,0,0,0,0,1];
    if(values[i*4+1]!==marks[i%8])throw Error(`Grain coverage differs at ${i}: ${values[i*4+1]} != ${marks[i%8]}`);
   }
   readback.unmap();return {passed:true,hash_cases:32,coverage_cases:32,adapter:adapter.info.vendor};
  }finally{device.destroy()}
 }.toString()})(${JSON.stringify(source+probe)},${JSON.stringify(expected)})`;
 const result=await call('Runtime.evaluate',{expression,awaitPromise:true,returnByValue:true});
 if(result.exceptionDetails)throw Error(result.exceptionDetails.exception?.description??JSON.stringify(result.exceptionDetails));
 console.log(JSON.stringify(result.result.value,null,2));
} finally {
 socket?.close();browser.kill();
 await new Promise(resolve=>browser.exitCode!==null?resolve():browser.once('exit',resolve));
 await rm(profile,{recursive:true,force:true,maxRetries:5,retryDelay:100});
}
