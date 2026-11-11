// Import the already loaded build in a dedicated worker, keeping the loading
// indicator and input handling responsive during exact layout and PDF encoding.
const resources = performance.getEntriesByType('resource');
const moduleURL = resources.findLast(entry => /benchup-app-[^/]+\.js(?:\?|$)/.test(entry.name))?.name;
const wasmURL = resources.findLast(entry => entry.name.endsWith('_bg.wasm'))?.name;
if (!moduleURL || !wasmURL) throw new Error('Could not load the PDF generator. Please reload and try again.');
const code = `
const initialized = import(${JSON.stringify(moduleURL)}).then(async module => {
    await module.default({module_or_path: ${JSON.stringify(wasmURL)}});
    return module;
});
self.onmessage = async event => {
    try {
        const module = await initialized;
        const bytes = Uint8Array.from(module.benchup_generate_pdf(event.data));
        self.postMessage({bytes}, [bytes.buffer]);
    } catch (error) { self.postMessage({error: String(error)}); }
};`;
const url = URL.createObjectURL(new Blob([code], {type: 'text/javascript'}));
let worker;
try { worker = new Worker(url, {type: 'module'}); }
finally { URL.revokeObjectURL(url); }
return new Promise((resolve, reject) => {
    const finish = (error, bytes) => {
        clearTimeout(timer);
        worker.terminate();
        if (error) reject(new Error(error)); else resolve(bytes);
    };
    const timer = setTimeout(() => finish('PDF generation timed out. Please try again.'), 120000);
    worker.onmessage = event => finish(event.data.error, event.data.bytes);
    worker.onerror = event => finish(event.message || 'Could not generate the PDF.');
    worker.onmessageerror = () => finish('Could not read the generated PDF.');
    try { worker.postMessage(source); }
    catch (error) { finish(String(error)); }
});
