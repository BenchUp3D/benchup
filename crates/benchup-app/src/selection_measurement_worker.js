// One worker per geometry revision. Source parsing and all exact solid work run
// there; selection requests are tiny and old-project jobs can be terminated.
const moduleURL = performance.getEntriesByType('resource').findLast(entry => /benchup-app-[^/]+\.js(?:\?|$)/.test(entry.name))?.name;
if (!moduleURL) throw new Error('Cannot locate the application module for measurements.');
const wasmURL=performance.getEntriesByType('resource').findLast(entry=>entry.name.endsWith('_bg.wasm'))?.name;
if(!wasmURL)throw new Error('Cannot locate the application WebAssembly module for measurements.');
const code = `
const initialized=import(${JSON.stringify(moduleURL)}).then(async module=>{await module.default({module_or_path:${JSON.stringify(wasmURL)}});return module});
self.onmessage=async event=>{
 const message=event.data;
 try {
  const module=await initialized;
  if(message.source!==undefined)module.benchup_measurement_load(message.source);
  else if(message.merge!==undefined)module.benchup_measurement_merge(message.merge);
  else self.postMessage({id:message.id,result:module.benchup_measure_selection(message.selected,message.context)});
 }catch(error){self.postMessage({id:message.id,error:String(error)})}
};`;
const url=URL.createObjectURL(new Blob([code],{type:'text/javascript'}));
const worker=new Worker(url,{type:'module'});URL.revokeObjectURL(url);
let sequence=0;const pending=new Map();
const fail=error=>{for(const request of pending.values())request.reject(error);pending.clear()};
worker.onmessage=event=>{
 const message=event.data,request=pending.get(message.id);
 if(!request){if(message.error)fail(new Error(message.error));return}
 pending.delete(message.id);if(message.error)request.reject(new Error(message.error));else request.resolve(message.result);
};
worker.onerror=event=>fail(new Error(event.message||'Measurement worker failed.'));
worker.onmessageerror=()=>fail(new Error('Invalid measurement worker response.'));
worker.postMessage({source});
return {
 merge(source){worker.postMessage({merge:source})},
 request(selected,context){return new Promise((resolve,reject)=>{const id=++sequence;pending.set(id,{resolve,reject});worker.postMessage({id,selected,context})})},
 stop(){worker.terminate();fail(new Error('Measurement source replaced.'))}
};
