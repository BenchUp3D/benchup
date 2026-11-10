// WebHID only exposes devices the user has granted this origin permission to
// use. Keep report decoding and camera motion in the shared Rust input path.
const FILTERS = [
    { vendorId: 0x256f, usagePage: 1, usage: 8 },
    { vendorId: 0x046d, usagePage: 1, usage: 8 },
];

function isSpaceMouse(device) {
    const vendor = device.vendorId === 0x256f ||
        (device.vendorId === 0x046d && device.productId >= 0xc603 && device.productId <= 0xc640);
    return vendor && device.collections?.some(c => c.usagePage === 1 && c.usage === 8);
}

class SpaceMouseConnection {
    constructor(onReport, onReset, wake, env) {
        this.env = env;
        this.onReport = onReport;
        this.onReset = onReset;
        this.wake = wake;
        this.hid = env.isSecureContext ? env.navigator?.hid : null;
        this.supported = !!this.hid?.getDevices && !!this.hid?.requestDevice;
        this.enabled = false;
        this.focused = false;
        this.busy = false;
        this.active = null;
        this.opening = null;
        this.generation = 0;
        this.disposed = false;
        // Serialize opens and closes, including when disable/re-enable races
        // with a permission prompt or a device's asynchronous open().
        this.io = Promise.resolve();
        this.status = this.supported ? "Disabled" : this.unsupportedMessage();
        this.input = event => this.inputReport(event);
        this.connected = event => {
            if (this.enabled && isSpaceMouse(event.device)) void this.scan();
        };
        this.disconnected = event => {
            if (event.device !== this.active && event.device !== this.opening) return;
            ++this.generation;
            this.busy = false;
            this.detach();
            this.publish("3D Mouse disconnected. Reconnect it or select Connect 3D Mouse.");
        };
        this.focusChanged = () => this.setFocused(this.pageFocused());
        if (this.supported) {
            this.hid.addEventListener("connect", this.connected);
            this.hid.addEventListener("disconnect", this.disconnected);
            env.addEventListener("blur", this.focusChanged);
            env.addEventListener("focus", this.focusChanged);
            env.document.addEventListener("visibilitychange", this.focusChanged);
        }
    }

    unsupportedMessage() {
        return this.env.isSecureContext
            ? "This browser does not support 3D Mouse access. Use Chrome or Edge with WebHID support."
            : "Open BenchUp over HTTPS or localhost to connect a 3D Mouse.";
    }

    get device() {
        return this.active ? this.active.productName || "3Dconnexion SpaceMouse" : undefined;
    }

    get canConnect() {
        return this.supported && this.enabled && !this.busy && !this.active && !this.disposed;
    }

    pageFocused() {
        return !this.env.document.hidden && this.env.document.hasFocus();
    }

    setFocused(focused) {
        if (this.focused && !focused) {
            this.onReset();
            this.wake();
        }
        this.focused = focused;
    }

    publish(status) {
        if (this.disposed) return;
        this.status = status;
        this.wake();
    }

    current(generation) {
        return !this.disposed && this.enabled && generation === this.generation;
    }

    queue(action) {
        const result = this.io.then(action);
        this.io = result.catch(() => {});
        return result;
    }

    async close(device) {
        try {
            if (device.opened) await device.close();
        } catch {
            // Unplugged devices can reject close(). Input is already detached.
        }
    }

    detach() {
        const device = this.active;
        this.active = null;
        if (device) {
            device.removeEventListener("inputreport", this.input);
            void this.queue(() => this.close(device));
        }
        this.onReset();
        this.wake();
    }

    sync(enabled, focused) {
        if (this.disposed) return;
        this.setFocused(focused && this.pageFocused());
        if (this.enabled === enabled) return;
        this.enabled = enabled;
        ++this.generation;
        this.busy = false;
        if (!enabled) {
            this.detach();
            this.publish("Disabled");
        } else if (!this.supported) {
            this.publish(this.unsupportedMessage());
        } else {
            void this.scan();
        }
    }

    async scan() {
        if (!this.supported || !this.enabled || this.active || this.busy || this.disposed) return;
        const generation = ++this.generation;
        this.busy = true;
        this.publish("Looking for a previously connected 3D Mouse...");
        try {
            const devices = await this.hid.getDevices();
            await this.attach(devices, generation, false);
        } catch (error) {
            this.failed(error, generation);
        }
    }

    connect() {
        if (!this.canConnect) return;
        const generation = ++this.generation;
        this.busy = true;
        this.publish("Choose a 3D Mouse in your browser.");
        try {
            // Invoke synchronously from the Connect button, before any await,
            // while the browser's transient user activation is still active.
            const selection = this.hid.requestDevice({ filters: FILTERS.map(f => ({ ...f })) });
            void Promise.resolve(selection)
                .then(devices => this.attach(devices, generation, true))
                .catch(error => this.failed(error, generation));
        } catch (error) {
            this.failed(error, generation);
        }
    }

    async attach(devices, generation, requested) {
        if (!this.current(generation)) return;
        let lastError;
        for (const device of devices.filter(isSpaceMouse)) {
            try {
                const opened = await this.queue(async () => {
                    if (!this.current(generation)) return false;
                    this.opening = device;
                    try {
                        if (!device.opened) await device.open();
                        if (!this.current(generation)) {
                            await this.close(device);
                            return false;
                        }
                        this.active = device;
                        device.addEventListener("inputreport", this.input);
                        return true;
                    } finally {
                        if (this.opening === device) this.opening = null;
                    }
                });
                if (!this.current(generation)) return;
                if (opened) {
                    this.busy = false;
                    this.publish(`Connected: ${this.device}`);
                    return;
                }
            } catch (error) {
                lastError = error;
            }
        }
        if (!this.current(generation)) return;
        if (lastError) {
            this.failed(lastError, generation);
        } else {
            this.busy = false;
            this.publish(requested
                ? "No 3D Mouse selected. Select Connect 3D Mouse to try again."
                : "No 3D Mouse connected. Select Connect 3D Mouse.");
        }
    }

    failed(error, generation) {
        if (!this.current(generation)) return;
        this.busy = false;
        const denied = error?.name === "SecurityError" || error?.name === "NotAllowedError";
        this.publish(denied
            ? "3D Mouse access was not allowed. Check this site's device permissions and try again."
            : "Could not connect the 3D Mouse. Close other apps using it, check device permissions, and try again.");
    }

    inputReport(event) {
        if (!this.enabled || !this.focused || !this.pageFocused() || event.device !== this.active) return;
        const { reportId, data } = event;
        // WebHID's DataView excludes the report ID. Preserve its byte offset,
        // and bound the copy to the report sizes the Rust decoder understands.
        let length;
        if (reportId === 1 && data.byteLength >= 6) length = data.byteLength >= 12 ? 12 : 6;
        else if (reportId === 2 && data.byteLength >= 6) length = 6;
        else if (reportId === 3 && data.byteLength >= 1) length = Math.min(data.byteLength, 4);
        else return;
        const report = new Uint8Array(length + 1);
        report[0] = reportId;
        report.set(new Uint8Array(data.buffer, data.byteOffset, length), 1);
        this.onReport(report);
    }

    dispose() {
        if (this.disposed) return;
        this.enabled = false;
        ++this.generation;
        this.detach();
        this.disposed = true;
        this.busy = false;
        if (this.supported) {
            this.hid.removeEventListener("connect", this.connected);
            this.hid.removeEventListener("disconnect", this.disconnected);
            this.env.removeEventListener("blur", this.focusChanged);
            this.env.removeEventListener("focus", this.focusChanged);
            this.env.document.removeEventListener("visibilitychange", this.focusChanged);
        }
        // Pending opens/prompts may outlive the Rust owner. Never call a
        // wasm-bindgen Closure after Rust has dropped it.
        this.onReport = this.onReset = this.wake = () => {};
    }
}

export function createSpaceMouse(onReport, onReset, wake, env = globalThis) {
    return new SpaceMouseConnection(onReport, onReset, wake, env);
}
