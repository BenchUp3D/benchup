#!/usr/bin/env node
// Benchmark an existing running app; never starts a build. Isolated browser data.
import {readFile, writeFile, mkdtemp, rm, access, readdir, mkdir} from 'node:fs/promises';
import {spawn, spawnSync} from 'node:child_process';
import {join, dirname} from 'node:path';
import {tmpdir, homedir} from 'node:os';
import {createHash} from 'node:crypto';
import {gunzipSync} from 'node:zlib';
import {fileURLToPath} from 'node:url';
const args=process.argv.slice(2);
const option=(key, fallback)=>{const i=args.indexOf(key);return i<0?fallback:args[i+1]};
const project=option('--project');
const usage='Usage: node scripts/benchmark_viewport_web.mjs --project FILE [--mode orbit|sun|select|scale|screenshot|box-select|groups|draw-line|draw-rectangle|measure-tape|walk|erase-edge|erase-face|erase-object] [--scope root|group|leaf] [--repeat-actions] [--flatten] [--reverse-box] [--seconds 20] [--url http://127.0.0.1:8080] [--output /tmp/viewport.json] [--browser PATH] [--headed] [--unsafe-webgpu] [--x11] [--preferences FILE] [--legacy] [--profile] [--max-open-ms MS] [--min-fps FPS] [--min-selected-fps FPS] [--validate-shader] [--validate-shadows] [--validate-markers] [--require-shadows] [--validate-state] [--validate-snapping] [--after-walk] [--require-pointer-lock] [--erase-with-key]';
if(args.includes('--help')){console.log(usage);process.exit(0)}
if(!project)throw Error(usage);
const mode=option('--mode','orbit');
if(!['screenshot','orbit','sun','select','box-select','groups','draw-line','draw-rectangle','measure-tape','walk','scale','erase-edge','erase-face','erase-object'].includes(mode))throw Error('--mode must be orbit, sun, select, groups, draw-line, draw-rectangle, measure-tape, walk, or erase-edge');
const projectBytes=await readFile(project);
let payload=projectBytes;
if(payload.subarray(0,8).equals(Buffer.from([137,80,78,71,13,10,26,10]))){
  let found=false;
  for(let offset=8;offset+12<=projectBytes.length;){
    const length=projectBytes.readUInt32BE(offset);
    if(offset+12+length>projectBytes.length)throw Error('Truncated project chunk');
    if(projectBytes.toString('ascii',offset+4,offset+8)==='buPd'){
      payload=projectBytes.subarray(offset+8,offset+8+length);found=true;break;
    }
    offset+=12+length;
  }
  if(!found)throw Error('PNG has no BenchUp project data');
}
const model=(payload[0]===0x1f&&payload[1]===0x8b?gunzipSync(payload):payload).toString('utf8');
const projectData=JSON.parse(model);
const scope=option('--scope',mode.startsWith('erase-')?'leaf':'root');
if(!['root','group','leaf'].includes(scope))throw Error('--scope must be root, group, or leaf');
const eraseSetup=mode.startsWith('erase-')||scope!=='root'?prepareErase(projectData):null;
let boxDocument;
if(mode==='box-select'){
 boxDocument=structuredClone(eraseSetup?.document??projectData);
 boxDocument.camera.distance*=2.5;
 boxDocument.viewport_cameras={"0":boxDocument.camera};
 boxDocument.selection=[];
 if(scope==='root')boxDocument.selection_context=null;
 if(args.includes('--flatten')){
  const context=boxDocument.selection_context;
  const ids=['edges','faces','dimensions','texts','guides'].flatMap(kind=>Object.keys(boxDocument.model[kind]??{}).map(Number));
  if(context===null)boxDocument.model.objects={};
  else {const object=boxDocument.model.objects[context];object.entities=ids;boxDocument.model.objects={[context]:object};}
 }
}
const preferenceSource=option('--preferences');
// Raw benchup.inspector.v1 RON, copied out of storage rather than a live profile.
const inspectorPreferences=preferenceSource?await readFile(preferenceSource,'utf8'):null;
const seconds=Number(option('--seconds','20'));
if(!Number.isFinite(seconds)||seconds<4||seconds>120)throw Error('--seconds must be between 4 and 120');
const output=option('--output','/tmp/benchup-web-viewport.json');
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
const profile=await mkdtemp(join(tmpdir(),'benchup-viewport-'));
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
    ui.send(JSON.stringify({id:1,method:'Runtime.evaluate',params:{expression:"new Promise(resolve=>chrome.tabs.create({url:'about:blank',active:true},tab=>chrome.windows.update(tab.windowId,{focused:true},()=>resolve(tab))))",awaitPromise:true,returnByValue:true}}));
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
 // CDP focus emulation does not raise a real XWayland window. The desktop
 // capture helper locates this isolated browser by PID; never touch user tabs.
 if(args.includes('--headed')&&args.includes('--x11')){
  const focus=spawnSync('python3',['-B','-c',`
import sys,ctypes as C,importlib.util,time
spec=importlib.util.spec_from_file_location('capture',sys.argv[2])
module=importlib.util.module_from_spec(spec);spec.loader.exec_module(module)
d=module.Desktop();window=d.window_for(int(sys.argv[1]))
if window is None: raise RuntimeError('Cannot find the benchmark browser window')
d.x.XRaiseWindow.argtypes=[C.c_void_p,C.c_ulong];d.x.XRaiseWindow(d.display,window)
class Message(C.Structure):
 _fields_=[('type',C.c_int),('serial',C.c_ulong),('send_event',C.c_int),('display',C.c_void_p),('window',C.c_ulong),('message_type',C.c_ulong),('format',C.c_int),('data',C.c_long*5)]
class Event(C.Union):
 _fields_=[('client',Message),('padding',C.c_long*24)]
m=Message(type=33,display=d.display,window=window,message_type=d.x.XInternAtom(d.display,b'_NET_ACTIVE_WINDOW',0),format=32)
m.data[0]=2
message=Event(client=m)
d.x.XSendEvent.argtypes=[C.c_void_p,C.c_ulong,C.c_int,C.c_long,C.POINTER(Event)]
d.x.XSendEvent.restype=C.c_int
d.x.XSendEvent(d.display,d.root,0,(1<<19)|(1<<20),C.byref(message))
d.x.XSetInputFocus(d.display,window,2,0);d.x.XFlush(d.display);time.sleep(.1)
if d.property(d.root,'_NET_ACTIVE_WINDOW') != [window]: raise RuntimeError('The benchmark browser is not the active X11 window')
`,String(browser.pid),fileURLToPath(new URL('./capture_readme_desktop.py',import.meta.url))],{encoding:'utf8'});
  if(focus.status!==0)throw Error('Cannot focus benchmark window: '+focus.stderr);
 }
 const installSeed=async (source,mode)=>await call('Page.addScriptToEvaluateOnNewDocument',{source:
 `{let value=${JSON.stringify(source)},edits=null;
 window.benchupBenchmarkLoadNonce=Math.random().toString(36);
 const NativeWebSocket=window.WebSocket;
 window.WebSocket=new Proxy(NativeWebSocket,{construct(target,args,newTarget){if(String(args[0]).includes('/.well-known/trunk/ws'))return Object.assign(new EventTarget(),{readyState:3,close(){},send(){}});return Reflect.construct(target,args,newTarget)}});
 window.benchupBenchmarkBaseDocument=()=>value;
 window.benchupBenchmarkDocument=()=>(${reconstructAutosave.toString()})(value,edits);
 if(globalThis.Element?.prototype.requestPointerLock){const lock=Element.prototype.requestPointerLock;Element.prototype.requestPointerLock=function(...args){try{const result=lock.apply(this,args);return result?.catch?result.catch(error=>{window.benchupPointerLockError=String(error);throw error}):result}catch(error){window.benchupPointerLockError=String(error);throw error}}}
 window.benchupStorageWrites={document:0,selection:0,edits:0};
 const recordRecovery=(k,v)=>{if(k==='benchup.document.v1'){window.benchupStorageWrites.document++;value=v;edits=null}else if(k==='benchup.edits.v1'){window.benchupStorageWrites.edits++;edits=v}else if(k==='benchup.selection.v1')window.benchupStorageWrites.selection++};
 // Recovery moved to IndexedDB. Seed its document read while retaining real
 // transactions and saved overlays, so reload checks exercise durable saves.
 if(globalThis.IDBObjectStore){
  const get=IDBObjectStore.prototype.get,put=IDBObjectStore.prototype.put;
  const recovery=store=>store.name==='recovery'&&store.transaction.db.name==='benchup-workspace';
  IDBObjectStore.prototype.get=function(k){const request=get.call(this,k);if(recovery(this)&&k==='benchup.document.v1')request.addEventListener('success',()=>Object.defineProperty(request,'result',{value,configurable:true}));return request};
  IDBObjectStore.prototype.put=function(v,k){const request=put.call(this,v,k);if(recovery(this))recordRecovery(k,v);return request};
 }
 {let frame=0,last=-1;const probe=window.benchupDrawingProbe={samples:[],record:false,walkMutations:0};
 const submitted=()=>{if(probe.record&&last!==frame){probe.samples.push(performance.now());last=frame}};
 const raf=()=>{frame++;requestAnimationFrame(raf)};requestAnimationFrame(raf);
 document.addEventListener('DOMContentLoaded',()=>{const canvas=document.getElementById('benchup-canvas');if(canvas)new MutationObserver(changes=>{if(probe.record)probe.walkMutations+=changes.length}).observe(canvas,{attributes:true,attributeFilter:['data-walk-active','data-walk-input-enabled','data-walk-placed']})});
 if(globalThis.GPUQueue){const submit=GPUQueue.prototype.submit;GPUQueue.prototype.submit=function(...args){submitted();return submit.apply(this,args)}}
 if(globalThis.WebGL2RenderingContext){for(const method of ['drawArrays','drawElements','drawArraysInstanced','drawElementsInstanced']){const draw=WebGL2RenderingContext.prototype[method];if(draw)WebGL2RenderingContext.prototype[method]=function(...args){if(this.canvas.id==='benchup-canvas')submitted();return draw.apply(this,args)}}}
 const getContext=HTMLCanvasElement.prototype.getContext;
 HTMLCanvasElement.prototype.getContext=function(kind,...args){const context=getContext.call(this,kind,...args);if(context&&kind==='webgl2'&&this.id==='benchup-canvas'){const info=context.getExtension('WEBGL_debug_renderer_info');const renderer=context.getParameter(info?info.UNMASKED_RENDERER_WEBGL:context.RENDERER);window.benchupViewportAdapter={backend:'webgl2',vendor:context.getParameter(info?info.UNMASKED_VENDOR_WEBGL:context.VENDOR),renderer,isFallbackAdapter:/swiftshader|llvmpipe|software/i.test(renderer)}}return context};}
 const get=Storage.prototype.getItem, set=Storage.prototype.setItem; Storage.prototype.getItem=function(k){return k==='benchup.document.v1'?value:k==='benchup.inspector.v1'&&${JSON.stringify(inspectorPreferences)}!==null?${JSON.stringify(inspectorPreferences)}:get.call(this,k)}; Storage.prototype.setItem=function(k,v){recordRecovery(k,v);if(k!=='benchup.document.v1')set.call(this,k,v)}; if(navigator.gpu){const prototype=Object.getPrototypeOf(navigator.gpu), request=prototype.requestAdapter; prototype.requestAdapter=async function(...args){const adapter=await request.apply(this,args);if(adapter)window.benchupViewportAdapter={backend:'webgpu',...Object.fromEntries(['vendor','architecture','device','description','isFallbackAdapter'].map(k=>[k,adapter.info[k]]))};return adapter;}} window.benchupViewportBenchmarkMode=${JSON.stringify(mode)}; window.benchupViewportBenchmarkCamera=${JSON.stringify(projectData.camera)}; window.benchupViewportBenchmark=${seconds}; window.benchupViewportBenchmarkLegacy=${args.includes("--legacy")};}`});
 let seed=await installSeed(boxDocument?JSON.stringify(boxDocument):eraseSetup?JSON.stringify(eraseSetup.document):model,(mode==='screenshot'||mode==='box-select'||mode.startsWith('draw-')||mode==='measure-tape'||mode==='walk'||mode.startsWith('erase-'))?'observe':mode);
 await call('Page.navigate',{url:option('--url','http://127.0.0.1:8080/')});
 await call('Page.bringToFront');
 const evaluate=async expression=>{
  const r=await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
  if(r.exceptionDetails)throw Error(JSON.stringify(r.exceptionDetails));
  return r.result.value;
 };
 await until(async()=>{
  if(diagnostics.includes('panicked at') || diagnostics.includes('Runtime.exceptionThrown'))throw Error(diagnostics);
  return evaluate(`window.benchupViewportBenchmarkState`);
 },'loaded workspace');
 const loadedNonce=await evaluate(`window.benchupBenchmarkLoadNonce`);
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
  await until(()=>evaluate(`window.benchupViewportBenchmarkState`),'walking workspace');
  await benchmarkWalk(call,evaluate,2,projectData);
  for(const type of ['keyDown','keyUp'])await call('Input.dispatchKeyEvent',{type,key:'Escape',code:'Escape',windowsVirtualKeyCode:27});
  const restored=await until(async()=>{const state=await evaluate(`window.benchupViewportBenchmarkState`);return state?.tool==='Select'?state:null},'leaving Walk');
  if(!Object.keys(projectData.camera).every(key=>JSON.stringify(restored.camera?.[key])===JSON.stringify(projectData.camera[key])))throw Error('Walk did not restore the original camera');
 }
 if(mode==='screenshot'){
  drawingResult=await benchmarkScreenshot(call,evaluate,output);
 }else if(mode==='box-select'){
  drawingResult=await benchmarkBoxSelection(call,evaluate,seconds);
 }else if(mode.startsWith('erase-')){
  drawingResult=await benchmarkErase(call,evaluate,seconds,eraseSetup);
 }else if(mode==='walk'){
  await until(()=>evaluate(`window.benchupViewportBenchmarkState`),'walking workspace');
  drawingResult=await benchmarkWalk(call,evaluate,seconds,projectData);
 }else if(mode.startsWith('draw-')||mode==='measure-tape'){
  await until(()=>evaluate(`window.benchupViewportBenchmarkState`),'drawing workspace');
  drawingResult=await benchmarkDrawing(call,evaluate,seconds,mode,projectData);
 }else await until(async()=>{if(diagnostics.includes('Runtime.exception') || diagnostics.includes('panicked at') || diagnostics.includes('GPUDevice:') || diagnostics.includes('Runtime.exceptionThrown')) throw Error(diagnostics); return await evaluate(`window.benchupViewportBenchmarkResult`)},'viewport benchmark',(seconds+60)*1000);
 if(args.includes('--profile'))await writeFile(output+'.cpu.json',JSON.stringify(await call('Profiler.stop')));
 if(await evaluate(`window.benchupBenchmarkLoadNonce`)!==loadedNonce)throw Error('The app reloaded during measurement; result discarded');
 const result=drawingResult??await evaluate(`window.benchupViewportBenchmarkResult`);
 await writeFile(output+'.partial.json',JSON.stringify(result,null,2)+'\n');
 if(args.includes('--after-walk'))result.after_walk=true;
 if(result.walk_bridge_mutations>6)throw Error('Repeated Walk bridge updates during interaction: '+result.walk_bridge_mutations);
 result.storage_writes=await evaluate(`window.benchupStorageWrites`);
 const shadowState=await evaluate('window.benchupViewportBenchmarkState');
 result.shadows=shadowState.shadows;result.shadow_builds=shadowState.shadow_builds;
 if(args.includes('--require-shadows')&&(!result.shadows||!(result.shadow_builds>0)))throw Error('Shadows were not enabled during the benchmark');
 if(args.includes('--require-shadows')&&['orbit','select','groups'].includes(mode)&&result.shadow_builds!==1)throw Error('Unchanged sunlight was rebuilt during camera/selection/scope changes: '+result.shadow_builds);
 if(args.includes('--validate-markers'))result.marker_regression=await evaluate(`(${validateMarkers.toString()})(${JSON.stringify(await readFile(new URL('../crates/benchup-app/src/resident_scene.wgsl',import.meta.url),'utf8'))})`);
 if(args.includes('--validate-shadows'))result.shadow_regression=await evaluate(`(${validateSunlight.toString()})(${JSON.stringify(await readFile(new URL('../crates/benchup-app/src/resident_scene.wgsl',import.meta.url),'utf8'))})`);
 if(mode==='erase-edge'&&scope==='leaf')result.cache_regression=await evaluate(`(${validateEraseCaches.toString()})(${eraseSetup.context},${eraseSetup.edge})`);
 if(args.includes('--validate-snapping'))result.snapping_regression=await evaluate(`(${validateSnapping.toString()})()`);
 if(args.includes('--validate-shader'))result.visibility_regression=await evaluate(`(${validateVisibility.toString()})(${JSON.stringify(await readFile(new URL('../crates/benchup-app/src/resident_scene.wgsl',import.meta.url),'utf8'))})`);
 result.project=project;
 result.browser_executable=executable;result.unsafe_webgpu=args.includes('--unsafe-webgpu');result.x11=args.includes('--x11');
 result.scope=scope;
 if(preferenceSource){result.preferences=preferenceSource;result.preferences_sha256=createHash('sha256').update(inspectorPreferences).digest('hex');}
 result.browser=await call('Browser.getVersion');
 result.project_sha256=createHash('sha256').update(projectBytes).digest('hex');
 result.wasm_url=loadedWasmUrl;result.wasm_sha256=loadedWasmHash;
 result.adapter=await evaluate(`window.benchupViewportAdapter`);
 if(!result.adapter || result.adapter.isFallbackAdapter)throw Error('Benchmark requires a hardware GPU adapter: '+JSON.stringify(result.adapter));
 result.canvas=await evaluate(`document.getElementById('benchup-canvas').toDataURL('image/png')`);
 const canvas=result.canvas; delete result.canvas; if(canvas?.startsWith('data:'))await writeFile(output+'.canvas.png',Buffer.from(canvas.split(',')[1],'base64'));
 const {data}=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
 if(Buffer.from(data,'base64').length<20000)throw Error('Benchmark screenshot is blank; no FPS result accepted');
 await writeFile(output.replace(/\.json$/,'')+'.png',Buffer.from(data,'base64'));
 if(result.measurement_errors)throw Error('Inspector measurements failed: '+result.measurement_error);
 if(mode==='select' && !(result.selected_frames>0 && result.actions>10 && result.measurements_ready>0))throw Error('Selection benchmark did not exercise selection and exact inspector measurements');
 if(mode==='groups' && !(result.group_frames>0 && result.scopes.length>=2 && result.actions>10))throw Error('Group benchmark did not enter nested editing scopes');
 if(args.includes('--validate-state')){
  const expected={selected:result.final_selection,context:result.final_context};
  const reload=async source=>{
   const previous=await evaluate('window.benchupBenchmarkLoadNonce');
   await call('Page.removeScriptToEvaluateOnNewDocument',{identifier:seed.identifier});
   seed=await installSeed(source,'observe');
   await call('Page.reload',{ignoreCache:true});
   return await until(()=>evaluate(`window.benchupBenchmarkLoadNonce!==${JSON.stringify(previous)}&&window.benchupViewportBenchmarkState`),'restored editor state');
  };
  const restored=await reload(await evaluate('window.benchupBenchmarkBaseDocument()'));
  if(restored.context!==expected.context || (restored.selected_count??restored.selected.length)!==expected.selected.length || JSON.stringify(restored.selected)!==JSON.stringify(expected.selected.slice(0,512)))throw Error('Selection/context did not survive reload: '+JSON.stringify({expected,restored}));
  if(restored.edges!==result.edges || restored.faces!==result.faces)throw Error('Edited geometry did not survive reload: '+JSON.stringify(restored));
  await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
  const reloadShot=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
  await writeFile(output+'.reload.png',Buffer.from(reloadShot.data,'base64'));
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
 const selectedMinimum=Number(option('--min-selected-fps','0'));
 if(!Number.isFinite(selectedMinimum)||selectedMinimum<0)throw Error('--min-selected-fps must be a nonnegative number');
 if(selectedMinimum>0){
  if(!(result.selected?.frames>0))throw Error('The app did not report selected frame timings');
  if(result.selected.fps<selectedMinimum)throw Error(`Selected objects measured ${result.selected.fps.toFixed(2)} FPS, below ${selectedMinimum} FPS`);
 }
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

// Open the actual screenshot modal and wait for completed GPU readback rather
// than timing only the menu click. Every run uses an isolated project/profile.
async function benchmarkScreenshot(call,evaluate,output) {
 const wait=async(fn,label)=>{const deadline=Date.now()+30000;while(Date.now()<deadline){const value=await fn();if(value)return value;await pause(10)}throw Error('Timed out waiting for '+label)};
 const state=()=>evaluate('window.benchupViewportBenchmarkState');
 const click=async(x,y)=>{await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});for(const type of ['mousePressed','mouseReleased'])await call('Input.dispatchMouseEvent',{type,x,y,button:'left',clickCount:1});};
 const painted=()=>evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
 const ready=()=>wait(async()=>{const current=await state();if(current.screenshot?.error)throw Error(current.screenshot.error);return current.screenshot?.ready?current:null},'screenshot preview');
 const shot=async(suffix)=>{await painted();const {data}=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});await writeFile(output+suffix+'.png',Buffer.from(data,'base64'));};
 await painted();
 const before=await state();
 const downloadDirectory=join(profile,'downloads');await mkdir(downloadDirectory);
 await call('Browser.setDownloadBehavior',{behavior:'allow',downloadPath:downloadDirectory});
 await evaluate('window.benchupDrawingProbe.record=true;window.benchupDrawingProbe.samples=[]');
 const opens=[];
 for(let i=0;i<3;i++){
  await click(357,32);await painted();
  const start=Date.now();await click(411,220);const click_ms=Date.now()-start;
  const current=await ready();await painted();opens.push({click_ms,preview_ms:Date.now()-start,pages:current.screenshot.pages});
  if(i===0)await shot('.preview');
  if(i===1)await shot('.selected');
  if(i<2){for(const type of ['keyDown','keyUp'])await call('Input.dispatchKeyEvent',{type,key:'Escape',code:'Escape',windowsVirtualKeyCode:27});await wait(async()=>!(await state()).screenshot.open,'closing screenshot modal');}
  if(i===0){await click(600,500);await painted();if(!((await state()).selected_count>0))throw Error('Selection regression did not select the model');}
 }
 const expectedSelection=(await state()).selected;
 const positions=[[197,258],[276,258],[197,294],[276,294],[197,330],[276,330],[197,365],[276,365]];
 const changes=[];
 const selected=new Set((await state()).screenshot.views);
 for(let view=0;view<8;view++)if(!selected.has(view)){
  const start=Date.now();await click(...positions[view]);await painted();await ready();changes.push({view,preview_ms:Date.now()-start});
 }
 await shot('.all-views');
 const start=Date.now();await click(197,460);await painted();const individual=await ready();
 if(individual.screenshot.pages!==8||individual.screenshot.combined)throw Error('Individual preview did not render all eight views');
 const layout_ms=Date.now()-start;
 await shot('.individual');
 const exportStart=Date.now();await click(1146,794);
 const exported=await wait(async()=>{const files=(await readdir(downloadDirectory)).filter(name=>!name.endsWith('.crdownload'));return files.length===1?files[0]:null},'screenshot export');
 const export_ms=Date.now()-exportStart;
 const bytes=await readFile(join(downloadDirectory,exported));
 if(bytes.readUInt32LE(0)!==0x04034b50)throw Error('Eight-view export is not a ZIP archive');
 const export_file=output+'.export.zip';await writeFile(export_file,bytes);
 await wait(async()=>!(await state()).screenshot.open,'completed export');await painted();
 const after=await state();
 if(JSON.stringify(after.selected)!==JSON.stringify(expectedSelection))throw Error('Screenshot capture changed selection');
 if(after.history_position!==before.history_position||after.faces!==before.faces||after.edges!==before.edges||JSON.stringify(after.camera)!==JSON.stringify(before.camera))throw Error('Screenshot capture changed the source document');
 const samples=await evaluate('window.benchupDrawingProbe.record=false;window.benchupDrawingProbe.samples');
 const limit=Number(option('--max-open-ms','500'));
 if(!Number.isFinite(limit)||limit<=0)throw Error('--max-open-ms must be positive');
 if(opens.some(run=>run.preview_ms>limit))throw Error('Screenshot opening exceeded '+limit+' ms: '+JSON.stringify(opens));
 return {mode:'screenshot',draws:samples.length,opens,changes,layout_ms,export_ms,export_file,export_bytes:bytes.length};
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
 await call('Emulation.setFocusEmulationEnabled',{enabled:true});
 await click(...(scope==='leaf'?eraseSetup.pixel:[600,500]));
 await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
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
 await evaluate(`window.benchupDrawingProbe.samples=[];window.benchupDrawingProbe.walkMutations=0;window.benchupDrawingProbe.record=true`);
 const start=Date.now();let moves=0,nextCommit=1.5;const repeated=[];
 while(Date.now()-start<seconds*1000){
  const t=(Date.now()-start)/1000;
  if(args.includes('--repeat-actions')&&mode==='draw-rectangle'&&t>=nextCommit&&t<seconds-.7){
   const begin=Date.now();await click(720+70*Math.sin(t*1.3),530+55*Math.cos(t*1.1));
   await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
   repeated.push(Date.now()-begin);
   if(scope==='root'){await key('Escape','Escape',27);await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');}
   await click(600,500);nextCommit+=1.5;
  }
  await move(720+70*Math.sin(t*1.3),530+55*Math.cos(t*1.1));moves++;
  await new Promise(resolve=>setTimeout(resolve,Math.max(0,start+moves*1000/60-Date.now())));
 }
 const previewState=await evaluate(`window.benchupViewportBenchmarkState`);
 if(mode==='measure-tape' && !(previewState.tool==='TapeMeasure' && previewState.measurement_points?.length===2 && previewState.measurement_value))throw Error('Tape measure preview is missing: '+JSON.stringify(previewState));
 if(args.includes('--profile')){await writeFile(output+'.preview.cpu.json',JSON.stringify(await call('Profiler.stop')));await call('Profiler.start');}
 const commitStart=Date.now();
 await click(735,570);
 if(mode!=='draw-rectangle')await key('Escape','Escape',27);
 await evaluate(`new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))`);
 const commitMs=Date.now()-commitStart;
 const elapsed=Date.now()-start;
 await evaluate('window.benchupDrawingProbe.record=false');
 if(mode==='draw-rectangle'&&scope==='root'){await key('Escape','Escape',27);await evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');}
 const data=await evaluate(`(()=>{window.benchupDrawingProbe.record=false;const doc=JSON.parse(window.benchupBenchmarkDocument());return {visibility:document.visibilityState,focused:document.hasFocus(),samples:window.benchupDrawingProbe.samples,walkMutations:window.benchupDrawingProbe.walkMutations,faces:Object.keys(doc.model.faces).length,edges:Object.keys(doc.model.edges).length,guides:Object.keys(doc.model.guides??{}).length,measurement:window.benchupViewportBenchmarkState,selection:doc.selection,context:doc.selection_context}})()`);
 const intervals=data.samples.slice(1).map((time,i)=>time-data.samples[i]).sort((a,b)=>a-b);
 const createdEdges=data.edges-Object.keys(source.model.edges).length;
 const createdGuides=data.guides-Object.keys(source.model.guides??{}).length;
 if(mode==='measure-tape'?createdGuides<=0:createdEdges<=0)throw Error('Interaction did not commit new geometry: '+JSON.stringify(data));
 return {mode,visibility:data.visibility,focused:data.focused,frames:data.samples.length,draws:data.samples.length,fps:data.samples.length*1000/elapsed,mean_ms:elapsed/Math.max(1,data.samples.length),p95_ms:intervals[Math.floor(intervals.length*.95)],max_ms:intervals.at(-1),commit_ms:commitMs,repeated_commits_ms:repeated,walk_bridge_mutations:data.walkMutations,pointer_moves:moves,created_edges:createdEdges,created_guides:createdGuides,measurement_preview:previewState,measurement:data.measurement,faces:data.faces,edges:data.edges,edges_visible:true,final_selection:data.selection??[],final_context:data.context??null,size:[1440,900]};
}

async function validateSnapping() {
 const resources=performance.getEntriesByType('resource');
 const glue=resources.findLast(r=>/benchup-app-[^/]+\.js(?:\?|$)/.test(r.name))?.name;
 const wasm=resources.findLast(r=>r.name.endsWith('_bg.wasm'))?.name;
 if(!glue||!wasm)throw Error('Cannot locate app modules for snapping regression');
 const code=`const ready=import(${JSON.stringify(glue)}).then(async m=>{await m.default({module_or_path:${JSON.stringify(wasm)}});return m});self.onmessage=async e=>{try{const m=await ready;self.postMessage({result:JSON.parse(m.benchup_validate_snapping(e.data))})}catch(error){self.postMessage({error:String(error)})}}`;
 const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'}));
 const worker=new Worker(url,{type:'module'});URL.revokeObjectURL(url);
 try {
  return await new Promise((resolve,reject)=>{
   const timeout=setTimeout(()=>reject(Error('Snapping regression timed out')),55000);
   worker.onerror=e=>{clearTimeout(timeout);reject(Error(e.message))};
   worker.onmessage=e=>{clearTimeout(timeout);if(e.data.error)reject(Error(e.data.error));else resolve(e.data.result)};
   worker.postMessage(window.benchupBenchmarkDocument());
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
 if((await evaluate(`window.benchupViewportBenchmarkState`)).tool!=='Walk')throw Error('Walk tool did not activate');
 let placed=false;
 for(const [x,y] of [[500,700],[400,600],[700,450],[750,350],[500,550]]) {
  await click(x,y);await painted();
  placed=await evaluate(`document.getElementById('benchup-canvas').dataset.walkPlaced==='true'`);
  if(placed)break;
 }
 if(!placed)throw Error('Could not place walking person on a real scene floor');
 const initial=await evaluate(`window.benchupViewportBenchmarkState`);
 await evaluate(`window.benchupDrawingProbe.samples=[];window.benchupDrawingProbe.walkMutations=0;window.benchupDrawingProbe.record=true`);
 const start=Date.now();let moves=0,held='w',phase=0;
 await key('keyDown',held);
 while(Date.now()-start<seconds*1000){
  const t=(Date.now()-start)/1000,next=Math.floor(t/1.5)%4;
  if(next!==phase){await key('keyUp',held);held=['w','d','s','a'][next];await key('keyDown',held);phase=next;}
  await move(650+40*Math.sin(t*2),470+15*Math.sin(t*1.3));moves++;
  await new Promise(resolve=>setTimeout(resolve,Math.max(0,start+moves*1000/60-Date.now())));
 }
 await key('keyUp',held);await painted();
 const data=await evaluate(`(()=>{window.benchupDrawingProbe.record=false;return {visibility:document.visibilityState,focused:document.hasFocus(),samples:window.benchupDrawingProbe.samples,walkMutations:window.benchupDrawingProbe.walkMutations,state:window.benchupViewportBenchmarkState,placed:document.getElementById('benchup-canvas').dataset.walkPlaced,capture:document.getElementById('benchup-canvas').dataset.walkPointerCapture,pointer_error:window.benchupPointerLockError??null}})()`);
 const elapsed=Date.now()-start,intervals=data.samples.slice(1).map((time,i)=>time-data.samples[i]).sort((a,b)=>a-b);
 if(data.placed!=='true'||!data.state.walk_feet)throw Error('Walk stopped during the benchmark');
 if(args.includes('--require-pointer-lock')&&data.capture!=='locked')throw Error('Pointer capture unavailable: '+await evaluate(`JSON.stringify({state:document.getElementById('benchup-canvas').dataset.walkPointerCapture,error:window.benchupPointerLockError,focused:document.hasFocus()})`));
 if(JSON.stringify(initial.camera)===JSON.stringify(data.state.camera))throw Error('Walk camera did not move');
 return {mode:'walk',frames:data.samples.length,draws:data.samples.length,fps:data.samples.length*1000/elapsed,mean_ms:elapsed/data.samples.length,p95_ms:intervals[Math.floor(intervals.length*.95)],max_ms:intervals.at(-1),walk_bridge_mutations:data.walkMutations,pointer_moves:moves,walk_initial:initial,walk_final:data.state,pointer_capture:data.capture,pointer_capture_error:data.pointer_error,faces:Object.keys(source.model.faces).length,edges:Object.keys(source.model.edges).length,edges_visible:source.display.edges,final_selection:data.state.selected,final_context:data.state.context,size:[1440,900]};
}

// Select a visible box edge in a small nested group, keeping the entire model.
function prepareErase(source){
 const {model:m,camera:c}=source, parents=new Map();
 for(const o of Object.values(m.objects))for(const id of o.entities)parents.set(id,o.id);
 const hidden=new Set(m.hidden??[]), locked=new Set(m.locked??[]);
 const blocked=id=>{while(id!==undefined){if(hidden.has(id)||locked.has(id))return true;id=parents.get(id)}return false};
 const add=(a,b)=>a.map((v,i)=>v+b[i]), sub=(a,b)=>a.map((v,i)=>v-b[i]), mul=(a,n)=>a.map(v=>v*n), dot=(a,b)=>a.reduce((v,x,i)=>v+x*b[i],0), cross=(a,b)=>[a[1]*b[2]-a[2]*b[1],a[2]*b[0]-a[0]*b[2],a[0]*b[1]-a[1]*b[0]];
 const forward=mul([Math.cos(c.pitch)*Math.cos(c.yaw),Math.cos(c.pitch)*Math.sin(c.yaw),Math.sin(c.pitch)],-1),right=[-Math.sin(c.yaw),Math.cos(c.yaw),0],up=cross(right,forward),eye=sub(c.target,mul(forward,c.distance));
 const project=p=>{const d=sub(p,eye),z=dot(d,forward),scale=748/(2*Math.tan(c.fov/2)*z);return z>0?[532+dot(d,right)*scale,374-dot(d,up)*scale]:null};
 const position=id=>m.vertices[id].position;
 const hit=(origin,dir,a,b,c)=>{const e=sub(b,a),f=sub(c,a),h=cross(dir,f),det=dot(e,h);if(Math.abs(det)<1e-12)return null;const inv=1/det,q=sub(origin,a),u=dot(q,h)*inv;if(u<-1e-8||u>1+1e-8)return null;const r=cross(q,e),v=dot(dir,r)*inv;if(v<-1e-8||u+v>1+1e-8)return null;const t=dot(f,r)*inv;return t>0?t:null};
 let best;const edgeCandidates=[];
 for(const o of Object.values(m.objects)){
  if(o.kind!=='Group'||blocked(o.id)||o.entities.some(id=>m.objects[id])||o.entities.length>(scope==='leaf'?60:10000))continue;
  const faces=o.entities.filter(id=>m.faces[id]).map(id=>m.faces[id]);
  if(!faces.length||(scope==='leaf'&&faces.some(f=>f.holes?.length||f.vertices.length!==4)))continue;
  for(const id of o.entities){const e=m.edges[id];if(!e)continue;const a=position(e.a),b=position(e.b),mid=mul(add(a,b),.5),pixel=project(mid),pa=project(a),pb=project(b);if(!pixel||!pa||!pb||pixel[0]<200||pixel[0]>850||pixel[1]<160||pixel[1]>600||Math.hypot(pa[0]-pb[0],pa[1]-pb[1])<25)continue;
   const dir=sub(mid,eye),visible=scope!=='leaf'||!faces.some(f=>{const ps=f.vertices.map(position);return [hit(eye,dir,ps[0],ps[1],ps[2]),hit(eye,dir,ps[0],ps[2],ps[3])].some(t=>t!==null&&t<1-1e-6)});if(!visible)continue;
   const score=(pixel[0]-532)**2+(pixel[1]-374)**2;
   const candidate={edge:id,context:o.id,pixel:[pixel[0]+76,pixel[1]+111],point:mid,score};edgeCandidates.push(candidate);if(!best||score<best.score)best=candidate;
  }
 }
 const hiddenOnly=id=>{while(id!==undefined){if(hidden.has(id))return true;id=parents.get(id)}return false};
 const polygons=scope==='leaf'?[]:Object.values(m.faces).filter(f=>!hiddenOnly(f.id)).map(f=>{
  const ps=f.vertices.map(position),origin=ps[0];let normal=[0,0,0];
  for(let i=0;i<ps.length;i++)normal=add(normal,cross(sub(ps[i],origin),sub(ps[(i+1)%ps.length],origin)));
  const axis=normal.map(Math.abs).indexOf(Math.max(...normal.map(Math.abs))),axes=[0,1,2].filter(i=>i!==axis);
  return {origin,normal,axes,rings:[f.vertices,...(f.holes??[])].map(ring=>ring.map(id=>axes.map(i=>position(id)[i])))};
 });
 const inRing=(point,ring)=>{let inside=false;for(let i=0,j=ring.length-1;i<ring.length;j=i++){
  const a=ring[i],b=ring[j];if((a[1]>point[1])!==(b[1]>point[1])&&point[0]<(b[0]-a[0])*(point[1]-a[1])/(b[1]-a[1])+a[0])inside=!inside;
 }return inside};
 const visibleGlobally=point=>{const dir=sub(point,eye);return !polygons.some(f=>{
  const denominator=dot(dir,f.normal);if(Math.abs(denominator)<1e-12)return false;
  const t=dot(sub(f.origin,eye),f.normal)/denominator;if(t<=0||t>=1-1e-6)return false;
  const p=f.axes.map(i=>eye[i]+dir[i]*t);return inRing(p,f.rings[0])&&!f.rings.slice(1).some(ring=>inRing(p,ring));
 })};
 if(scope!=='leaf'){
  best=edgeCandidates.sort((a,b)=>a.score-b.score||b.edge-a.edge).find(candidate=>visibleGlobally(candidate.point));
 }
 if(!best)throw Error('No visible grouped box edge found for eraser benchmark');
 const document=structuredClone(source), leaf=best.context;
 let root=leaf;while(parents.has(root))root=parents.get(root);
 let context=scope==='leaf'?leaf:scope==='group'?root:null;
 let entity=best.edge,kind='edges';
 if(mode==='erase-face'){
  const candidates=m.objects[leaf].entities.filter(id=>m.faces[id]&&m.faces[id].vertices.length===4&&!m.faces[id].holes?.length).flatMap(id=>{
   const f=m.faces[id],ps=f.vertices.map(position),center=mul(ps.reduce(add,[0,0,0]),1/ps.length),pixel=project(center);
   if(!pixel)return [];
   const dir=sub(center,eye),front=scope!=='leaf'||!m.objects[leaf].entities.filter(id=>m.faces[id]).some(other=>{
    const q=m.faces[other].vertices.map(position);return [hit(eye,dir,q[0],q[1],q[2]),hit(eye,dir,q[0],q[2],q[3])].some(t=>t!==null&&t<1-1e-6);
   });
   return front&&(scope==='leaf'||visibleGlobally(center))?[{entity:id,pixel:[pixel[0]+76,pixel[1]+111],score:(pixel[0]-532)**2+(pixel[1]-374)**2}]:[];
  }).sort((a,b)=>a.score-b.score);
  if(!candidates.length)throw Error('No visible face for eraser benchmark');
  entity=candidates[0].entity;best.pixel=candidates[0].pixel;kind='faces';
 }
 if(mode==='erase-object'){
  entity=leaf;kind='objects';context=scope==='root'?null:root;
  const parent=parents.get(leaf);
  if(parent!==context){
   if(parent!==undefined)document.model.objects[parent].entities=document.model.objects[parent].entities.filter(id=>id!==leaf);
   if(context!==null)document.model.objects[context].entities=[...document.model.objects[context].entities,leaf].sort((a,b)=>a-b);
  }
 }else if(mode.startsWith('erase-')&&scope!=='leaf'){
  const children=document.model.objects[leaf].entities;document.model.objects[leaf].entities=[];
  if(context!==null)document.model.objects[context].entities=[...document.model.objects[context].entities,...children].sort((a,b)=>a-b);
 }
 document.selection=args.includes('--erase-with-key')?[entity]:[];document.selection_context=context;
 return {...best,context,entity,kind,document};
}
async function benchmarkErase(call,evaluate,seconds,setup){
 await call('Emulation.setFocusEmulationEnabled',{enabled:true});
 await evaluate('document.getElementById("benchup-canvas").focus()');
 const key=async(key,code,keyCode,modifiers=0)=>{for(const type of ['keyDown','keyUp'])await call('Input.dispatchKeyEvent',{type,key,code,windowsVirtualKeyCode:keyCode,modifiers})};
 const frames=()=>evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
 const inspect=()=>evaluate(`(()=>{const doc=JSON.parse(window.benchupBenchmarkDocument());return {present:!!doc.model.${setup.kind}[${setup.entity}],objects:Object.keys(doc.model.objects).length,edges:Object.keys(doc.model.edges).length,faces:Object.keys(doc.model.faces).length,state:window.benchupViewportBenchmarkState}})()`);
 await frames();
 const initial=await inspect();
 const keyboard=args.includes('--erase-with-key');
 if(initial.state.context!==setup.context||(keyboard&&!initial.state.selected.includes(setup.entity)))throw Error('Grouped edge fixture did not restore: '+JSON.stringify(initial));
 const [x,y]=setup.pixel;
 await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
 await frames();
 if(args.includes('--profile')){await call('Profiler.stop');await call('Profiler.start');}
 const hoverStart=Date.now();
 if(!keyboard){for(const type of ['mousePressed','mouseReleased'])await call('Input.dispatchMouseEvent',{type,x:38,y:235,button:'left',clickCount:1});await frames();await call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});}
 await frames();const hoverMs=Date.now()-hoverStart;
 const samples=[];
 await evaluate('window.benchupDrawingProbe.samples=[];window.benchupDrawingProbe.record=true');
 do{
  if(keyboard){for(const type of ['mousePressed','mouseReleased'])await call('Input.dispatchMouseEvent',{type,x,y,button:'left',clickCount:1});await frames();}
  const profileCommit=args.includes('--profile')&&samples.length===1;
  if(profileCommit){await call('Profiler.stop');await call('Profiler.start');}
  const begin=Date.now();let previewMs=0;
  if(keyboard)await key('Delete','Delete',46);
  else{await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});await frames();previewMs=Date.now()-begin;await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});}
  await frames();const commitMs=Date.now()-begin;
  if(profileCommit){await writeFile(output+'.commit.cpu.json',JSON.stringify(await call('Profiler.stop')));await call('Profiler.start');}
  const erased=await inspect();if(erased.present||(erased.edges===initial.edges&&erased.faces===initial.faces&&erased.objects===initial.objects))throw Error('The grouped edge was not erased: '+JSON.stringify({setup:{edge:setup.edge,context:setup.context,pixel:setup.pixel},erased}));
  const undoStart=Date.now();await key('z','KeyZ',90,2);await frames();const undoMs=Date.now()-undoStart;
  const restored=await inspect();if(!restored.present||restored.edges!==initial.edges||restored.faces!==initial.faces||restored.state.context!==setup.context)throw Error('Erase undo did not restore the grouped geometry: '+JSON.stringify(restored));
  samples.push({preview_ms:previewMs,commit_ms:commitMs,undo_ms:undoMs});
 }while(samples.length<3);
 // Measure continuous real eraser motion, including an actual deletion. Idle
 // frames during isolated click/undo timings are not an interaction FPS test.
 await evaluate('window.benchupDrawingProbe.samples=[];window.benchupDrawingProbe.record=true');
 if(keyboard){for(const type of ['mousePressed','mouseReleased'])await call('Input.dispatchMouseEvent',{type,x,y,button:'left',clickCount:1});await frames();}
 const interactiveStart=Date.now();let moves=0,released=false,interactiveCommitMs=0;
 if(!keyboard)await call('Input.dispatchMouseEvent',{type:'mousePressed',x,y,button:'left',clickCount:1});
 while(Date.now()-interactiveStart<seconds*1000){
  const t=(Date.now()-interactiveStart)/1000;
  if(!released&&t>=seconds*.5){
   const commitStart=Date.now();
   if(keyboard)await key('Delete','Delete',46);
   else await call('Input.dispatchMouseEvent',{type:'mouseReleased',x,y,button:'left',clickCount:1});
   await frames();interactiveCommitMs=Date.now()-commitStart;released=true;
  }
  await call('Input.dispatchMouseEvent',{type:'mouseMoved',x:x+Math.sin(t*2),y:y+Math.cos(t*2)});moves++;
  await new Promise(resolve=>setTimeout(resolve,Math.max(0,interactiveStart+moves*1000/60-Date.now())));
 }
 const draws=await evaluate('window.benchupDrawingProbe.record=false;window.benchupDrawingProbe.samples.length');
 const elapsed=Date.now()-interactiveStart;
 const erased=await inspect();if(erased.present)throw Error('Interactive eraser motion did not delete its entity');
 if(erased.state.resident_builds!==initial.state.resident_builds)throw Error('A local erase rebuilt the retained scene: '+JSON.stringify({initial:initial.state.resident_builds,after:erased.state.resident_builds}));
 const deletedShot=await call('Page.captureScreenshot',{format:'png',captureBeyondViewport:false});
 await writeFile(output+'.erased.png',Buffer.from(deletedShot.data,'base64'));
 await key('z','KeyZ',90,2);await frames();
 const restored=await inspect();if(!restored.present||restored.edges!==initial.edges||restored.faces!==initial.faces)throw Error('Interactive eraser undo failed');
 return {mode,resident_builds:erased.state.resident_builds,entity:setup.entity,entity_kind:setup.kind,objects:initial.objects,interactive_commit_ms:interactiveCommitMs,pointer_moves:moves,keyboard,edge:setup.edge,context:setup.context,pixel:setup.pixel,hover_ms:hoverMs,samples,commit_mean_ms:samples.reduce((n,s)=>n+s.commit_ms,0)/samples.length,frames:draws,draws,fps:draws*1000/elapsed,faces:initial.faces,edges:initial.edges,size:[1440,900],final_context:setup.context,final_selection:[]};
}

async function validateEraseCaches(context,edge){
 const resources=performance.getEntriesByType('resource');
 const glue=resources.findLast(r=>/benchup-app-[^/]+\.js(?:\?|$)/.test(r.name))?.name;
 const wasm=resources.findLast(r=>r.name.endsWith('_bg.wasm'))?.name;
 if(!glue||!wasm)throw Error('Cannot locate app modules for erase regression');
 const code=`const ready=import(${JSON.stringify(glue)}).then(async m=>{await m.default({module_or_path:${JSON.stringify(wasm)}});return m});self.onmessage=async e=>{try{const m=await ready;self.postMessage({result:JSON.parse(m.benchup_validate_erase_caches(e.data.source,BigInt(e.data.context),BigInt(e.data.edge)))})}catch(error){self.postMessage({error:String(error)})}}`;
 const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'}));
 const worker=new Worker(url,{type:'module'});URL.revokeObjectURL(url);
 try{return await new Promise((resolve,reject)=>{
  const timeout=setTimeout(()=>reject(Error('Erase cache regression timed out')),55000);
  worker.onerror=e=>{clearTimeout(timeout);reject(Error(e.message))};
  worker.onmessage=e=>{clearTimeout(timeout);if(e.data.error)reject(Error(e.data.error));else resolve(e.data.result)};
  worker.postMessage({source:window.benchupBenchmarkDocument(),context,edge});
 })}finally{worker.terminate()}
}

// Reconstruct the exact recovery checkpoint outside measured UI operations.
function reconstructAutosave(source, edits) {
 if(!edits)return source;
 const base=JSON.parse(source), saved=JSON.parse(edits).checkpoint, model=base.model;
 for(const [field, changes] of Object.entries(saved.model)) {
  if(field==='next_id'){model.next_id=changes[1];continue}
  if(field==='edit_context')continue;
  if(field==='hidden'||field==='locked'){
   const set=new Set(model[field]??[]);
   for(const id of changes.removed??[])set.delete(id);
   for(const id of changes.added??[])set.add(id);
   model[field]=[...set].sort((a,b)=>a-b);continue;
  }
  const map=model[field]??={};
  for(const [id,,after] of changes){if(after===null)delete map[id];else map[id]=after}
 }
 const h=base.history??{format_version:1,undo:[],redo:[]}, delta=saved.history;
 const history={format_version:h.format_version,undo:[...h.undo.slice(0,delta.undo_prefix),...delta.undo],redo:[...h.redo.slice(0,delta.redo_prefix),...delta.redo],actions:delta.actions};
 return JSON.stringify({...saved.metadata,model,history});
}

// Execute the production sunlight fragment helper on a hardware GPU. Exercise
// self/coplanar rejection, contact shadows, empty pixels and optional filtering.
async function validateSunlight(shader) {
 const adapter=await navigator.gpu?.requestAdapter();
 if(!adapter||adapter.info.isFallbackAdapter)throw Error('Hardware WebGPU adapter required for sunlight regression');
 const device=await adapter.requestDevice();
 try {
  device.pushErrorScope('validation');
  const module=device.createShaderModule({code:shader+`
@vertex
fn test_sun_vertex(@builtin(vertex_index) vertex: u32, @builtin(instance_index) instance: u32) -> Output {
 let positions=array<vec2<f32>,3>(vec2<f32>(-1.0,-1.0),vec2<f32>(3.0,-1.0),vec2<f32>(-1.0,3.0));
 let depths=array<f32,8>(0.45,0.45,0.6,0.3,0.4000005,0.40001,0.4,0.6);
 let centers=array<f32,8>(-0.5,-0.5,0.5,-0.5,-0.5,-0.5,-0.5,0.0);
 var value: Output;
 value.position=vec4<f32>(positions[vertex],0.5,1.0);
 let xy=vec2<f32>(centers[instance],0.0)+positions[vertex]*0.01;
 value.shadow=vec3<f32>(xy,depths[instance]+0.2*(xy.x-centers[instance]));
 value.faces=vec4<u32>(select(7u,11u,instance==1u),0u,0u,0u);
 return value;
}
@fragment
fn test_sun_fragment(value: Output) -> @location(0) vec4<f32> {
 return vec4<f32>(shadow_coverage(value),0.0,0.0,1.0);
}`});
  const compilation=await module.getCompilationInfo();
  const errors=compilation.messages.filter(m=>m.type==='error');
  if(errors.length)throw Error(JSON.stringify(errors));
  const pipeline=device.createRenderPipeline({layout:'auto',vertex:{module,entryPoint:'test_sun_vertex'},fragment:{module,entryPoint:'test_sun_fragment',targets:[{format:'rgba8unorm'}]}});
  const sunlight=device.createTexture({size:[32,32],format:'rg32uint',usage:GPUTextureUsage.TEXTURE_BINDING|GPUTextureUsage.COPY_DST});
  const bits=new Uint32Array(32*32*2),f=new Float32Array(1),u=new Uint32Array(f.buffer);
  for(let y=0;y<32;y++)for(let x=0;x<16;x++){const i=(y*32+x)*2;bits[i]=11;f[0]=.3+.4*(x+.5)/32;bits[i+1]=u[0];}
  device.queue.writeTexture({texture:sunlight},bits,{bytesPerRow:256},[32,32]);
  const uniform=device.createBuffer({size:480,usage:GPUBufferUsage.UNIFORM|GPUBufferUsage.COPY_DST});
  const camera=new Float32Array(120);camera[32]=.35;camera[34]=.000002;
  const bind0=device.createBindGroup({layout:pipeline.getBindGroupLayout(0),entries:[{binding:0,resource:{buffer:uniform}}]});
  const bind1=device.createBindGroup({layout:pipeline.getBindGroupLayout(1),entries:[{binding:1,resource:sunlight.createView()}]});
  const target=device.createTexture({size:[8,1],format:'rgba8unorm',usage:GPUTextureUsage.RENDER_ATTACHMENT|GPUTextureUsage.COPY_SRC});
  const readback=device.createBuffer({size:256,usage:GPUBufferUsage.COPY_DST|GPUBufferUsage.MAP_READ});
  const checks=[];
  for(const antialias of [false,true]){
   camera[33]=Number(antialias);device.queue.writeBuffer(uniform,0,camera);
   const encoder=device.createCommandEncoder(),pass=encoder.beginRenderPass({colorAttachments:[{view:target.createView(),clearValue:[0,0,0,0],loadOp:'clear',storeOp:'store'}]});
   pass.setPipeline(pipeline);pass.setBindGroup(0,bind0);pass.setBindGroup(1,bind1);
   for(let i=0;i<8;i++){pass.setViewport(i,0,1,1,0,1);pass.draw(3,1,0,i)}
   pass.end();encoder.copyTextureToBuffer({texture:target},{buffer:readback,bytesPerRow:256},[8,1]);device.queue.submit([encoder.finish()]);
   await readback.mapAsync(GPUMapMode.READ);const bytes=new Uint8Array(readback.getMappedRange());const coverage=Array.from({length:8},(_,i)=>bytes[i*4]/255);readback.unmap();
   const expected=[1,0,0,0,0,1,0,antialias?.5:0];
   if(coverage.some((v,i)=>Math.abs(v-expected[i])>.03))throw Error('Incorrect sunlight coverage: '+JSON.stringify({antialias,coverage,expected}));
   checks.push({antialias,coverage});
  }
  const error=await device.popErrorScope();if(error)throw Error(error.message);
  return {passed:true,cases:16,checks};
 } finally {device.destroy()}
}

async function benchmarkBoxSelection(call,evaluate,seconds) {
 const frames=()=>evaluate('new Promise(resolve=>requestAnimationFrame(()=>requestAnimationFrame(resolve)))');
 const move=(x,y)=>call('Input.dispatchMouseEvent',{type:'mouseMoved',x,y});
 const button=(type,x,y)=>call('Input.dispatchMouseEvent',{type,x,y,button:'left',clickCount:1});
 const state=()=>evaluate(`(()=>{const s=window.benchupViewportBenchmarkState;return {count:s.selected_count??s.selected.length,context:s.context,tool:s.tool}})()`);
 await call('Emulation.setFocusEmulationEnabled',{enabled:true});
 await move(38,110);await button('mousePressed',38,110);await button('mouseReleased',38,110);await frames();
 const initial=await state();
 const from=args.includes('--reverse-box')?[1040,740]:[96,132];
 const to=args.includes('--reverse-box')?[96,132]:[1118,836];
 const samples=[];
 for(let attempt=0;attempt<3;attempt++){
  // A new replacement marquee clears/replaces the preceding selection on release.
  // Escape closes an editing scope, so repeated drags deliberately keep it open.
  await move(...from);await button('mousePressed',...from);await frames();
  await evaluate('window.benchupDrawingProbe.samples=[];window.benchupDrawingProbe.record=true');
  const begin=Date.now();let moves=0;
  while(Date.now()-begin<seconds*500){
   const t=Math.min(1,(Date.now()-begin)/(seconds*500));
   await move(from[0]+(to[0]-from[0])*t,from[1]+(to[1]-from[1])*t);moves++;
   await new Promise(resolve=>setTimeout(resolve,Math.max(0,begin+moves*1000/60-Date.now())));
  }
  await move(...to);await frames();
  const drag=await evaluate('window.benchupDrawingProbe.record=false;window.benchupDrawingProbe.samples');
  const dragElapsed=Date.now()-begin;
  if(args.includes('--profile')){await call('Profiler.stop');await call('Profiler.start');}
  const release=Date.now();await button('mouseReleased',...to);await frames();
  const releaseMs=Date.now()-release;
  if(args.includes('--profile')){await writeFile(output+`.release-${attempt}.cpu.json`,JSON.stringify(await call('Profiler.stop')));await call('Profiler.start');}
  const selected=await state();
  if(!selected.count||selected.context!==initial.context)throw Error('Large marquee failed to select in the current scope: '+JSON.stringify({initial,selected}));
  await evaluate('window.benchupDrawingProbe.samples=[];window.benchupDrawingProbe.record=true');
  const hold=Date.now();moves=0;
  while(Date.now()-hold<seconds*500){
   const t=(Date.now()-hold)/1000;await move(700+50*Math.sin(t*2),480+40*Math.cos(t));moves++;
   await new Promise(resolve=>setTimeout(resolve,Math.max(0,hold+moves*1000/60-Date.now())));
  }
  const held=await evaluate('window.benchupDrawingProbe.record=false;window.benchupDrawingProbe.samples');
  const holdElapsed=Date.now()-hold;
  const intervals=held.slice(1).map((v,i)=>v-held[i]).sort((a,b)=>a-b);
  samples.push({selected:selected.count,release_ms:releaseMs,drag_fps:drag.length*1000/dragElapsed,selected_fps:held.length*1000/holdElapsed,selected_p95_ms:intervals[Math.floor(intervals.length*.95)],drag_frames:drag.length,selected_frames:held.length,drag_elapsed_ms:dragElapsed,selected_elapsed_ms:holdElapsed});
  await writeFile(output+'.samples.json',JSON.stringify(samples,null,2));
 }
 const saved=await evaluate(`(()=>{const doc=JSON.parse(window.benchupBenchmarkDocument()),state=window.benchupViewportBenchmarkState,overlay=JSON.parse(localStorage.getItem('benchup.selection.v1'));return {faces:Object.keys(doc.model.faces).length,edges:Object.keys(doc.model.edges).length,selection:overlay?.selected??state.selected,context:overlay?.context??state.context,camera:state.camera,guides:doc.display.guides,count:state.selected_count??state.selected.length}})()`);
 if(saved.selection.length!==saved.count)throw Error('Full selection was not persisted: '+JSON.stringify({saved:saved.selection.length,current:saved.count}));
 const project_selection_regression=await evaluate(`(${validateProjectBox.toString()})(${JSON.stringify(saved.selection)},${JSON.stringify({camera:saved.camera,size:[1064,748],start:[from[0]-76,from[1]-111],end:[to[0]-76,to[1]-111],guides:saved.guides})})`);
 const total=samples.reduce((n,s)=>n+s.drag_elapsed_ms+s.selected_elapsed_ms+s.release_ms,0),draws=samples.reduce((n,s)=>n+s.drag_frames+s.selected_frames,0);
 return {mode:'box-select',fps:draws*1000/total,draws,frames:draws,samples,release_mean_ms:samples.reduce((n,s)=>n+s.release_ms,0)/samples.length,release_max_ms:Math.max(...samples.map(s=>s.release_ms)),faces:saved.faces,edges:saved.edges,final_selection:saved.selection,final_context:saved.context,project_selection_regression,size:[1440,900],flattened:args.includes('--flatten')};
}

async function validateMarkers(shader) {
 const adapter=await navigator.gpu?.requestAdapter();
 if(!adapter||adapter.info.isFallbackAdapter)throw Error('Hardware WebGPU adapter required for marker regression');
 const device=await adapter.requestDevice();
 try {
  device.pushErrorScope('validation');
  const module=device.createShaderModule({code:shader});
  const pipeline=device.createRenderPipeline({layout:'auto',vertex:{module,entryPoint:'marker_vertex',buffers:[{arrayStride:28,stepMode:'instance',attributes:[{shaderLocation:0,offset:0,format:'float32x3'},{shaderLocation:1,offset:12,format:'uint32x4'}]}]},fragment:{module,entryPoint:'marker_color',targets:[{format:'rgba8unorm',blend:{color:{srcFactor:'one',dstFactor:'one-minus-src-alpha'},alpha:{srcFactor:'one',dstFactor:'one-minus-src-alpha'}}}]}});
  const values=new ArrayBuffer(28*8),f=new Float32Array(values),u=new Uint32Array(values);
  for(let i=0;i<8;i++){const row=Math.floor(i/2),base=i*7;f.set([i%2?.5:-.5,[-.5,0,.5,0][row],row===3?-1:.5],base);u[base+3]=row+1;}
  const vertices=device.createBuffer({size:values.byteLength,usage:GPUBufferUsage.VERTEX|GPUBufferUsage.COPY_DST});device.queue.writeBuffer(vertices,0,values);
  const states=device.createTexture({size:[512,1],format:'r32uint',usage:GPUTextureUsage.TEXTURE_BINDING|GPUTextureUsage.COPY_DST});
  const flags=new Uint32Array(512);flags[1]=1;flags[3]=65;flags[4]=1;device.queue.writeTexture({texture:states},flags,{bytesPerRow:2048},[512,1]);
  const uniform=device.createBuffer({size:480,usage:GPUBufferUsage.UNIFORM|GPUBufferUsage.COPY_DST});
  const camera=new Float32Array(120);for(let i=0;i<4;i++)camera[i*5]=1;camera.set([64,64,1,1.8],64);camera.set([1,0,0,1],88);camera.set([1,1,1,1],108);
  const bind=device.createBindGroup({layout:pipeline.getBindGroupLayout(0),entries:[{binding:0,resource:{buffer:uniform}},{binding:1,resource:states.createView()}]});
  const target=device.createTexture({size:[64,64],format:'rgba8unorm',usage:GPUTextureUsage.RENDER_ATTACHMENT|GPUTextureUsage.COPY_SRC});
  const readback=device.createBuffer({size:16384,usage:GPUBufferUsage.MAP_READ|GPUBufferUsage.COPY_DST});
  const checks=[];
  for(const overlay of [false,true]){
   new Uint32Array(camera.buffer)[75]=overlay?4:0;device.queue.writeBuffer(uniform,0,camera);
   const encoder=device.createCommandEncoder(),pass=encoder.beginRenderPass({colorAttachments:[{view:target.createView(),clearValue:[0,0,0,1],loadOp:'clear',storeOp:'store'}]});
   pass.setPipeline(pipeline);pass.setBindGroup(0,bind);pass.setVertexBuffer(0,vertices);pass.draw(6,8);pass.end();
   encoder.copyTextureToBuffer({texture:target},{buffer:readback,bytesPerRow:256},[64,64]);device.queue.submit([encoder.finish()]);
   await readback.mapAsync(GPUMapMode.READ);const bytes=new Uint8Array(readback.getMappedRange());
   const pixel=(x,y)=>Array.from(bytes.slice((y*64+x)*4,(y*64+x)*4+3));
   const centers=[pixel(16,48),pixel(48,48),pixel(16,32),pixel(48,32),pixel(16,16),pixel(48,16)];
   const expected=[true,true,false,false,overlay,overlay];
   if(centers.some((rgb,i)=>expected[i]?rgb.some(v=>v<240):rgb.some(v=>v!==0)))throw Error('Selection marker visibility differs: '+JSON.stringify({overlay,centers}));
   const stroke=pixel(19,48);if(stroke[0]<80||stroke[1]>30||stroke[2]>30)throw Error('Marker stroke is missing: '+JSON.stringify(stroke));
   if(pixel(23,48).some(v=>v!==0))throw Error('Marker expanded beyond its screen-space radius');
   checks.push({overlay,centers,stroke});readback.unmap();
  }
  const error=await device.popErrorScope();if(error)throw Error(error.message);
  return {passed:true,cases:16,checks};
 } finally {device.destroy()}
}

async function validateProjectBox(expected,query) {
 const glue=performance.getEntriesByType('resource').findLast(r=>/benchup-app-[^/]+\.js(?:\?|$)/.test(r.name))?.name;
 const wasm=performance.getEntriesByType('resource').findLast(r=>r.name.endsWith('_bg.wasm'))?.name;
 const code=`const ready=import(${JSON.stringify(glue)}).then(async m=>{await m.default({module_or_path:${JSON.stringify(wasm)}});return m});self.onmessage=async e=>{try{const m=await ready;const ids=JSON.parse(m.benchup_box_selection(e.data.source,JSON.stringify(e.data.query)));if(JSON.stringify(ids)!==JSON.stringify(e.data.expected))throw Error('Marquee selection differs from the independent ownership index');self.postMessage({passed:true,selected:ids.length})}catch(error){self.postMessage({error:String(error)})}}`;
 const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'})),worker=new Worker(url,{type:'module'});URL.revokeObjectURL(url);
 try{return await new Promise((resolve,reject)=>{const timer=setTimeout(()=>reject(Error('Project box-selection regression timed out')),60000);worker.onerror=e=>{clearTimeout(timer);reject(Error(e.message))};worker.onmessage=e=>{clearTimeout(timer);if(e.data.error)reject(Error(e.data.error));else resolve(e.data)};worker.postMessage({source:window.benchupBenchmarkBaseDocument(),query,expected});})}finally{worker.terminate()}
}
