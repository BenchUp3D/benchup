// Called synchronously from the app's save action to retain user activation.
// null means this browser needs the download fallback; a Promise means the
// picker owns the operation, including cancellation and write failures.
if (typeof window.showSaveFilePicker !== 'function') return null;
// Rust supplies compressed bytes asynchronously. Handle payload rejection even
// if the picker is cancelled or fails before the payload is needed.
const payload = Promise.resolve(contents).then(value => ({value}), error => ({error}));
return (async () => {
    let handle;
    try {
        handle = await window.showSaveFilePicker({
            id: 'benchup-project-save',
            suggestedName: name,
            types: [{
                description,
                accept: {'application/gzip': ['.bup', '.k3d', '.ketchup']},
            }],
        });
    } catch (error) {
        if (error.name === 'AbortError') return null;
        throw error;
    }
    const data = await payload;
    if ('error' in data) throw data.error;
    const writable = await handle.createWritable();
    try {
        await writable.write(data.value);
        await writable.close();
    } catch (error) {
        // Do not commit a partial file if writing or closing fails.
        try { await writable.abort(); } catch {}
        throw error;
    }
    return handle.name;
})();
