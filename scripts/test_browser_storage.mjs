#!/usr/bin/env node
// Exercise the production recovery module in a real, isolated browser.
// No Rust build or running app is required.
import assert from 'node:assert/strict';
import {createServer} from 'node:http';
import {readFile, mkdtemp, rm, readdir, access} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import {homedir, tmpdir} from 'node:os';
import {join} from 'node:path';
const source = await readFile(new URL('../crates/benchup-app/src/browser_storage.js', import.meta.url));
const candidates = [process.env.CHROME_BIN, '/usr/bin/chromium', '/usr/bin/google-chrome'];
for (const name of (await readdir(join(homedir(), '.cache/ms-playwright')).catch(() => [])).filter(n => /^chromium-\d+$/.test(n)).sort().reverse()) {
    candidates.push(join(homedir(), '.cache/ms-playwright', name, 'chrome-linux64/chrome'));
}
let executable;
for (const candidate of candidates.filter(Boolean)) {
    try { await access(candidate); executable = candidate; break; } catch {}
}
assert(executable, 'Chromium is required; set CHROME_BIN.');
const server = createServer((req, res) => {
    res.setHeader('Cache-Control', 'no-store');
    if (req.url.split('?')[0] === '/browser_storage.js') {
        res.setHeader('Content-Type', 'text/javascript'); res.end(source);
    } else { res.setHeader('Content-Type', 'text/html'); res.end('<!doctype html><title>Recovery test</title>'); }
});
await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
const url = `http://127.0.0.1:${server.address().port}/`;
const profile = await mkdtemp(join(tmpdir(), 'benchup-storage-test-'));
const browser = spawn(executable, ['--headless=new', '--no-sandbox', '--disable-dev-shm-usage',
    '--no-first-run', '--no-default-browser-check', '--remote-debugging-port=0', `--user-data-dir=${profile}`, 'about:blank'],
    {stdio: ['ignore', 'ignore', 'pipe']});
let diagnostics = '';
browser.stderr.on('data', chunk => { diagnostics = (diagnostics + chunk).slice(-10000); });
const pause = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(action, label, timeout = 15000) {
    const deadline = Date.now() + timeout;
    while (Date.now() < deadline) { const result = await action(); if (result) return result; await pause(50); }
    throw Error(`Timed out: ${label}\n${diagnostics}`);
}
let socket;
try {
    const port = await until(async () => {
        try { return (await readFile(join(profile, 'DevToolsActivePort'), 'utf8')).split('\n')[0]; } catch { return null; }
    }, 'browser startup');
    const tabs = await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
    socket = new WebSocket(tabs.find(tab => tab.type === 'page').webSocketDebuggerUrl);
    await new Promise((resolve, reject) => { socket.onopen = resolve; socket.onerror = reject; });
    let sequence = 0;
    const requests = new Map();
    socket.onmessage = event => {
        const value = JSON.parse(event.data);
        if (value.id) {
            const request = requests.get(value.id); requests.delete(value.id);
            if (value.error) request.reject(Error(JSON.stringify(value.error))); else request.resolve(value.result);
        }
    };
    const call = (method, params = {}) => new Promise((resolve, reject) => {
        const id = ++sequence;
        const timer = setTimeout(() => { requests.delete(id); reject(Error(`CDP timeout: ${method}`)); }, 20000);
        requests.set(id, {resolve: value => { clearTimeout(timer); resolve(value); }, reject: error => { clearTimeout(timer); reject(error); }});
        socket.send(JSON.stringify({id, method, params}));
    });
    async function evaluate(expression) {
        const value = await call('Runtime.evaluate', {expression, awaitPromise: true, returnByValue: true});
        if (value.exceptionDetails) throw Error(value.exceptionDetails.exception?.description || JSON.stringify(value.exceptionDetails));
        return value.result.value;
    }
    await call('Page.enable');
    await call('Page.navigate', {url});
    await until(() => evaluate(`location.href === ${JSON.stringify(url)} && document.readyState === 'complete'`), 'test page');
    async function load() {
        await evaluate(`(async()=>{globalThis.recovery = await import('/browser_storage.js'); await recovery.loadRecovery(); return true})()`);
    }
    async function reload() {
        await call('Page.reload');
        await until(() => evaluate(`typeof recovery === 'undefined' && document.readyState === 'complete'`).catch(() => false), 'reload');
        await load();
    }
    const result = await evaluate(`(async()=>{
        localStorage.setItem('benchup.document.v1', '{"name":"Legacy project"}');
        localStorage.setItem('benchup.edits.v1', '{"legacyEdits":true}');
        localStorage.setItem('benchup.selection.v1', '{"selected":[7]}');
        globalThis.recovery = await import('/browser_storage.js');
        await recovery.loadRecovery();
        return {document:recovery.readRecovery('benchup.document.v1'),
            edits:recovery.readRecovery('benchup.edits.v1'), selection:recovery.readRecovery('benchup.selection.v1'),
            oldDocument:localStorage.getItem('benchup.document.v1')};
    })()`);
    assert.equal(JSON.parse(result.document).name, 'Legacy project');
    assert.equal(JSON.parse(result.edits).legacyEdits, true);
    assert.deepEqual(JSON.parse(result.selection).selected, [7]);
    assert.equal(result.oldDocument, null);
    await reload();
    assert.equal(await evaluate(`JSON.parse(recovery.readRecovery('benchup.document.v1')).name`), 'Legacy project');
    console.log('PASS: legacy snapshot and overlays migrate atomically and reload.');

    const size = await evaluate(`(()=>{
        const png = Array.from({length:2000000},(_,i)=>i%256);
        globalThis.snapshot = JSON.stringify({name:'Image project',model:{
            image_assets:{photo:{png,pixels:[1024,1024]}},reference_images:{1:{asset:'photo',settings:{opacity:0.75,zoom:1}}}}});
        let quotaExceeded = false;
        try {localStorage.setItem('oversized-test',snapshot);} catch(e) {quotaExceeded=e.name==='QuotaExceededError';}
        if (!quotaExceeded) throw Error('Fixture must exceed localStorage quota.');
        recovery.writeRecovery('benchup.document.v1',snapshot,true);
        if (!recovery.recoveryPending()) throw Error('Must stay pending until IndexedDB commits.');
        recovery.writeRecovery('benchup.edits.v1',JSON.stringify({opacity:0.5,zoom:2}),false);
        recovery.writeRecovery('benchup.edits.v1',JSON.stringify({opacity:0.25,zoom:3,rotation:90}),false);
        recovery.writeRecovery('benchup.selection.v1','{"selected":[1]}',false);
        return snapshot.length;
    })()`);
    await until(() => evaluate('!recovery.recoveryPending()'), 'large image commit');
    assert.equal(await evaluate('recovery.takeRecoveryResult()'), '');
    await reload();
    assert.deepEqual(await evaluate(`(()=>{const doc=JSON.parse(recovery.readRecovery('benchup.document.v1'));return {name:doc.name,bytes:doc.model.image_assets.photo.png.length,lastByte:doc.model.image_assets.photo.png.at(-1)}})()`),
        {name:'Image project',bytes:2000000,lastByte:127});
    assert.deepEqual(await evaluate(`JSON.parse(recovery.readRecovery('benchup.edits.v1'))`), {opacity:0.25,zoom:3,rotation:90});
    assert.deepEqual(await evaluate(`JSON.parse(recovery.readRecovery('benchup.selection.v1')).selected`), [1]);
    console.log(`PASS: ${size} characters exceed localStorage quota but image and rapid edits survive refresh.`);

    await evaluate(`(()=>{
        const original=IDBObjectStore.prototype.put;
        IDBObjectStore.prototype.put=function(value,key){
            // A duplicate key makes the request fail asynchronously and aborts
            // the transaction, even when an explicit commit was submitted.
            return key==='benchup.edits.v1' ? this.add(value,key) : original.call(this,value,key);
        };
        recovery.writeRecovery('benchup.edits.v1','{"lost":true}',false);
        IDBObjectStore.prototype.put=original;
        recovery.writeRecovery('benchup.selection.v1','{"selected":[]}',false);
        return true;
    })()`);
    await until(() => evaluate('!recovery.recoveryPending()'), 'failed transaction');
    assert.match(await evaluate('recovery.takeRecoveryResult()'), /Could not save workspace/);
    await reload();
    assert.deepEqual(await evaluate(`JSON.parse(recovery.readRecovery('benchup.edits.v1'))`), {opacity:0.25,zoom:3,rotation:90});
    console.log('PASS: aborted writes report failure even after a successful selection save; committed changes remain intact.');

    await evaluate(`(()=>{
        recovery.writeRecovery('benchup.edits.v1','{"obsolete":true}',false);
        recovery.writeRecovery('benchup.document.v1','{"name":"Latest snapshot"}',true);
        return true;
    })()`);
    await until(() => evaluate('!recovery.recoveryPending()'), 'snapshot compaction');
    await reload();
    assert.equal(await evaluate(`JSON.parse(recovery.readRecovery('benchup.document.v1')).name`), 'Latest snapshot');
    assert.equal(await evaluate(`recovery.readRecovery('benchup.edits.v1') ?? null`), null);
    console.log('PASS: snapshot compaction removes stale edits in the same transaction.');

    await evaluate(`(()=>{
        recovery.writeRecovery('benchup.document.v1','{"name":"Earlier successful save"}',true);
        const original=IDBObjectStore.prototype.put;
        IDBObjectStore.prototype.put=()=>{throw new DOMException('Storage full','QuotaExceededError');};
        try {recovery.writeRecovery('benchup.edits.v1','{"latest":true}',false);} catch(_) {}
        finally {IDBObjectStore.prototype.put=original;}
        return true;
    })()`);
    await until(() => evaluate('!recovery.recoveryPending()'), 'synchronous save failure');
    assert.match(await evaluate('recovery.takeRecoveryResult()'), /Storage full/);
    await evaluate(`recovery.writeRecovery('benchup.document.v1','{"name":"Retry succeeded"}',true)`);
    await until(() => evaluate('!recovery.recoveryPending()'), 'retry');
    assert.equal(await evaluate('recovery.takeRecoveryResult()'), '');
    await reload();
    assert.equal(await evaluate(`JSON.parse(recovery.readRecovery('benchup.document.v1')).name`), 'Retry succeeded');
    console.log('PASS: an earlier success cannot mask a newer submission failure; a committed retry clears the error.');

    await evaluate(`(()=>{
        recovery.writeRecovery('benchup.document.v1','{"name":"Immediate refresh"}',true);
        recovery.writeRecovery('benchup.edits.v1','{"opacity":0.2,"rotation":45}',false);
        return true;
    })()`);
    await reload();
    assert.equal(await evaluate(`JSON.parse(recovery.readRecovery('benchup.document.v1')).name`), 'Immediate refresh');
    assert.deepEqual(await evaluate(`JSON.parse(recovery.readRecovery('benchup.edits.v1'))`), {opacity:0.2,rotation:45});
    console.log('PASS: immediately refreshing after submitting edits preserves the latest write.');



    await evaluate(`(async()=>{
        const original = indexedDB.open;
        indexedDB.open = () => { throw new DOMException('Storage blocked','SecurityError'); };
        localStorage.setItem('benchup.document.v1','{"name":"Fallback project"}');
        try {
            globalThis.fallback = await import('/browser_storage.js?fallback');
            await fallback.loadRecovery();
            fallback.writeRecovery('benchup.document.v1','{"name":"Fallback saved"}',true);
        } finally { indexedDB.open = original; }
        return true;
    })()`);
    assert.equal(await evaluate(`JSON.parse(fallback.readRecovery('benchup.document.v1')).name`), 'Fallback saved');
    assert.equal(await evaluate(`JSON.parse(localStorage.getItem('benchup.document.v1')).name`), 'Fallback saved');
    assert.equal(await evaluate(`fallback.recoveryPending()`), false);
    assert.equal(await evaluate(`(()=>{try {fallback.writeRecovery('benchup.document.v1','x'.repeat(6*1024*1024),true);return false;}catch(e){return e.name==='QuotaExceededError';}})()`), true);
    console.log('PASS: blocked IndexedDB falls back to legacy storage and reports quota failures.');

} finally {
    socket?.close();
    browser.kill();
    await new Promise(resolve => browser.exitCode !== null ? resolve() : browser.once('exit',resolve));
    await new Promise(resolve => server.close(resolve));
    await rm(profile,{recursive:true,force:true,maxRetries:10,retryDelay:100});
}
