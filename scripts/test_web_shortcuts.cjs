// Run with node --test scripts/test_web_shortcuts.cjs; no app build required.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const path = require('node:path');
const vm = require('node:vm');

const html = fs.readFileSync(path.join(__dirname, '../index.html'), 'utf8');
const source = html.match(/<script id="workspace-shortcuts">([\s\S]*?)<\/script>/)[1];
function createWorkspace({keyboardSupported = true, pointerSupported = true, legacyPointerLock = false, lock, pointerLock, requestFullscreen} = {}) {
    const canvasListeners = new Map();
    const documentListeners = new Map();
    const windowListeners = new Map();
    const calls = {lock: [], unlock: 0, fullscreen: 0, exitFullscreen: 0, dispatched: [], pointerLock: 0, exitPointerLock: 0, order: []};
    let handler;
    let observer;
    const listen = (map, type, callback) => {
        if (!map.has(type)) map.set(type, []);
        map.get(type).push(callback);
    };
    const canvas = {
        dataset: {walkActive: 'false', walkInputEnabled: 'true', walkPlaced: 'true'},
        clientWidth: 1000, clientHeight: 500,
        getBoundingClientRect: () => ({left: 0, top: 0, width: 1000, height: 500}),
        getAttribute: key => key === 'data-model-viewports' ? '[[0,0,1000,500]]' : null,
        focus() {},
        addEventListener(type, callback) { listen(canvasListeners, type, callback); },
        dispatchEvent(event) {
            calls.dispatched.push(event);
            event.target = canvas;
            if (event.type === 'keydown') handler(event);
        },
    };
    const workspace = {
        addEventListener(type, callback, options) {
            assert.equal(type, 'keydown');
            assert.equal(options.capture, true);
            assert.equal(options.passive, false);
            handler = callback;
        },
        async requestFullscreen() {
            calls.order.push('fullscreen');
            calls.fullscreen++;
            if (requestFullscreen) await requestFullscreen();
            document.fullscreenElement = workspace;
        },
    };
    const document = {
        fullscreenElement: null,
        pointerLockElement: null,
        getElementById: id => ({'benchup-workspace': workspace, 'benchup-canvas': canvas})[id],
        addEventListener(type, callback) { listen(documentListeners, type, callback); },
        async exitFullscreen() {
            calls.exitFullscreen++;
            document.fullscreenElement = null;
            for (const callback of documentListeners.get('fullscreenchange') || []) callback();
        },
        exitPointerLock() {
            calls.exitPointerLock++;
            document.pointerLockElement = null;
            for (const callback of documentListeners.get('pointerlockchange') || []) callback();
        },
    };
    if (pointerSupported) canvas.requestPointerLock = () => {
        calls.pointerLock++;
        calls.order.push('pointer');
        const result = Promise.resolve().then(async () => {
            if (pointerLock) await pointerLock();
            document.pointerLockElement = canvas;
            for (const callback of documentListeners.get('pointerlockchange') || []) callback();
        });
        if (legacyPointerLock) {
            result.catch(() => { for (const callback of documentListeners.get('pointerlockerror') || []) callback(); });
            return;
        }
        return result;
    };
    const navigator = {keyboard: keyboardSupported ? {
        async lock(codes) {
            assert(document.fullscreenElement, 'Keyboard capture requires fullscreen first');
            calls.lock.push(Array.from(codes));
            if (lock) await lock();
        },
        unlock() { calls.unlock++; },
    } : undefined};
    vm.runInNewContext(source, {
        document, navigator,
        window: {addEventListener(type, callback) { listen(windowListeners, type, callback); }},
        MutationObserver: class {
            constructor(callback) { observer = callback; }
            observe(_target, options) { assert.deepEqual(Array.from(options.attributeFilter), ['data-walk-active', 'data-walk-input-enabled', 'data-walk-placed']); }
        },
        KeyboardEvent: class {
            constructor(type, values) { this.type = type; Object.assign(this, values); }
            getModifierState() { return false; }
            preventDefault() { this.defaultPrevented = true; }
        },
    });
    return {
        canvas, workspace, document, calls,
        keydown: event => handler(event),
        setWalking(active) { canvas.dataset.walkActive = String(active); observer(); },
        setPlaced(placed) { canvas.dataset.walkPlaced = String(placed); observer(); },
        setInputEnabled(enabled) { canvas.dataset.walkInputEnabled = String(enabled); observer(); },
        mousemove(movementX, movementY) {
            for (const callback of documentListeners.get('mousemove') || []) callback({movementX, movementY});
        },
        pointerup(options = {}) {
            for (const callback of canvasListeners.get('pointerup') || []) {
                callback({button: 0, clientX: 200, clientY: 200, ...options});
            }
        },
        blur() { for (const callback of windowListeners.get('blur') || []) callback(); },
    };
}
const environment = createWorkspace();
const {canvas} = environment;
const textInput = {};

function cancelled(key, options = {}) {
    const event = {
        key, code: '', target: canvas, ctrlKey: false, metaKey: false,
        isComposing: false, getModifierState: () => false,
        preventDefault() { this.defaultPrevented = true; },
        // Cancelling browser defaults must not stop delivery to eframe.
        stopPropagation() { assert.fail('Shortcut did not reach eframe'); },
        stopImmediatePropagation() { assert.fail('Shortcut did not reach eframe'); },
        ...options,
    };
    environment.keydown(event);
    return !!event.defaultPrevented;
}

test('canvas and eframe sibling input share the capture listener', () => {
    assert.match(html, /<div id="benchup-workspace">\s*<canvas[^>]*id="benchup-canvas"[^>]*><\/canvas>\s*<\/div>/);
});

test('app commands cancel browser defaults on canvas and in text fields', () => {
    for (const target of [canvas, textInput]) {
        for (const modifier of ['ctrlKey', 'metaKey']) {
            for (const key of ['a', 'f', 'n', 'o', 's', 'y', 'z', 'A', 'F', 'N', 'O', 'S', 'Y', 'Z']) {
                for (const shiftKey of [false, true]) {
                    assert(cancelled(key, {target, [modifier]: true, shiftKey}), `${modifier}+${key}`);
                }
            }
        }
    }
    assert(cancelled('z', {ctrlKey: true, repeat: true}));
});

test('clipboard commands still generate native clipboard events', () => {
    for (const target of [canvas, textInput]) {
        for (const modifier of ['ctrlKey', 'metaKey']) {
            for (const key of ['c', 'x', 'v', 'C', 'X', 'V']) {
                for (const shiftKey of [false, true]) {
                    assert(!cancelled(key, {target, [modifier]: true, shiftKey}));
                }
            }
        }
    }
});

test('tools and navigation cancel defaults without breaking text entry', () => {
    for (const key of [' ', ...'ab cdefgilmopqrst'.replaceAll(' ', ''),
        'ArrowDown', 'ArrowLeft', 'ArrowRight', 'ArrowUp', 'Backspace',
        'Delete', 'Enter', 'Tab']) {
        assert(cancelled(key), key);
        assert(!cancelled(key, {target: textInput}), `text input: ${key}`);
    }
    for (const key of ['F1', 'Escape']) {
        assert(cancelled(key));
        assert(cancelled(key, {target: textInput}));
    }
});

test('unassigned browser shortcuts and ordinary typing remain available', () => {
    for (const modifier of ['ctrlKey', 'metaKey']) {
        for (const key of ['l', 'r', 't', 'w', '+', '-', '0']) {
            assert(!cancelled(key, {[modifier]: true}), `${modifier}+${key}`);
        }
    }
    for (const key of ['F5', 'F12', '1', '.', ',', 'u']) assert(!cancelled(key));
});

test('non-Latin layouts use the same physical fallback as eframe', () => {
    assert(cancelled('я', {code: 'KeyZ', ctrlKey: true}));
    assert(cancelled('д', {code: 'KeyL'}));
    assert(!cancelled('y', {code: 'KeyZ', ctrlKey: true, getModifierState: key => key === 'AltGraph'}));
});

test('IME composition is not intercepted', () => {
    assert(!cancelled('z', {ctrlKey: true, isComposing: true}));
    assert(!cancelled('Enter', {isComposing: true}));
});

const settle = async () => { for (let i = 0; i < 12; i++) await Promise.resolve(); };

test('Shift crouching and movement reach Walk without interfering with text input', () => {
    environment.setWalking(true);
    for (const key of ['Shift', 'W', 'A', 'S', 'D', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', 'Tab']) {
        assert(cancelled(key, {shiftKey: true}), key);
        assert(!cancelled(key, {shiftKey: true, target: textInput}), `text input: ${key}`);
    }
    environment.setWalking(false);
    assert(!cancelled('Shift', {shiftKey: true}));
});

test('Walk cancels Ctrl+W defaults and still delivers movement to eframe', () => {
    environment.setWalking(true);
    for (const key of ['w', 'W', 'a', 's', 'd', 'ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight', ' ']) {
        assert(cancelled(key, {ctrlKey: true}), key);
    }
    assert(!cancelled('w', {ctrlKey: true, target: textInput}));
    environment.setWalking(false);
    assert(!cancelled('w', {ctrlKey: true}));
});

test('Walk stays windowed and captures the pointer without requesting fullscreen', async () => {
    const app = createWorkspace();
    app.setWalking(true);
    app.pointerup();
    await settle();
    assert.equal(app.calls.fullscreen, 0);
    assert.equal(app.document.fullscreenElement, null);
    assert.equal(app.document.pointerLockElement, app.canvas);
    assert.deepEqual(app.calls.lock, []);
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'unavailable');
    app.pointerup();
    await settle();
    assert.equal(app.calls.lock.length, 0);
    app.setWalking(false);
    await settle();
    assert.equal(app.calls.unlock, 1);
    assert.equal(app.calls.exitFullscreen, 0);
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'idle');
});

test('capture only starts from an enabled model-pane click in Walk', async () => {
    const app = createWorkspace();
    app.pointerup();
    app.setWalking(true);
    app.pointerup({button: 2});
    app.pointerup({clientX: 1200});
    app.canvas.dataset.walkInputEnabled = 'false';
    app.pointerup();
    await settle();
    assert.equal(app.calls.fullscreen, 0);
    assert.equal(app.calls.lock.length, 0);
});

test('leaving existing fullscreen releases keyboard capture without reentering fullscreen', async () => {
    const app = createWorkspace();
    app.document.fullscreenElement = {};
    app.setWalking(true);
    app.pointerup();
    await settle();
    await app.document.exitFullscreen();
    assert.equal(app.canvas.dataset.walkExitRequested, undefined);
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'unavailable');
    assert.equal(app.calls.unlock, 1);
    assert.equal(app.calls.fullscreen, 0);
});

test('existing fullscreen is preserved when Walk is deactivated', async () => {
    const app = createWorkspace();
    const originalFullscreen = {};
    app.document.fullscreenElement = originalFullscreen;
    app.setWalking(true);
    app.pointerup();
    await settle();
    assert.deepEqual(app.calls.lock, [['KeyW', 'KeyA', 'KeyS', 'KeyD']]);
    app.setWalking(false);
    assert.equal(app.calls.fullscreen, 0);
    assert.equal(app.calls.exitFullscreen, 0);
    assert.equal(app.document.fullscreenElement, originalFullscreen);
});

test('unsupported or denied keyboard capture is reported instead of claiming protection', async () => {
    for (const options of [{keyboardSupported: false}, {lock: async () => { throw new Error('Denied'); }}]) {
        const app = createWorkspace(options);
        const fullscreen = {};
        app.document.fullscreenElement = fullscreen;
        app.setWalking(true);
        app.pointerup();
        await settle();
        assert.equal(app.canvas.dataset.walkKeyboardCapture, 'unavailable');
        app.setWalking(false);
        await settle();
        assert.equal(app.document.fullscreenElement, fullscreen);
        assert.equal(app.calls.fullscreen, 0);
        assert.equal(app.calls.exitFullscreen, 0);
    }
});

test('no Walk capture path calls fullscreen entry or exit APIs', async () => {
    assert.doesNotMatch(source, /\b(?:requestFullscreen|exitFullscreen)\s*\(/);
    const app = createWorkspace({requestFullscreen: () => { assert.fail('Walk requested fullscreen'); }});
    app.setWalking(true);
    app.pointerup();
    await settle();
    app.setWalking(false);
    await settle();
    assert.equal(app.document.fullscreenElement, null);
    assert.equal(app.calls.lock.length, 0);
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'idle');
});

test('losing focus releases protected keys and requests exit from Walk', async () => {
    const app = createWorkspace();
    app.setWalking(true);
    app.pointerup();
    await settle();
    app.blur();
    await settle();
    assert.equal(app.canvas.dataset.walkExitRequested, 'true');
    assert.equal(app.document.fullscreenElement, null);
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'idle');
});

test('windowed walking does not request fullscreen permission', async () => {
    const app = createWorkspace({requestFullscreen: async () => { throw new Error('Denied'); }});
    app.setWalking(true);
    app.pointerup();
    await settle();
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'unavailable');
    assert.equal(app.calls.lock.length, 0);
    assert.equal(app.calls.fullscreen, 0);
    assert.equal(app.canvas.dataset.walkPointerCapture, 'locked');
    assert.equal(app.document.fullscreenElement, null);
});

test('a keyboard permission result arriving after Walk exits is released', async () => {
    let resolve;
    const pending = new Promise(done => { resolve = done; });
    const app = createWorkspace({lock: () => pending});
    const fullscreen = {};
    app.document.fullscreenElement = fullscreen;
    app.setWalking(true);
    app.pointerup();
    await settle();
    assert.equal(app.calls.lock.length, 1);
    app.setWalking(false);
    resolve();
    await settle();
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'idle');
    assert.equal(app.document.fullscreenElement, fullscreen);
    assert(app.calls.unlock >= 1);
});

test('Walk captures the pointer in the current window from the placement gesture', async () => {
    const app = createWorkspace();
    app.setWalking(true);
    app.pointerup();
    await settle();
    assert.deepEqual(app.calls.order, ['pointer']);
    assert.equal(app.document.pointerLockElement, app.canvas);
    assert.equal(app.canvas.dataset.walkPointerCapture, 'locked');
    assert.equal(app.calls.pointerLock, 1);
    app.pointerup();
    await settle();
    assert.equal(app.calls.pointerLock, 1);
});

test('locked mouse look accumulates relative movement with no button held', async () => {
    const app = createWorkspace();
    app.setWalking(true);
    app.pointerup();
    await settle();
    app.mousemove(12, -4);
    app.mousemove(8, 3);
    assert.equal(app.canvas.dataset.walkLookX, '20');
    assert.equal(app.canvas.dataset.walkLookY, '-1');
    // Rust drains these counters once per frame; stationary absolute cursor
    // coordinates do not limit subsequent movement at the edge of the screen.
    app.canvas.dataset.walkLookX = '0';
    app.canvas.dataset.walkLookY = '0';
    app.mousemove(6, 2);
    app.mousemove(NaN, Infinity);
    assert.equal(app.canvas.dataset.walkLookX, '6');
    assert.equal(app.canvas.dataset.walkLookY, '2');
    app.setWalking(false);
    assert.equal(app.document.pointerLockElement, null);
    assert.equal(app.canvas.dataset.walkLookX, '0');
    assert.equal(app.canvas.dataset.walkLookY, '0');
    app.mousemove(100, 100);
    assert.equal(app.canvas.dataset.walkLookX, '0');
});

test('browser pointer unlock requests exiting Walk and restoring the original view', async () => {
    const app = createWorkspace();
    app.setWalking(true);
    app.pointerup();
    await settle();
    app.document.exitPointerLock();
    await settle();
    assert.equal(app.canvas.dataset.walkExitRequested, 'true');
    assert.equal(app.document.fullscreenElement, null);
    assert.equal(app.canvas.dataset.walkPointerCapture, 'idle');
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'idle');
    assert.deepEqual(app.calls.dispatched.map(event => [event.type, event.key]), [['keydown', 'Escape'], ['keyup', 'Escape']]);
});

test('dialogs release the pointer without exiting Walk and a click can recapture', async () => {
    const app = createWorkspace();
    app.setWalking(true);
    app.pointerup();
    await settle();
    app.mousemove(30, 10);
    app.setInputEnabled(false);
    assert.equal(app.document.pointerLockElement, null);
    assert.equal(app.canvas.dataset.walkPointerCapture, 'idle');
    assert.equal(app.canvas.dataset.walkExitRequested, undefined);
    assert.equal(app.canvas.dataset.walkKeyboardCapture, 'unavailable');
    assert.equal(app.canvas.dataset.walkLookX, '0');
    app.mousemove(100, 100);
    assert.equal(app.canvas.dataset.walkLookX, '0');
    app.setInputEnabled(true);
    app.pointerup();
    await settle();
    assert.equal(app.document.pointerLockElement, app.canvas);
    assert.equal(app.calls.pointerLock, 2);
    assert.equal(app.calls.fullscreen, 0);
    assert.equal(app.calls.lock.length, 0);
});

test('pointer capture supports older browsers that return void', async () => {
    const app = createWorkspace({legacyPointerLock: true});
    app.setWalking(true);
    app.pointerup();
    await settle();
    assert.equal(app.document.pointerLockElement, app.canvas);
    assert.equal(app.canvas.dataset.walkPointerCapture, 'locked');
    app.mousemove(7, -2);
    assert.equal(app.canvas.dataset.walkLookX, '7');
});

test('unsupported and denied pointer lock never force fullscreen', async () => {
    for (const options of [{pointerSupported: false},
        {pointerLock: async () => { throw new Error('Denied'); }},
        {legacyPointerLock: true, pointerLock: async () => { throw new Error('Denied'); }}]) {
        const app = createWorkspace(options);
        app.setWalking(true);
        app.pointerup();
        await settle();
        assert.equal(app.canvas.dataset.walkPointerCapture, 'unavailable');
        assert.equal(app.document.pointerLockElement, null);
        assert.equal(app.canvas.dataset.walkKeyboardCapture, 'unavailable');
        assert.equal(app.calls.fullscreen, 0);
        assert.equal(app.document.fullscreenElement, null);
        app.setWalking(false);
        assert.equal(app.canvas.dataset.walkPointerCapture, 'idle');
    }
});

test('invalid placement cancels a late pointer permission result', async () => {
    let resolve;
    const pending = new Promise(done => { resolve = done; });
    const app = createWorkspace({pointerLock: () => pending});
    app.setWalking(true);
    app.pointerup();
    app.setPlaced(false);
    resolve();
    await settle();
    assert.equal(app.document.pointerLockElement, null);
    assert.equal(app.canvas.dataset.walkPointerCapture, 'idle');
    assert.equal(app.canvas.dataset.walkExitRequested, undefined);
});
