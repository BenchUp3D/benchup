// Browser recovery uses IndexedDB so embedded reference images do not consume
// localStorage's small shared quota. Preferences remain in localStorage.
const DATABASE = 'benchup-workspace';
const STORE = 'recovery';
const KEYS = ['benchup.document.v1', 'benchup.edits.v1', 'benchup.selection.v1'];
const MIGRATED = 'benchup.storage-migrated.v1';
let database;
let ready = false;
let localFallback = false;
let values = new Map();
let pending = 0;
let revision = 0;
let failure;
let changed = false;

function message(error) {
    return `Could not save workspace: ${error?.message || String(error)}`;
}
function openDatabase() {
    return new Promise((resolve, reject) => {
        const request = indexedDB.open(DATABASE, 1);
        request.onupgradeneeded = () => request.result.createObjectStore(STORE);
        request.onsuccess = () => resolve(request.result);
        request.onerror = () => reject(request.error);
        request.onblocked = () => reject(new Error('Browser workspace storage is busy in another tab.'));
    });
}
function transaction(mode) {
    // Older browsers do not accept durability options.
    try { return database.transaction(STORE, mode, {durability: 'strict'}); }
    catch (error) {
        if (error instanceof TypeError) return database.transaction(STORE, mode);
        throw error;
    }
}
function completed(tx) {
    return new Promise((resolve, reject) => {
        tx.oncomplete = resolve;
        tx.onabort = () => reject(tx.error || new Error('Workspace save was cancelled.'));
        tx.onerror = () => {}; // Report the final transaction outcome, not an individual request.
    });
}
async function readDatabase() {
    const tx = transaction('readonly');
    const done = completed(tx);
    const result = new Map();
    for (const key of KEYS) {
        const request = tx.objectStore(STORE).get(key);
        request.onsuccess = () => {
            if (typeof request.result === 'string') result.set(key, request.result);
        };
    }
    await done;
    return result;
}
function readLegacy() {
    const result = new Map();
    const migrated = localStorage.getItem(MIGRATED) !== null;
    for (const key of KEYS) {
        const value = localStorage.getItem(key) ??
            (migrated ? null : localStorage.getItem(key.replace('benchup.', 'ketchup.')));
        if (value !== null) result.set(key, value);
    }
    return result;
}
export async function loadRecovery() {
    if (ready) return;
    try {
        database = await openDatabase();
        database.onversionchange = () => { database.close(); database = undefined; };
    } catch (_) {
        // Preserve the previous behavior when IndexedDB is unavailable.
        values = readLegacy();
        localFallback = true;
        ready = true;
        return;
    }
    values = await readDatabase();
    if (!values.has(KEYS[0])) {
        const legacy = readLegacy();
        if (legacy.has(KEYS[0])) {
            const tx = transaction('readwrite');
            const done = completed(tx);
            for (const [key, value] of legacy) tx.objectStore(STORE).put(value, key);
            await done;
            values = legacy;
        }
    }
    // Only remove the old snapshot after the entire migration commits.
    // Leave preferences and unrelated browser data alone.
    try {
        for (const key of KEYS) {
            if (values.has(key) && localStorage.getItem(key) === values.get(key)) localStorage.removeItem(key);
        }
        localStorage.setItem(MIGRATED, 'true');
    } catch (_) { /* IndexedDB remains usable if localStorage is blocked. */ }
    ready = true;
}
export function readRecovery(key) {
    if (!ready) throw new Error('Browser workspace storage is not ready.');
    return values.get(key);
}
export function writeRecovery(key, value, snapshot) {
    const current = ++revision;
    try { submitRecovery(key, value, snapshot, current); }
    catch (error) {
        failure = {revision: current, error: message(error)};
        changed = true;
        throw error;
    }
}
function submitRecovery(key, value, snapshot, current) {
    if (!ready) throw new Error('Browser workspace storage is not ready.');
    if (!KEYS.includes(key)) throw new Error('Unknown workspace storage key.');
    if (localFallback) {
        localStorage.setItem(key, value);
        if (snapshot) localStorage.removeItem(KEYS[1]);
        values.set(key, value);
        if (snapshot) {
            values.delete(KEYS[1]);
            if (failure && current > failure.revision) failure = undefined;
        }
        return;
    }
    // Create transactions immediately, rather than through a promise queue.
    // IndexedDB orders them, including rapid edits submitted before a repaint.
    const tx = transaction('readwrite');
    const store = tx.objectStore(STORE);
    try {
        store.put(value, key);
        if (snapshot) store.delete(KEYS[1]);
    } catch (error) { tx.abort(); throw error; }
    ++pending;
    tx.oncomplete = () => {
        values.set(key, value);
        if (snapshot) values.delete(KEYS[1]);
        if (snapshot && failure && current > failure.revision) failure = undefined;
        --pending;
        changed = true;
    };
    tx.onabort = () => {
        if (!failure || current > failure.revision) {
            failure = {revision: current, error: message(tx.error || 'Workspace save was cancelled.')};
        }
        --pending;
        changed = true;
    };
    tx.onerror = () => {};
    // Submit the commit before returning to a potentially long viewport frame.
    if (typeof tx.commit === 'function') tx.commit();
}
export function recoveryPending() {
    return pending > 0;
}
export function takeRecoveryResult() {
    if (!changed || pending > 0) return undefined;
    changed = false;
    return failure?.error ?? '';
}
