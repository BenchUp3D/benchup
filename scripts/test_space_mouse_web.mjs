import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

// Load the same ES module wasm-bindgen bundles, without requiring package.json
// or a browser just to exercise WebHID connection and report lifecycle rules.
const source = await readFile(process.env.BENCHUP_SPACE_MOUSE_SOURCE ??
    new URL('../crates/benchup-app/src/space_mouse_web.js', import.meta.url), 'utf8');
const { createSpaceMouse } = await import(`data:text/javascript;base64,${Buffer.from(source).toString('base64')}`);
const settle = () => new Promise(resolve => setImmediate(resolve));
function deferred() {
    let resolve, reject;
    const promise = new Promise((yes, no) => { resolve = yes; reject = no; });
    return { promise, resolve, reject };
}
function emit(target, type, values = {}) {
    target.dispatchEvent(Object.assign(new Event(type), values));
}

class Device extends EventTarget {
    constructor(properties = {}) {
        super();
        Object.assign(this, {
            vendorId: 0x256f, productId: 0xc635,
            productName: 'SpaceMouse Compact', collections: [{ usagePage: 1, usage: 8 }],
            opened: false, opens: 0, closes: 0,
        }, properties);
    }
    async open() {
        ++this.opens;
        if (this.openGate) await this.openGate.promise;
        if (this.openError) throw this.openError;
        this.opened = true;
    }
    async close() {
        ++this.closes;
        if (this.closeGate) await this.closeGate.promise;
        this.opened = false;
    }
    report(reportId, bytes, offset = 0) {
        const buffer = new Uint8Array(offset + bytes.length + 2);
        buffer.set(bytes, offset);
        emit(this, 'inputreport', { device: this, reportId, data: new DataView(buffer.buffer, offset, bytes.length) });
    }
}
class HID extends EventTarget {
    constructor() {
        super();
        this.granted = [];
        this.selection = [];
        this.scans = 0;
        this.requests = [];
    }
    async getDevices() {
        ++this.scans;
        if (this.scanError) throw this.scanError;
        return this.scanGate ? await this.scanGate.promise : this.granted;
    }
    requestDevice(options) {
        this.requests.push(options);
        if (this.requestError) return Promise.reject(this.requestError);
        if (this.requestGate) return this.requestGate.promise;
        this.granted = [...new Set([...this.granted, ...this.selection])];
        return Promise.resolve(this.selection);
    }
}
class Environment extends EventTarget {
    constructor(hid = new HID()) {
        super();
        this.isSecureContext = true;
        this.navigator = { hid };
        this.document = new EventTarget();
        this.document.hidden = false;
        this.document.focused = true;
        this.document.hasFocus = () => this.document.focused;
    }
    focus(focused) {
        this.document.focused = focused;
        emit(this, focused ? 'focus' : 'blur');
    }
    hidden(hidden) {
        this.document.hidden = hidden;
        emit(this.document, 'visibilitychange');
    }
}
function fixture(t, { hid = new HID(), env = new Environment(hid) } = {}) {
    const result = { hid, env, reports: [], resets: 0, wakes: 0, input: null };
    result.connection = createSpaceMouse(
        report => { result.reports.push([...report]); result.input = [...report]; },
        () => { ++result.resets; result.input = null; },
        () => { ++result.wakes; }, env,
    );
    t.after(() => result.connection.dispose());
    return result;
}
async function ready(t, devices = []) {
    const f = fixture(t);
    f.hid.granted = devices;
    f.connection.sync(true, true);
    await settle();
    return f;
}

test('unsupported browsers and insecure contexts do not request device access', t => {
    for (const secure of [true, false]) {
        const env = new Environment();
        env.isSecureContext = secure;
        if (secure) env.navigator = {};
        const f = fixture(t, { env });
        f.connection.sync(true, true);
        f.connection.connect();
        assert.equal(f.connection.supported, false);
        assert.equal(f.connection.canConnect, false);
        assert.equal(f.connection.status, secure ? "unsupported" : "insecure");
        assert.equal(f.env.navigator.hid?.requests.length ?? 0, 0);
    }
});

test('reload automatically opens only a previously authorized multi-axis SpaceMouse', async t => {
    const device = new Device({ vendorId: 0x046d, productId: 0xc626, productName: 'SpaceNavigator' });
    const unrelated = new Device({ productId: 1, collections: [{ usagePage: 1, usage: 2 }] });
    const logitechMouse = new Device({ vendorId: 0x046d, productId: 0xc52b });
    const f = await ready(t, [unrelated, logitechMouse, device]);
    assert.equal(f.connection.device, 'SpaceNavigator');
    assert.equal(device.opens, 1);
    assert.equal(unrelated.opens, 0);
    assert.equal(logitechMouse.opens, 0);
    assert.equal(f.hid.requests.length, 0);
    assert.equal(f.connection.canConnect, false);
    assert.equal(f.connection.status, "connected");
    assert.equal(f.connection.device, "SpaceNavigator");
    for (let i = 0; i < 5; ++i) f.connection.sync(true, true);
    await settle();
    assert.equal(device.opens, 1);
    assert.equal(f.hid.scans, 1);
});

test('Connect requests permission synchronously with narrow device filters', async t => {
    const f = await ready(t);
    const device = new Device();
    f.hid.selection = [device];
    assert.equal(f.hid.requests.length, 0);
    assert.equal(f.connection.canConnect, true);
    f.connection.connect();
    assert.equal(f.hid.requests.length, 1, 'requestDevice must run before yielding user activation');
    assert.deepEqual(f.hid.requests[0].filters, [
        { vendorId: 0x256f, usagePage: 1, usage: 8 },
        { vendorId: 0x046d, usagePage: 1, usage: 8 },
    ]);
    f.connection.connect();
    assert.equal(f.hid.requests.length, 1, 'do not open a second permission prompt');
    await settle();
    assert.equal(f.connection.device, device.productName);
    assert.equal(device.opens, 1);
    f.connection.dispose();
    await settle();
    const reloaded = fixture(t, { hid: f.hid });
    reloaded.connection.sync(true, true);
    await settle();
    assert.equal(reloaded.connection.device, device.productName);
    assert.equal(f.hid.requests.length, 1, 'browser permission survives app reload');
});

test('split/combined motion and Fit reports retain the ID and DataView byte offset', async t => {
    const device = new Device();
    const f = await ready(t, [device]);
    const combined = [0x5e, 1, 0xa2, 0xfe, 0, 0, 0, 0, 0x5e, 1, 0, 0];
    device.report(1, combined, 5);
    device.report(2, [0, 0, 0, 0, 0x5e, 1], 3);
    device.report(3, [2, 0], 7);
    assert.deepEqual(f.reports, [
        [1, ...combined], [2, 0, 0, 0, 0, 0x5e, 1], [3, 2, 0],
    ]);
    device.report(1, [1, 2, 3, 4, 5, 6]);
    assert.deepEqual(f.reports.at(-1), [1, 1, 2, 3, 4, 5, 6]);
});

test('malformed reports are ignored and oversized report copies stay bounded', async t => {
    const device = new Device();
    const f = await ready(t, [device]);
    device.report(9, [1, 2, 3, 4, 5, 6]);
    device.report(1, [1, 2, 3, 4, 5]);
    device.report(2, [1]);
    device.report(3, []);
    assert.equal(f.reports.length, 0);
    device.report(1, Array(1000).fill(7));
    device.report(2, Array(1000).fill(8));
    device.report(3, Array(1000).fill(9));
    assert.deepEqual(f.reports.map(r => r.length), [13, 7, 5]);
});

test('blur/hidden tabs clear motion and require fresh reports after focus returns', async t => {
    const device = new Device();
    const f = await ready(t, [device]);
    device.report(1, [1, 2, 3, 4, 5, 6]);
    device.report(3, [2]);
    assert.ok(f.input);
    const resets = f.resets;
    f.env.focus(false);
    assert.equal(f.resets, resets + 1);
    assert.equal(f.input, null);
    device.report(1, [1, 2, 3, 4, 5, 6]);
    assert.equal(f.input, null);
    f.env.focus(true);
    assert.equal(f.input, null);
    device.report(1, [1, 2, 3, 4, 5, 6]);
    assert.ok(f.input);
    f.env.hidden(true);
    assert.equal(f.input, null);
    device.report(3, [2]);
    assert.equal(f.input, null);
    f.env.hidden(false);
    assert.equal(f.input, null);
    f.connection.sync(true, false);
    device.report(1, [1, 2, 3, 4, 5, 6]);
    assert.equal(f.input, null, 'egui focus also gates reports');
});

test('unplug clears input and reconnect reopens without another permission prompt', async t => {
    const device = new Device();
    const f = await ready(t, [device]);
    device.report(1, [1, 2, 3, 4, 5, 6]);
    emit(f.hid, 'disconnect', { device });
    assert.equal(f.input, null);
    assert.equal(f.connection.device, undefined);
    assert.equal(f.connection.status, "disconnected");
    device.report(3, [2]);
    assert.equal(f.input, null);
    await settle();
    emit(f.hid, 'connect', { device });
    await settle();
    assert.equal(f.connection.device, device.productName);
    assert.equal(device.opens, 2);
    assert.equal(f.hid.requests.length, 0);
});

test('disabling closes the device and re-enabling restores authorized access', async t => {
    const device = new Device();
    const f = await ready(t, [device]);
    device.report(1, [1, 2, 3, 4, 5, 6]);
    f.connection.sync(false, true);
    assert.equal(f.connection.status, 'disabled');
    assert.equal(f.connection.device, undefined);
    assert.equal(f.input, null);
    device.report(3, [2]);
    assert.equal(f.input, null);
    await settle();
    assert.equal(device.opened, false);
    f.connection.sync(true, true);
    await settle();
    assert.equal(device.opened, true);
    assert.equal(f.hid.requests.length, 0);
});

test('permission denial and cancellation are recoverable without automatic prompts', async t => {
    const f = await ready(t);
    f.hid.requestError = new DOMException('blocked', 'NotAllowedError');
    f.connection.connect();
    await settle();
    assert.equal(f.connection.status, "denied");
    assert.equal(f.connection.canConnect, true);
    assert.equal(f.hid.requests.length, 1);
    f.hid.requestError = null;
    f.connection.connect();
    await settle();
    assert.equal(f.connection.status, "no_selection");
    assert.equal(f.connection.canConnect, true);
    const device = new Device();
    f.hid.selection = [device];
    f.connection.connect();
    await settle();
    assert.equal(f.connection.device, device.productName);
});

test('policy rejection of getDevices is shown and does not repeatedly retry', async t => {
    const f = fixture(t);
    f.hid.scanError = new DOMException('policy', 'SecurityError');
    f.connection.sync(true, true);
    await settle();
    assert.equal(f.connection.status, "denied");
    assert.equal(f.connection.canConnect, true);
    for (let i = 0; i < 5; ++i) f.connection.sync(true, true);
    assert.equal(f.hid.scans, 1);
    assert.equal(f.hid.requests.length, 0);
});

test('exclusive-access failures permit retry and another granted device can open', async t => {
    const first = new Device({ openError: new DOMException('busy', 'NetworkError') });
    const second = new Device({ productName: 'SpaceMouse Pro' });
    const f = await ready(t, [first]);
    assert.equal(f.connection.status, "open_failed");
    assert.equal(f.connection.canConnect, true);
    // Chrome reports an OS-level open failure as NotAllowedError.
    first.openError = new DOMException('Failed to open the device.', 'NotAllowedError');
    f.hid.selection = [first];
    f.connection.connect();
    await settle();
    assert.equal(f.connection.status, "open_failed");
    assert.equal(f.connection.canConnect, true);
    f.hid.selection = [first, second];
    f.connection.connect();
    await settle();
    assert.equal(f.connection.device, 'SpaceMouse Pro');
    assert.equal(second.opens, 1);
});

test('turning off during authorization ignores the late selection', async t => {
    const f = await ready(t);
    const device = new Device();
    f.hid.requestGate = deferred();
    f.connection.connect();
    f.connection.sync(false, true);
    f.hid.requestGate.resolve([device]);
    await settle();
    assert.equal(device.opens, 0);
    assert.equal(f.connection.device, undefined);
    assert.equal(f.connection.status, 'disabled');
});

test('turning off while a scan is pending never opens its result', async t => {
    const f = fixture(t);
    const device = new Device();
    f.hid.scanGate = deferred();
    f.connection.sync(true, true);
    f.connection.sync(false, true);
    f.hid.scanGate.resolve([device]);
    await settle();
    assert.equal(device.opens, 0);
    assert.equal(f.connection.status, 'disabled');
});

test('an in-flight open is closed before re-enabling can open the same device again', async t => {
    const device = new Device({ openGate: deferred(), closeGate: deferred() });
    const f = await ready(t, [device]);
    assert.equal(device.opens, 1);
    f.connection.sync(false, true);
    f.connection.sync(true, true);
    await settle();
    device.openGate.resolve();
    await settle();
    assert.equal(device.closes, 1);
    assert.equal(device.opens, 1, 'the second open waits for the obsolete handle to close');
    device.closeGate.resolve();
    await settle();
    assert.equal(device.opens, 2);
    assert.equal(device.closes, 1);
    assert.equal(f.connection.device, device.productName);
});

test('disconnect while opening invalidates the pending connection', async t => {
    const device = new Device({ openGate: deferred() });
    const f = await ready(t, [device]);
    emit(f.hid, 'disconnect', { device });
    device.openGate.resolve();
    await settle();
    assert.equal(device.opened, false);
    assert.equal(f.connection.device, undefined);
    assert.equal(f.connection.status, "disconnected");
});

test('dispose detaches listeners and pending operations never call dropped Rust closures', async t => {
    for (const pending of ['open', 'prompt']) {
        const device = new Device(pending === 'open' ? { openGate: deferred() } : {});
        const f = await ready(t, pending === 'open' ? [device] : []);
        if (pending === 'prompt') {
            f.hid.requestGate = deferred();
            f.connection.connect();
        }
        f.connection.dispose();
        const counts = [f.reports.length, f.resets, f.wakes];
        if (pending === 'open') device.openGate.resolve();
        else f.hid.requestGate.resolve([device]);
        await settle();
        emit(f.hid, 'connect', { device });
        emit(f.hid, 'disconnect', { device });
        f.env.focus(false);
        f.env.hidden(true);
        device.report(3, [2]);
        assert.deepEqual([f.reports.length, f.resets, f.wakes], counts);
        assert.equal(device.opened, false);
    }
});
