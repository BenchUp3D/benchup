#!/usr/bin/env node
// Benchmark an existing running app; never starts a build. Isolated browser data.
import {readFile, writeFile, mkdtemp, rm, access, readdir, mkdir} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import {join, dirname} from 'node:path';
import {tmpdir, homedir} from 'node:os';
import {createHash} from 'node:crypto';
const args=process.argv.slice(2);
const option=(key, fallback)=>{const i=args.indexOf(key);return i<0?fallback:args[i+1]};
const project=option('--project');
const usage='Usage: node scripts/benchmark_viewport_web.mjs --project FILE [--mode orbit|select|groups|draw-line|draw-rectangle|measure-tape|walk] [--seconds 20] [--url http://127.0.0.1:8080] [--output /tmp/viewport.json] [--browser PATH] [--headed] [--unsafe-webgpu] [--x11] [--preferences FILE] [--legacy] [--profile] [--min-fps FPS] [--validate-shader] [--validate-state] [--validate-snapping] [--after-walk] [--require-pointer-lock]';
if(args.includes('--help')){console.log(usage);process.exit(0)}
if(!project)throw Error(usage);
const mode=option('--mode','orbit');
if(!['orbit','select','groups','draw-line','draw-rectangle','measure-tape','walk'].includes(mode))throw Error('--mode must be orbit, select, groups, draw-line, draw-rectangle, measure-tape, or walk');
const model=await readFile(project,'utf8');
const projectData=JSON.parse(model);
const preferenceSource=option('--preferences');
// Raw ketchup.inspector.v1 RON, copied out of storage rather than a live profile.
const inspectorPreferences=preferenceSource?await readFile(preferenceSource,'utf8'):null;
const seconds=Number(option('--seconds','20'));
if(!Number.isFinite(seconds)||seconds<4||seconds>120)throw Error('--seconds must be between 4 and 120');
const output=option('--output','/tmp/ketchup-web-viewport.json');
const explicitBrowser=option('--browser',process.env.CHROME_BIN);
const cache=join(homedir(),'.cache/ms-playwright');
const cachedBrowsers=(await readdir(cache).catch(()=>[])).filter(name=>/^chromium-\d+$/.test(name)).sort().reverse()
 .flatMap(name=>['chrome-linux64/chrome','chrome-linux/chrome'].map(path=>join(cache,name,path)));
let executable;
for(const candidate of explicitBrowser?[explicitBrowser]:['/usr/bin/chromium','/usr/bin/google-chrome',...cachedBrowsers]){
 try{await access(candidate);executable=candidate;break}catch{}
}
if(!executable)throw Error('Chromium not found; set CHROME_BIN or --browser PATH');
await mkdir(dirname(output),{recursive:true});
const profile=await mkdtemp(join(tmpdir(),'ketchup-viewport-'));
const browser=spawn(executable,[...(args.includes('--headed')?[]:['--headless=new']),'--no-sandbox','--disable-dev-shm-usage',
 ...(args.includes('--unsafe-webgpu')?['--enable-unsafe-webgpu','--ignore-gpu-blocklist']:[]),...(args.includes('--vulkan')?['--use-angle=vulkan','--enable-features=Vulkan']:[]),'--disable-background-timer-throttling','--disable-renderer-backgrounding','--disable-backgrounding-occluded-windows',
 ...(args.includes('--x11')?['--ozone-platform=x11']:[]),
 '--no-first-run','--no-default-browser-check','--force-device-scale-factor=1','--window-size=1440,900','--remote-debugging-port=0',
 `--user-data-dir=${profile}`,'about:blank'],{stdio:['ignore','ignore','pipe']});
let diagnostics='';
browser.stderr.on('data',chunk=>{diagnostics=(diagnostics+chunk).slice(0,1000000)});
browser.on('error',error=>{diagnostics+=error.message});
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(fn, label, timeout = 60000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) { const value = await fn(); if (value) return value; await pause(200); }
  throw Error(`Timed out waiting for ${label}\n${diagnostics}`);
}
let socket;
try {
  const port = await until(async () => {
    try { return (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; } catch { return null; }
  }, 'Chromium startup');
  // Vivaldi advertises its initial tab before its browser UI has attached
  // the WebView. Wait for that UI before enabling page-domain events.
  if(/vivaldi/i.test(executable))await pause(3000);
  let tabs = await until(async()=>{
   const list=await(await fetch(`http://127.0.0.1:${port}/json/list`)).json();
   return list.some(tab=>tab.type==='page'&&tab.url==='about:blank')?list:null;
  },'initial browser tab',15000);
  if(/vivaldi/i.test(executable)){
   // Create a tab through Vivaldi's UI so it is attached and actively painted.
   const old=new Set(tabs.map(tab=>tab.id));
   const shell=tabs.find(tab=>tab.type==='app'&&tab.url.endsWith('/main.html'));
   if(!shell)throw Error('Vivaldi browser UI is unavailable');
   const ui=new WebSocket(shell.webSocketDebuggerUrl);
   await new Promise((resolve,reject)=>{ui.onopen=resolve;ui.onerror=reject});
   try{await new Promise((resolve,reject)=>{
    const timer=setTimeout(()=>reject(Error('Vivaldi tab creation timed out')),10000);
    ui.onmessage=e=>{const m=JSON.parse(e.data);if(m.id===1){clearTimeout(timer);if(m.result?.exceptionDetails||m.error)reject(Error(JSON.stringify(m)));else resolve(m.result)}};
    ui.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:"new Promise(resolve=>chrome.tabs.create({url:'about:blank',active:true},resolve))",awaitPromise:true,returnByValue:true}}));
   })}finally{ui.close()}
   await pause(2000);
   tabs=await until(async()=>{
    const list=await(await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    return list.some(tab=>tab.type==='page'&&tab.url==='about:blank'&&!old.has(tab.id))?list.filter(tab=>!old.has(tab.id)):null;
   },'active Vivaldi tab',10000);
  }
  socket = new WebSocket((tabs.find(tab => tab.type==='page'&&tab.url==='about:blank')??tabs.find(tab=>tab.type==='page')).webSocketDebuggerUrl);
  await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
  let sequence = 0;
  const pending = new Map();
  socket.onmessage = event => {
    const message = JSON.parse(event.data);
    if (message.method === 'Runtime.exceptionThrown') diagnostics += '\n' + JSON.stringify(message.params);
    if (message.method === 'Runtime.consoleAPICalled') diagnostics += '\n' + message.params.args.map(a=>a.value ?? a.description).join(' ');
    if (message.id) {
      const entry = pending.get(message.id); pending.delete(message.id); if(!entry)return;
      if (message.error) entry.reject(Error(JSON.stringify(message.error))); else entry.resolve(message.result);
    }
  };
  const call = (method, params = {}) => new Promise((resolve, reject) => {
    const id = ++sequence;
    const timer = setTimeout(() => { pending.delete(id); reject(Error(`CDP timeout: ${method}`)); }, 60000);
    pending.set(id, { resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); } });
    socket.send(JSON.stringify({ id, method, params }));
  });

 if(args.includes('--profile')){await call('Profiler.enable');await call('Profiler.setSamplingInterval',{interval:1000});await call('Profiler.start');}
 await call('Page.enable');
 await call('Runtime.enable');
 if(mode==='walk'||args.includes('--after-walk')){
  const protocol=await(await fetch(`http://127.0.0.1:${port}/json/protocol`)).json();
  const supported=protocol.domains.find(d=>d.domain==='Browser')?.types.find(t=>t.id==='PermissionType')?.enum??[];
  const permissions=['pointerLock','keyboardLock'].filter(p=>supported.includes(p));
  if(permissions.length)await call('Browser.grantPermissions',{origin:new URL(option('--url','http://127.0.0.1:8080/')).origin,permissions});
 }
 // Pin the loaded artifact: development-server reloads must not reset a run.
 await call('Network.enable');
 await call('Network.setBlockedURLs',{urls:['*://*/.well-known/trunk/ws']});
 if(/vivaldi/i.test(executable)){
  // Vivaldi's WebGL surface keeps its native dimensions under CDP emulation.
  // Resize the real window so the canvas and captured viewport agree.
  const {windowId,bounds}=await call('Browser.getWindowForTarget');
  const inner=await call('Runtime.evaluate',{expression:'({width:innerWidth,height:innerHeight})',returnByValue:true});
  await call('Browser.setWindowBounds',{windowId,bounds:{width:bounds.width+1440-inner.result.value.width,height:bounds.height+900-inner.result.value.height}});
 }else await call('Emulation.setDeviceMetricsOverride',{width:1440,height:900,deviceScaleFactor:1,mobile:false});
 const installSeed=async (source,mode)=>await call('Page.addScriptToEvaluateOnNewDocument',{source:
 `{let value=${JSON.stringify(source)};
 window.ketchupBenchmarkLoadNonce=Math.random().toString(36);
 const NativeWebSocket=window.WebSocket;
 window.WebSocket=new Proxy(NativeWebSocket,{construct(target,args,newTarget){if(String(args[0]).includes('/.well-known/trunk/ws'))return Object.assign(new EventTarget(),{readyState:3,close(){},send(){}});return Reflect.construct(target,args,newTarget)}});
 window.ketchupBenchmarkDocument=()=>value;
 if(globalThis.Element?.prototype.requestPointerLock){const lock=Element.prototype.requestPointerLock;Element.prototype.requestPointerLock=function(...args){try{const result=lock.apply(this,args);return result?.catch?result.catch(error=>{window.ketchupPointerLockError=String(error);throw error}):result}catch(error){window.ketchupPointerLockError=String(error);throw error}}}
 window.ketchupStorageWrites={document:0,selection:0};
 {let frame=0,last=-1;const probe=window.ketchupDrawingProbe={samples:[],record:false,walkMutations:0};
 const submitted=()=>{if(probe.record&&last!==frame){probe.samples.push(performance.now());last=frame}};
 const raf=()=>{frame++;requestAnimationFrame(raf)};requestAnimationFrame(raf);
 document.addEventListener('DOMContentLoaded',()=>{const canvas=document.getElementById('ketchup-canvas');if(canvas)new MutationObserver(changes=>{if(probe.record)probe.walkMutations+=changes.length}).observe(canvas,{attributes:true,attributeFilter:['data-walk-active','data-walk-input-enabled','data-walk-placed']})});
 if(globalThis.GPUQueue){const submit=GPUQueue.prototype.submit;GPUQueue.prototype.submit=function(...args){submitted();return submit.apply(this,args)}}
 if(globalThis.WebGL2RenderingContext){for(const method of ['drawArrays','drawElements','drawArraysInstanced','drawElementsInstanced']){const draw=WebGL2RenderingContext.prototype[method];if(draw)WebGL2RenderingContext.prototype[method]=function(...args){if(this.canvas.id==='ketchup-canvas')submitted();return draw.apply(this,args)}}}
 const getContext=HTMLCanvasElement.prototype.getContext;
 HTMLCanvasElement.prototype.getContext=function(kind,...args){const context=getContext.call(this,kind,...args);if(context&&kind==='webgl2'&&this.id==='ketchup-canvas'){const info=context.getExtension('WEBGL_debug_renderer_info');const renderer=context.getParameter(info?info.UNMASKED_RENDERER_WEBGL:context.RENDERER);window.ketchupViewportAdapter={backend:'webgl2',vendor:context.getParameter(info?info.UNMASKED_VENDOR_WEBGL:context.VENDOR),renderer,isFallbackAdapter:/swiftshader|llvmpipe|software/i.test(renderer)}}return context};}
 const get=Storage.prototype.getItem, set=Storage.prototype.setItem; Storage.prototype.getItem=function(k){return k==='ketchup.document.v1'?value:k==='ketchup.inspector.v1'&&${JSON.stringify(inspectorPreferences)}!==null?${JSON.stringify(inspectorPreferences)}:get.call(this,k)}; Storage.prototype.setItem=function(k,v){if(k==='ketchup.document.v1'){window.ketchupStorageWrites.document++;value=v}else{if(k==='ketchup.selection.v1')window.ketchupStorageWrites.selection++;set.call(this,k,v)}}; if(navigator.gpu){const prototype=Object.getPrototypeOf(navigator.gpu), request=prototype.requestAdapter; prototype.requestAdapter=async function(...args){const adapter=await request.apply(this,args);if(adapter)window.ketchupViewportAdapter={backend:'webgpu',...Object.fromEntries(['vendor','architecture','device','description','isFallbackAdapter'].map(k=>[k,adapter.info[k]]))};return adapter;}} window.ketchupViewportBenchmarkMode=${JSON.stringify(mode)}; window.ketchupViewportBenchmarkCamera=${JSON.stringify(projectData.camera)}; window.ketchupViewportBenchmark=${seconds}; window.ketchupViewportBenchmarkLegacy=${args.includes("--legacy")};}`});
 let seed=await installSeed(model,(mode.startsWith('draw-')||mode==='measure-tape'||mode==='walk')?'observe':mode);
 await call('Page.navigate',{url:option('--url','http://127.0.0.1:8080/')});
 await call('Page.bringToFront');
 const evaluate=async expression=>{
  const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
  if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
 };
 await until(()=>evaluate(`window.ketchupViewportBenchmarkState`),'loaded workspace');
 const loadedNonce=await evaluate(`window.ketchupBenchmarkLoadNonce`);
 const loadedWasmUrl=await evaluate(`performance.getEntriesByType('resource').find(r=>r.name.endsWith('.wasm'))?.name`);
 let loadedWasmHash;
 if(loadedWasmUrl){
  const response=await fetch(loadedWasmUrl);
  if(!response.ok)throw Error('Loaded WebAssembly artifact is no longer available: '+loadedWasmUrl);
  const bytes=Buffer.from(await response.arrayBuffer());
  if(bytes.readUInt32LE(0)!==0x6d736100)throw Error('WebAssembly artifact response is invalid');
  loadedWasmHash=createHash('sha256').update(bytes).digest('hex');
 }
 let drawingResult;
 if(args.includes('--after-walk')){
  await until(()=>evaluate(`window.ketchupViewportBenchmarkState`),'walking workspace');
  await benchmarkWalk(call,evaluate,2,projectData);
  for(const type of ['keyDown','keyUp'])await call('Input.dispatchKeyEvent',{type,key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
  const restored=await until(async()=>{const state=await evaluate(`window.ketchupViewportBenchmarkState`);return state?.tool==='Select'?state:null},'leaving Walk');
  if(!Object.keys(projectData.camera).every(key=>JSON.stringify(restored.camera?.[key])===JSON.stringify(projectData.camera[key])))throw Error('Walk did not restore the original camera');
 }
 if(mode==='walk'){
  await until(()=>evaluate(`window.ketchupViewportBenchmarkState`),'walking workspace');
  drawingResult=await benchmarkWalk(call,evaluate,seconds,projectData);
 }else if(mode.startsWith('draw-')||mode==='measure-tape'){
  await until(()=>evaluate(`window.ketchupViewportBenchmarkState`),'drawing workspace');
  drawingResult=await benchmarkDrawing(call,evaluate,seconds,mode,projectData);
 }else await until(async()=>{if(diagnostics.includes('Runtime.exception') || diagnostics.includes('panicked at') || diagnostics.includes('GPUDevice:') || diagnostics.includes('Runtime.exceptionThrown')) throw Error(diagnostics); return await evaluate(`window.ketchupViewportBenchmarkResult`)},'viewport benchmark',(seconds+60)*1000);
 if(args.includes('--profile'))await writeFile(output+'.cpu.json',JSON.stringify(await call('Profiler.stop')));
 if(await evaluate(`window.ketchupBenchmarkLoadNonce`)!==loadedNonce)throw Error('The app reloaded during measurement; result discarded');
 const result=drawingResult??await evaluate(`window.ketchupViewportBenchmarkResult`);
 if(args.includes('--after-walk'))result.after_walk=true;
 if(result.walk_bridge_mutations>6)throw Error('Repeated Walk bridge updates during interaction: '+result.walk_bridge_mutations);
 result.storage_writes=await evaluate(`window.ketchupStorageWrites`);
 if(args.includes('--validate-snapping'))result.snapping_regression=await evaluate(`(${validateSnapping.toString()})()`);
 if(args.includes('--validate-shader'))result.visibility_regression=await evaluate(`(${validateVisibility.toString()})(${JSON.stringify(await readFile(new URL('../crates/ketchup-app/src/resident_scene.wgsl',import.meta.url),'utf8'))})`);
 result.project=project;
 result.browser_executable=executable;result.unsafe_webgpu=args.includes('--unsafe-webgpu');result.x11=args.includes('--x11');
 if(preferenceSource){result.preferences=preferenceSource;result.preferences_sha256=createHash('sha256').update(inspectorPreferences).digest('hex');}
 result.browser=await call('Browser.getVersion');
 result.project_sha256=createHash('sha256').update(model).digest('hex');
 result.wasm_url=loadedWasmUrl;result.wasm_sha256=loadedWasmHash;
 result.adapter=await evaluate(`window.ketchupViewportAdapter`);
 if(!result.adapter || result.adapter.isFallbackAdapter)throw Error('Benchmark requires a hardware GPU adapter: '+JSON.stringify(result.adapter));
 result.canvas=await evaluate(`document.getElementById('ketchup-canvas').toDataURL('image/png')`);
 const canvas=result.canvas; delete result.canvas; if(canvas?.startsWith('data:'))await writeFile(output+'.canvas.png',Buffer.from(canvas.split(',')[1],'base64'));
 await evaluate(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
 const {data}=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
 if(Buffer.from(data,'base64').length<20000)throw Error('Benchmark screenshot is blank; no FPS result accepted');
 await writeFile(output.replace(/\.json$/,'')+'.png',Buffer.from(data,'base64'));
 if(result.measurement_errors)throw Error('Inspector measurements failed: '+result.measurement_error);
 if(mode==='select' && !(result.selected_frames>0 && result.actions>10 && result.measurements_ready>0))throw Error('Selection benchmark did not exercise selection and exact inspector measurements');
 if(mode==='groups' && !(result.group_frames>0 && result.scopes.length>=2 && result.actions>10))throw Error('Group benchmark did not enter nested editing scopes');
 if(args.includes('--validate-state')){
  const expected={selected:result.final_selection,context:result.final_context};
  const reload=async source=>{
   await call('Page.removeScriptToEvaluateOnNewDocument',{identifier:seed.identifier});
   seed=await installSeed(source,'observe');
   await call('Page.reload',{ignoreCache:true});
   return await until(()=>evaluate(`window.ketchupViewportBenchmarkState`),'restored editor state');
  };
  const restored=await reload(model);
  if(restored.context!==expected.context || JSON.stringify(restored.selected)!==JSON.stringify(expected.selected))throw Error('Selection/context did not survive reload: '+JSON.stringify({expected,restored}));
  const other=JSON.parse(model);other.name+=' (isolated selection check)';other.selection=[];other.selection_context=null;
  const unrelated=await reload(JSON.stringify(other));
  if(unrelated.selected.length || unrelated.context!==null)throw Error('Selection leaked into another project: '+JSON.stringify(unrelated));
  result.state_regression={passed:true,restored,unrelated};
 }
 await writeFile(output,JSON.stringify(result,null,2)+'\n');
 await writeFile(output+'.log',diagnostics);
 if(!(result.draws>0 && (result.mode!=='orbit' || result.yaw_range[1]-result.yaw_range[0]>0.2)))throw Error('The benchmark did not render a moving camera');
 if(createHash('sha256').update(await readFile(project)).digest('hex')!==result.project_sha256)throw Error('Source project changed during the benchmark');
 console.log(JSON.stringify(result,null,2));
 const minimum=Number(option('--min-fps','0'));
 if(!Number.isFinite(minimum)||minimum<0)throw Error('--min-fps must be a nonnegative number');
 if(result.fps<minimum)throw Error(`Measured ${result.fps.toFixed(2)} FPS, below ${minimum} FPS`);
} catch(error) {
 if(socket?.readyState===1) {
  try {
   const id=900000; const shot=await new Promise(resolve=>{
    const timer=setTimeout(()=>{socket.removeEventListener('message',listener);resolve(null)},5000);
    const listener=event=>{const m=JSON.parse(event.data);if(m.id===id){clearTimeout(timer);socket.removeEventListener('message',listener);resolve(m.result)}};
    socket.addEventListener('message',listener);socket.send(JSON.stringify({id,method:'Page.captureScreenshot',params:{format:'png'}}));
   });
   if(shot?.data)await writeFile(output+'.failure.png',Buffer.from(shot.data,'base64'));
  }catch{}
 }
 await writeFile(output+'.log',diagnostics);
 throw error;
} finally {
 socket?.close();browser.kill();
 await new Promise(resolve=>{if(browser.exitCode!==null)resolve();else{browser.once('exit',resolve);setTimeout(()=>{browser.kill('SIGKILL');resolve()},3000).unref()}});
 await rm(profile,{recursive:true,force:true,maxRetries:5,retryDelay:200});
}

// Exercise the actual visibility helper on a hardware GPU: an unrelated front
// face hides an edge; an incident face, background, and rear face do not.
async function validateVisibility(shader) {
 const adapter=await navigator.gpu.requestAdapter();
 if(!adapter||adapter.info.isFallbackAdapter)throw Error('Hardware adapter required for shader regression');
 const device=await adapter.requestDevice();
 try {
  device.pushErrorScope('validation');
  const module=device.createShaderModule({code:shader+`
@vertex
fn test_visibility_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Output {
 let positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
 let faces=array<u32,4>(11u,7u,0u,11u);
 var value: Output;
 value.position=vec4<f32>(positions[vertex],select(0.5,0.7,instance>=4u),1.0);
 value.faces=vec4<u32>(select(faces[instance%4u],99u,instance>=4u),0u,0u,0u);
 return value;
}
@group(2) @binding(0) var<storage,read_write> regression: array<u32>;
@compute @workgroup_size(1)
fn test_center_visibility() {
 for(var i=0u;i<4u;i++) {
  var value: Output;
  value.position.w=1.0;
  value.center=vec3<f32>((f32(i)+0.5)/2.0-1.0,0.0,select(0.6,0.4,i==3u));
  value.faces=vec4<u32>(7u,8u,9u,10u);
  regression[i]=select(0u,1u,hidden_center(value));
 }
}`});
  const compilation=await module.getCompilationInfo();
  const errors=compilation.messages.filter(message=>message.type==='error');
  if(errors.length)throw Error(JSON.stringify(errors));
  const pipeline=device.createComputePipeline({layout:'auto',compute:{module,entryPoint:'test_center_visibility'}});
  const visibility=device.createRenderPipeline({layout:'auto',vertex:{module,entryPoint:'test_visibility_vertex'},fragment:{module,entryPoint:'fragment_visibility',targets:[{format:'rg32uint'}]},depthStencil:{format:'depth32float',depthWriteEnabled:true,depthCompare:'less-equal'}});
  const ids=device.createTexture({size:[4,1],format:'rg32uint',usage:GPUTextureUsage.RENDER_ATTACHMENT|GPUTextureUsage.TEXTURE_BINDING});
  const depth=device.createTexture({size:[4,1],format:'depth32float',usage:GPUTextureUsage.RENDER_ATTACHMENT});
  const result=device.createBuffer({size:16,usage:GPUBufferUsage.STORAGE|GPUBufferUsage.COPY_SRC});
  const readback=device.createBuffer({size:16,usage:GPUBufferUsage.MAP_READ|GPUBufferUsage.COPY_DST});
  const encoder=device.createCommandEncoder();
  const render=encoder.beginRenderPass({colorAttachments:[{view:ids.createView(),clearValue:[0,0,0,0],loadOp:'clear',storeOp:'store'}],depthStencilAttachment:{view:depth.createView(),depthClearValue:1,depthLoadOp:'clear',depthStoreOp:'discard'}});
  render.setPipeline(visibility);
  // Write actual face/depth pairs, then try to overwrite them with rear faces.
  for(let instance=0;instance<8;instance++){render.setViewport(instance%4,0,1,1,0,1);render.draw(3,1,0,instance)}
  render.end();
  const pass=encoder.beginComputePass();pass.setPipeline(pipeline);
  pass.setBindGroup(0,device.createBindGroup({layout:pipeline.getBindGroupLayout(0),entries:[]}));
  pass.setBindGroup(1,device.createBindGroup({layout:pipeline.getBindGroupLayout(1),entries:[{binding:0,resource:ids.createView()}]}));
  pass.setBindGroup(2,device.createBindGroup({layout:pipeline.getBindGroupLayout(2),entries:[{binding:0,resource:{buffer:result}}]}));
  pass.dispatchWorkgroups(1);pass.end();
  encoder.copyBufferToBuffer(result,0,readback,0,16);
  device.queue.submit([encoder.finish()]);
  const validation=await device.popErrorScope();if(validation)throw Error(validation.message);
  await readback.mapAsync(GPUMapMode.READ);
  const actual=Array.from(new Uint32Array(readback.getMappedRange()));readback.unmap();
  if(JSON.stringify(actual)!=='[1,0,0,0]')throw Error('Center visibility regression: '+JSON.stringify(actual));
  return {passed:true,results:actual};
 } finally {device.destroy();}
}

// Exercise real egui input handlers, inference, preview, geometry creation and
// autosave. Count frames submitted by the app, rather than compositor RAFs.
async function benchmarkDrawing(call,evaluate,seconds,mode,source) {
 const move=async (x,y)=>call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
 const click=async (x,y)=>{await move(x,y);await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1})};
 const key=async (key,code,keyCode)=>{await call('Input.dispatchKeyEvent',{type:'keyDown',key,code,windowsVirtualKeyCode:keyCode});await call('Input.dispatchKeyEvent',{type:'keyUp',key,code,windowsVirtualKeyCode:keyCode})};
 await new Promise(resolve=>setTimeout(resolve,1000));
 const letter=mode==='measure-tape'?'t':mode==='draw-line'?'l':'r';
 await key(letter,'Key'+letter.toUpperCase(),letter.toUpperCase().charCodeAt(0));
 await click(600,500);
 await move(690,560);
 if(mode==='measure-tape'){
  // Keep a real saved guide in the model throughout the measured preview.
  await click(690,560);await key('Escape','Escape',27);
  await click(600,500);await move(690,560);
 }
 await evaluate(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
 await evaluate(`window.ketchupDrawingProbe.samples=[];window.ketchupDrawingProbe.walkMutations=0;window.ketchupDrawingProbe.record=true`);
 const start=Date.now();let moves=0;
 while(Date.now()-start<seconds*1000){
  const t=(Date.now()-start)/1000;
  await move(720+70*Math.sin(t*1.3),530+55*Math.cos(t*1.1));moves++;
  await new Promise(resolve=>setTimeout(resolve,Math.max(0,start+moves*1000/60-Date.now())));
 }
 const previewState=await evaluate(`window.ketchupViewportBenchmarkState`);
 if(mode==='measure-tape' && !(previewState.tool==='TapeMeasure' && previewState.measurement_points?.length===2 && previewState.measurement_value))throw Error('Tape measure preview is missing: '+JSON.stringify(previewState));
 const commitStart=Date.now();
 await click(735,570);
 await key('Escape','Escape',27);
 await evaluate(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
 const commitMs=Date.now()-commitStart;
 const data=await evaluate(`(()=>{window.ketchupDrawingProbe.record=false;const doc=JSON.parse(window.ketchupBenchmarkDocument());return {samples:window.ketchupDrawingProbe.samples,walkMutations:window.ketchupDrawingProbe.walkMutations,faces:Object.keys(doc.model.faces).length,edges:Object.keys(doc.model.edges).length,guides:Object.keys(doc.model.guides??{}).length,measurement:window.ketchupViewportBenchmarkState,selection:doc.selection,context:doc.selection_context}})()`);
 const elapsed=Date.now()-start, intervals=data.samples.slice(1).map((time,i)=>time-data.samples[i]).sort((a,b)=>a-b);
 const createdEdges=data.edges-Object.keys(source.model.edges).length;
 const createdGuides=data.guides-Object.keys(source.model.guides??{}).length;
 if(mode==='measure-tape'?createdGuides<=0:createdEdges<=0)throw Error('Interaction did not commit new geometry: '+JSON.stringify(data));
 return {mode,frames:data.samples.length,draws:data.samples.length,fps:data.samples.length*1000/elapsed,mean_ms:elapsed/Math.max(1,data.samples.length),p95_ms:intervals[Math.floor(intervals.length*.95)],max_ms:intervals.at(-1),commit_ms:commitMs,walk_bridge_mutations:data.walkMutations,pointer_moves:moves,created_edges:createdEdges,created_guides:createdGuides,measurement_preview:previewState,measurement:data.measurement,faces:data.faces,edges:data.edges,edges_visible:true,final_selection:data.selection??[],final_context:data.context??null,size:[1440,900]};
}

async function validateSnapping() {
 const resources=performance.getEntriesByType('resource');
 const glue=resources.findLast(r=>/ketchup-app-[^/]+\.js(?:\?|$)/.test(r.name))?.name;
 const wasm=resources.findLast(r=>r.name.endsWith('_bg.wasm'))?.name;
 if(!glue||!wasm)throw Error('Cannot locate app modules for snapping regression');
 const code=`const ready=import(${JSON.stringify(glue)}).then(async m=>{await m.default({module_or_path:${JSON.stringify(wasm)}});return m});self.onmessage=async e=>{try{const m=await ready;self.postMessage({result:JSON.parse(m.ketchup_validate_snapping(e.data))})}catch(error){self.postMessage({error:String(error)})}}`;
 const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'}));
 const worker=new Worker(url,{type:'module'});URL.revokeObjectURL(url);
 try {
  return await new Promise((resolve,reject)=>{
   const timeout=setTimeout(()=>reject(Error('Snapping regression timed out')),55000);
   worker.onerror=e=>{clearTimeout(timeout);reject(Error(e.message))};
   worker.onmessage=e=>{clearTimeout(timeout);if(e.data.error)reject(Error(e.data.error));else resolve(e.data.result)};
   worker.postMessage(window.ketchupBenchmarkDocument());
  });
 } finally {worker.terminate();}
}

async function benchmarkWalk(call,evaluate,seconds,source) {
 const move=(x,y)=>call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
 const click=async(x,y)=>{await move(x,y);await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1})};
 const key=(type,letter)=>call('Input.dispatchKeyEvent',{type,key:letter,code:'Key'+letter.toUpperCase(),windowsVirtualKeyCode:letter.toUpperCase().charCodeAt(0)});
 const painted=()=>evaluate(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
 await new Promise(resolve=>setTimeout(resolve,1000));
 await click(635,87);await painted();
 if((await evaluate(`window.ketchupViewportBenchmarkState`)).tool!=='Walk')throw Error('Walk tool did not activate');
 let placed=false;
 for(const [x,y] of [[500,700],[400,600],[700,450],[750,350],[500,550]]) {
  await click(x,y);await painted();
  placed=await evaluate(`document.getElementById('ketchup-canvas').dataset.walkPlaced==='true'`);
  if(placed)break;
 }
 if(!placed)throw Error('Could not place walking person on a real scene floor');
 const initial=await evaluate(`window.ketchupViewportBenchmarkState`);
 await evaluate(`window.ketchupDrawingProbe.samples=[];window.ketchupDrawingProbe.walkMutations=0;window.ketchupDrawingProbe.record=true`);
 const start=Date.now();let moves=0,held='w',phase=0;
 await key('keyDown',held);
 while(Date.now()-start<seconds*1000){
  const t=(Date.now()-start)/1000,next=Math.floor(t/1.5)%4;
  if(next!==phase){await key('keyUp',held);held=['w','d','s','a'][next];await key('keyDown',held);phase=next;}
  await move(650+40*Math.sin(t*2),470+15*Math.sin(t*1.3));moves++;
  await new Promise(resolve=>setTimeout(resolve,Math.max(0,start+moves*1000/60-Date.now())));
 }
 await key('keyUp',held);await painted();
 const data=await evaluate(`(()=>{window.ketchupDrawingProbe.record=false;return {samples:window.ketchupDrawingProbe.samples,walkMutations:window.ketchupDrawingProbe.walkMutations,state:window.ketchupViewportBenchmarkState,placed:document.getElementById('ketchup-canvas').dataset.walkPlaced,capture:document.getElementById('ketchup-canvas').dataset.walkPointerCapture,pointer_error:window.ketchupPointerLockError??null}})()`);
 const elapsed=Date.now()-start,intervals=data.samples.slice(1).map((time,i)=>time-data.samples[i]).sort((a,b)=>a-b);
 if(data.placed!=='true'||!data.state.walk_feet)throw Error('Walk stopped during the benchmark');
 if(args.includes('--require-pointer-lock')&&data.capture!=='locked')throw Error('Pointer capture unavailable: '+await evaluate(`JSON.stringify({state:document.getElementById('ketchup-canvas').dataset.walkPointerCapture,error:window.ketchupPointerLockError,focused:document.hasFocus()})`));
 if(JSON.stringify(initial.camera)===JSON.stringify(data.state.camera))throw Error('Walk camera did not move');
 return {mode:'walk',frames:data.samples.length,draws:data.samples.length,fps:data.samples.length*1000/elapsed,mean_ms:elapsed/data.samples.length,p95_ms:intervals[Math.floor(intervals.length*.95)],max_ms:intervals.at(-1),walk_bridge_mutations:data.walkMutations,pointer_moves:moves,walk_initial:initial,walk_final:data.state,pointer_capture:data.capture,pointer_capture_error:data.pointer_error,faces:Object.keys(source.model.faces).length,edges:Object.keys(source.model.edges).length,edges_visible:source.display.edges,final_selection:data.state.selected,final_context:data.state.context,size:[1440,900]};
}
