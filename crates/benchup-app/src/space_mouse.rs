//! 3Dconnexion SpaceMouse navigation in object mode: pushing the cap moves the
//! model. Desktop builds read the device on a background thread: through the
//! 3DxWare driver on macOS and spacenavd on Linux when they are running, and
//! over USB HID otherwise. Browser builds use WebHID with an explicit device
//! permission prompt, then reconnect to previously granted devices.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]
use super::*;
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
#[path = "space_mouse_web.rs"]
mod browser;

/// Fit button on SpaceMouse Compact, Wireless, and Pro button reports.
const FIT_BUTTON: u32 = 1 << 1;
/// Typical full deflection of the cap in raw HID counts.
const FULL_SCALE: f64 = 350.;
const DEAD_ZONE: f64 = 0.04;
/// Full deflection rates per second, in the units of Camera's mouse helpers.
const ORBIT_RATE: f64 = 450.;
const PAN_RATE: f64 = 1.;
const ZOOM_RATE: f64 = 750.;
/// Seconds after the cap comes to rest during which wheel input is still
/// treated as the operating system's scroll emulation of the cap.
const WHEEL_GRACE: f64 = 0.3;

#[derive(Clone, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct Settings {
    pub enabled: bool,
    pub orbit_speed: f64,
    pub pan_speed: f64,
    pub zoom_speed: f64,
    pub invert_orbit: bool,
    pub invert_pan: bool,
    pub invert_zoom: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            enabled: true,
            orbit_speed: 1.,
            pan_speed: 1.,
            zoom_speed: 1.,
            invert_orbit: false,
            invert_pan: false,
            invert_zoom: false,
        }
    }
}

impl Settings {
    pub(super) fn normalize(&mut self) {
        for speed in [&mut self.orbit_speed, &mut self.pan_speed, &mut self.zoom_speed] {
            *speed = if speed.is_finite() { speed.clamp(0.1, 4.) } else { 1. };
        }
    }
}

/// Cap deflection in device axes (x right, y toward the user, z down),
/// normalized to -1..=1 with the dead zone removed.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct Axes {
    pub translation: DVec3,
    pub rotation: DVec3,
}

impl Axes {
    pub fn idle(&self) -> bool {
        self.translation == DVec3::ZERO && self.rotation == DVec3::ZERO
    }
}

#[derive(Default)]
struct Shared {
    axes: Axes,
    buttons: u32,
    /// Buttons pressed since the UI last read them, so short taps are not lost.
    pressed: u32,
    device: Option<String>,
}

impl Shared {
    #[cfg(any(target_arch = "wasm32", test))]
    fn clear_input(&mut self) {
        self.axes = Axes::default();
        self.buttons = 0;
        self.pressed = 0;
    }

    fn report(&mut self, report: &[u8]) -> bool {
        let Some((&id, data)) = report.split_first() else {
            return false;
        };
        match id {
            1 if data.len() >= 6 => {
                self.axes.translation = axis_triplet(&data[..6]);
                // Newer devices send translation and rotation in one report.
                if data.len() >= 12 {
                    self.axes.rotation = axis_triplet(&data[6..12]);
                }
            }
            2 if data.len() >= 6 => self.axes.rotation = axis_triplet(&data[..6]),
            3 => {
                let buttons = data
                    .iter()
                    .take(4)
                    .enumerate()
                    .fold(0, |bits, (i, &byte)| bits | u32::from(byte) << (i * 8));
                self.pressed |= buttons & !self.buttons;
                self.buttons = buttons;
            }
            _ => return false,
        }
        true
    }
}

impl Shared {
    /// Apply a 3DxWare device state message: axes are x, y, z, rx, ry, rz in
    /// the same orientation as HID reports.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    fn driver_state(&mut self, command: u16, axis: [i16; 6], buttons: u32) -> bool {
        match command {
            DRIVER_AXIS => {
                let triplet = |i: usize| DVec3::new(
                    normalize_axis(axis[i]),
                    normalize_axis(axis[i + 1]),
                    normalize_axis(axis[i + 2]),
                );
                self.axes = Axes { translation: triplet(0), rotation: triplet(3) };
            }
            DRIVER_BUTTONS => {
                self.pressed |= buttons & !self.buttons;
                self.buttons = buttons;
            }
            _ => return false,
        }
        true
    }
}

impl Shared {
    /// Apply a spacenavd protocol 0 event: a type (0 motion, 1 button press,
    /// 2 button release) followed by x, y, z, rx, ry, rz or a button number.
    /// spacenavd axes are x right, y up, z away from the user.
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    fn spnav_event(&mut self, event: [i32; 8]) -> bool {
        let button = |number: i32| {
            u32::try_from(number).ok().and_then(|number| 1u32.checked_shl(number)).unwrap_or(0)
        };
        match event[0] {
            0 => {
                let limit = i32::from(i16::MAX);
                let axis = |i: usize| normalize_axis(event[i].clamp(-limit, limit) as i16);
                self.axes = Axes {
                    translation: DVec3::new(axis(1), -axis(3), -axis(2)),
                    rotation: DVec3::new(axis(4), -axis(6), -axis(5)),
                };
            }
            1 => {
                let bit = button(event[1]);
                self.pressed |= bit & !self.buttons;
                self.buttons |= bit;
            }
            2 => self.buttons &= !button(event[1]),
            _ => return false,
        }
        true
    }
}

fn is_3dconnexion(vendor: u32, product: u32) -> bool {
    match vendor {
        0x256f => true,
        // Logitech-era 3Dconnexion devices; the rest of this vendor is mice and keyboards.
        0x046d => (0xc603..=0xc640).contains(&product),
        _ => false,
    }
}

/// The 3Dconnexion device listed in the text of `/proc/bus/input/devices`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn input_device_name(devices: &str) -> Option<&'static str> {
    devices.lines().filter(|line| line.starts_with("I:")).find_map(|line| {
        let field = |key: &str| {
            line.split_whitespace()
                .find_map(|field| field.strip_prefix(key))
                .and_then(|value| u32::from_str_radix(value, 16).ok())
        };
        let (vendor, product) = (field("Vendor=")?, field("Product=")?);
        is_3dconnexion(vendor, product).then(|| product_name(product))
    })
}

/// kConnexionCmdHandleButtons and kConnexionCmdHandleAxis.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const DRIVER_BUTTONS: u16 = 2;
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
const DRIVER_AXIS: u16 = 3;

fn axis_triplet(data: &[u8]) -> DVec3 {
    let axis = |i: usize| normalize_axis(i16::from_le_bytes([data[i * 2], data[i * 2 + 1]]));
    DVec3::new(axis(0), axis(1), axis(2))
}

fn normalize_axis(raw: i16) -> f64 {
    let value = (f64::from(raw) / FULL_SCALE).clamp(-1., 1.);
    if value.abs() <= DEAD_ZONE {
        0.
    } else {
        // Rescale past the dead zone and square it for fine control near rest.
        let value = (value.abs() - DEAD_ZONE) / (1. - DEAD_ZONE);
        value * value * f64::from(raw).signum()
    }
}

/// Move the camera for `dt` seconds of cap deflection. Roll is ignored so the
/// horizon stays level; tilt and twist orbit around `pivot`.
pub(super) fn apply(
    camera: &mut Camera,
    axes: Axes,
    dt: f64,
    settings: &Settings,
    pivot: DVec3,
    viewport: DVec2,
) {
    let sign = |invert: bool| if invert { -1. } else { 1. };
    let Axes { translation: t, rotation: r } = axes;
    let orbit = DVec2::new(-r.z, r.x)
        * (ORBIT_RATE * settings.orbit_speed * sign(settings.invert_orbit) * dt);
    if orbit != DVec2::ZERO {
        camera.orbit_around(orbit, pivot);
    }
    let pan = DVec2::new(t.x, t.z)
        * (viewport.y * PAN_RATE * settings.pan_speed * sign(settings.invert_pan) * dt);
    if pan != DVec2::ZERO {
        camera.pan(pan, viewport.y);
    }
    // Pulling the cap toward you brings the model closer.
    let zoom = t.y * ZOOM_RATE * settings.zoom_speed * sign(settings.invert_zoom) * dt;
    if zoom != 0. {
        camera.zoom(zoom, viewport * 0.5, viewport);
    }
}

#[derive(Default)]
pub(super) struct SpaceMouse {
    shared: Arc<Mutex<Shared>>,
    #[cfg(not(target_arch = "wasm32"))]
    worker: Option<worker::Worker>,
    #[cfg(target_arch = "wasm32")]
    browser: Option<browser::Browser>,
    /// Orbit pivot locked for one continuous push of the cap.
    pub pivot: Option<DVec3>,
    pub last_time: Option<f64>,
    /// Input time until which wheel and zoom events are discarded.
    wheel_quiet_until: f64,
}

impl SpaceMouse {
    /// Start or stop the device reader to match the setting.
    pub fn sync(&mut self, ctx: &egui::Context, enabled: bool) {
        // Tests inject axes and must not read a real device.
        #[cfg(not(target_arch = "wasm32"))]
        if !cfg!(test) {
            if enabled != self.worker.is_some() {
                self.worker =
                    enabled.then(|| worker::Worker::start(ctx.clone(), self.shared.clone()));
                if !enabled {
                    *self.lock() = Shared::default();
                }
            }
            if let Some(worker) = &mut self.worker {
                worker.focus(ctx.input(|i| i.focused));
            }
        }
        #[cfg(target_arch = "wasm32")]
        if !cfg!(test) {
            let shared = self.shared.clone();
            let browser = self
                .browser
                .get_or_insert_with(|| browser::Browser::new(ctx.clone(), shared));
            browser.sync(enabled, ctx.input(|i| i.focused));
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn connect_browser(&self) {
        if let Some(browser) = &self.browser {
            browser.connect();
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub fn browser_can_connect(&self) -> bool {
        self.browser
            .as_ref()
            .is_some_and(browser::Browser::can_connect)
    }

    #[cfg(target_arch = "wasm32")]
    pub fn browser_status(&self) -> String {
        self.browser.as_ref().map_or_else(
            || "Select Connect 3D Mouse to grant access to your device.".to_owned(),
            browser::Browser::status,
        )
    }

    /// Without 3DxWare, macOS also turns cap pushes into scroll events, which
    /// the viewport would apply as wheel zoom on top of the cap's own motion.
    /// Discard wheel and zoom input while the cap is deflected and briefly
    /// after it is released.
    pub fn filter_input(&mut self, raw: &mut egui::RawInput) {
        let Some(now) = raw.time else {
            return;
        };
        if !self.lock().axes.idle() {
            self.wheel_quiet_until = now + WHEEL_GRACE;
        }
        if now < self.wheel_quiet_until {
            raw.events.retain(|event| {
                !matches!(event, egui::Event::MouseWheel { .. } | egui::Event::Zoom(_))
            });
        }
    }

    /// The current deflection and the buttons pressed since the last call.
    pub fn take(&mut self) -> (Axes, u32) {
        let mut shared = self.lock();
        (shared.axes, std::mem::take(&mut shared.pressed))
    }

    pub fn device(&self) -> Option<String> {
        #[cfg(target_arch = "wasm32")]
        {
            self.browser.as_ref().and_then(browser::Browser::device)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.lock().device.clone()
        }
    }

    #[cfg(test)]
    pub fn inject(&self, axes: Axes, pressed: u32) {
        let mut shared = self.lock();
        shared.axes = axes;
        shared.pressed |= pressed;
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Shared> {
        self.shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

impl BenchUpApp {
    pub(super) fn space_mouse_navigation(&mut self, ctx: &egui::Context) {
        self.space_mouse.sync(ctx, self.general_settings.space_mouse.enabled);
        let (axes, pressed) = self.space_mouse.take();
        #[cfg(not(target_arch = "wasm32"))]
        let updating = self.updates.open;
        #[cfg(target_arch = "wasm32")]
        let updating = false;
        let blocked = updating
            || self.project.blocked()
            || self.screenshot.open
            || self.settings_open
            || self.help
            || self.about
            || self.release_notes.open
            || self.object_creation.is_some()
            || self.object_rename.is_some()
            || self.viewports.renaming.is_some()
            || self.text_editor.is_some()
            || self.tool == Tool::Walk
            || !self.general_settings.space_mouse.enabled;
        if blocked || axes.idle() {
            self.space_mouse.pivot = None;
            self.space_mouse.last_time = None;
        }
        if blocked {
            return;
        }
        if pressed & FIT_BUTTON != 0 {
            self.fit(ctx);
        }
        let viewport = dv(self.viewport.size());
        if axes.idle() || viewport.min_element() < 1. {
            return;
        }
        let now = ctx.input(|i| i.time);
        let dt = self
            .space_mouse
            .last_time
            .map_or(1. / 60., |last| (now - last).clamp(0., 0.1));
        self.space_mouse.last_time = Some(now);
        let pivot = match self.space_mouse.pivot {
            Some(pivot) => pivot,
            None => {
                let pivot = self.orbit_center();
                self.space_mouse.pivot = Some(pivot);
                pivot
            }
        };
        apply(
            &mut self.doc.camera,
            axes,
            dt,
            &self.general_settings.space_mouse,
            pivot,
            viewport,
        );
        self.hover = None;
        self.flip_hover = None;
        self.camera_changed(ctx);
        // A cap held still sends no reports; keep moving until it is released.
        ctx.request_repaint();
    }
}

#[cfg(not(target_arch = "wasm32"))]
mod worker {
    #[cfg(target_os = "macos")]
    use super::driver;
    use super::{Shared, egui};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    const SCAN_INTERVAL: Duration = Duration::from_secs(2);

    pub struct Worker {
        stop: Arc<AtomicBool>,
        #[cfg(target_os = "macos")]
        driver: Option<driver::Client>,
        /// spacenavd sends events to every client, focused or not.
        #[cfg(target_os = "linux")]
        focused: Arc<AtomicBool>,
    }

    impl Drop for Worker {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
        }
    }

    impl Worker {
        pub fn start(ctx: egui::Context, shared: Arc<Mutex<Shared>>) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            // The 3DxWare driver opens the device exclusively, so receive its
            // events when it is installed and read raw HID otherwise.
            #[cfg(target_os = "macos")]
            if let Some(driver) = driver::Client::start(ctx.clone(), shared.clone()) {
                return Self { stop, driver: Some(driver) };
            }
            let flag = stop.clone();
            #[cfg(target_os = "linux")]
            let focused = Arc::new(AtomicBool::new(true));
            #[cfg(target_os = "linux")]
            let source = Source { focused: focused.clone() };
            #[cfg(not(target_os = "linux"))]
            let source = Source {};
            let spawned = std::thread::Builder::new()
                .name("space-mouse".into())
                .spawn(move || run(&ctx, &shared, &flag, &source));
            if let Err(error) = spawned {
                eprintln!("Could not start 3D mouse support: {error}");
            }
            Self {
                stop,
                #[cfg(target_os = "macos")]
                driver: None,
                #[cfg(target_os = "linux")]
                focused,
            }
        }

        /// Only the focused window receives driver events, like other 3D apps.
        #[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(unused_variables))]
        pub fn focus(&mut self, focused: bool) {
            #[cfg(target_os = "linux")]
            self.focused.store(focused, Ordering::Relaxed);
            #[cfg(target_os = "macos")]
            if let Some(driver) = &mut self.driver {
                driver.focus(focused);
            }
        }
    }

    fn lock(shared: &Mutex<Shared>) -> std::sync::MutexGuard<'_, Shared> {
        shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn is_space_mouse(info: &hidapi::DeviceInfo) -> bool {
        // Multi-axis controller collection; some backends cannot report usages.
        super::is_3dconnexion(info.vendor_id().into(), info.product_id().into())
            && ((info.usage_page() == 0x01 && info.usage() == 0x08) || info.usage_page() == 0)
    }

    /// What the reader thread needs besides the shared state.
    struct Source {
        #[cfg(target_os = "linux")]
        focused: Arc<AtomicBool>,
    }

    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
    fn run(ctx: &egui::Context, shared: &Mutex<Shared>, stop: &AtomicBool, source: &Source) {
        let mut api = match hidapi::HidApi::new() {
            Ok(api) => Some(api),
            Err(error) => {
                eprintln!("3D mouse USB access is unavailable: {error}");
                // spacenavd can still deliver events on Linux.
                if !cfg!(target_os = "linux") {
                    return;
                }
                None
            }
        };
        while !stop.load(Ordering::Relaxed) {
            // spacenavd owns the device when it runs, and raw HID access
            // usually needs extra permissions on Linux, so prefer the daemon.
            #[cfg(target_os = "linux")]
            if spacenavd(ctx, shared, stop, &source.focused) {
                *lock(shared) = Shared::default();
                ctx.request_repaint();
            }
            let found = api.as_mut().and_then(|api| {
                api.refresh_devices().ok()?;
                let info = api.device_list().find(|info| is_space_mouse(info))?;
                let name = info.product_string().unwrap_or("3D mouse").to_owned();
                info.open_device(api).ok().map(|device| (device, name))
            });
            if let Some((device, name)) = found {
                lock(shared).device = Some(name);
                ctx.request_repaint();
                read(ctx, shared, stop, &device);
                // Never leave the camera moving after a disconnect.
                *lock(shared) = Shared::default();
                ctx.request_repaint();
            }
            let mut waited = Duration::ZERO;
            while waited < SCAN_INTERVAL && !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
                waited += Duration::from_millis(100);
            }
        }
    }

    /// Read events from a running spacenavd until it goes away. Returns false
    /// when the daemon is not running.
    #[cfg(target_os = "linux")]
    fn spacenavd(
        ctx: &egui::Context,
        shared: &Mutex<Shared>,
        stop: &AtomicBool,
        focused: &AtomicBool,
    ) -> bool {
        use std::io::{ErrorKind, Read};
        let path = std::env::var_os("SPNAV_SOCKET").unwrap_or_else(|| "/var/run/spnav.sock".into());
        let Ok(mut stream) = std::os::unix::net::UnixStream::connect(path) else {
            return false;
        };
        if stream.set_read_timeout(Some(Duration::from_millis(100))).is_err() {
            return false;
        }
        // Clients that send nothing stay on protocol 0: eight native ints per event.
        let mut event = [0u8; 32];
        let mut filled = 0;
        let mut scanned: Option<std::time::Instant> = None;
        while !stop.load(Ordering::Relaxed) {
            // The daemon stays up without a device, so report what is plugged in.
            if scanned.is_none_or(|time| time.elapsed() >= SCAN_INTERVAL) {
                scanned = Some(std::time::Instant::now());
                let devices = std::fs::read_to_string("/proc/bus/input/devices").unwrap_or_default();
                let device = super::input_device_name(&devices).map(str::to_owned);
                let mut shared = lock(shared);
                if shared.device != device {
                    if device.is_none() {
                        *shared = Shared::default();
                    }
                    shared.device = device;
                    ctx.request_repaint();
                }
            }
            match stream.read(&mut event[filled..]) {
                Ok(0) => break,
                Ok(len) => {
                    filled += len;
                    if filled < event.len() {
                        continue;
                    }
                    filled = 0;
                    let mut values = [0i32; 8];
                    for (value, bytes) in values.iter_mut().zip(event.chunks_exact(4)) {
                        *value = i32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
                    }
                    let mut shared = lock(shared);
                    let changed = if focused.load(Ordering::Relaxed) {
                        shared.spnav_event(values)
                    } else {
                        // Another app is using the device; stay still.
                        let moving = !shared.axes.idle();
                        shared.axes = super::Axes::default();
                        shared.buttons = 0;
                        moving
                    };
                    if changed {
                        ctx.request_repaint();
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(_) => break,
            }
        }
        true
    }

    fn read(ctx: &egui::Context, shared: &Mutex<Shared>, stop: &AtomicBool, device: &hidapi::HidDevice) {
        let mut buffer = [0u8; 64];
        while !stop.load(Ordering::Relaxed) {
            match device.read_timeout(&mut buffer, 100) {
                Ok(0) => {}
                Ok(len) => {
                    if lock(shared).report(&buffer[..len]) {
                        ctx.request_repaint();
                    }
                }
                Err(_) => return,
            }
        }
    }
}

/// 3DxWare client on macOS, loaded at runtime so BenchUp still starts
/// without the driver.
#[cfg(all(not(target_arch = "wasm32"), target_os = "macos"))]
mod driver {
    use super::{Shared, egui};
    use std::ffi::{c_char, c_int, c_void};
    use std::sync::{Arc, Mutex, OnceLock};

    const FRAMEWORK: &std::ffi::CStr =
        c"/Library/Frameworks/3DconnexionClient.framework/3DconnexionClient";
    /// kConnexionClientManual: BenchUp activates itself while focused.
    const MANUAL_CLIENT: u32 = 0x2B2B_2B2B;
    const MODE_TAKE_OVER: u16 = 1;
    const MASK_ALL: u32 = 0x3FFF;
    const MASK_ALL_BUTTONS: u32 = 0xFFFF_FFFF;
    const MSG_DEVICE_STATE: u32 = 0x3364_5352;
    const CTL_ACTIVATE: u32 = 0x3364_6163;
    const CTL_DEACTIVATE: u32 = 0x3364_6463;
    const RTLD_LAZY: c_int = 1;

    unsafe extern "C" {
        fn dlopen(path: *const c_char, mode: c_int) -> *mut c_void;
        fn dlsym(handle: *mut c_void, symbol: *const c_char) -> *mut c_void;
    }

    /// ConnexionDeviceState from ConnexionClient.h, which uses `#pragma pack(2)`.
    #[repr(C, packed(2))]
    struct DeviceState {
        version: u16,
        client: u16,
        command: u16,
        param: i16,
        value: i32,
        time: u64,
        report: [u8; 8],
        buttons8: u16,
        axis: [i16; 6],
        address: u16,
        buttons: u32,
    }

    // Field offsets must match the packed C layout.
    const _: () = assert!(std::mem::size_of::<DeviceState>() == 48);
    const _: () = assert!(std::mem::offset_of!(DeviceState, time) == 12);
    const _: () = assert!(std::mem::offset_of!(DeviceState, axis) == 30);
    const _: () = assert!(std::mem::offset_of!(DeviceState, buttons) == 44);

    type MessageHandler = extern "C" fn(u32, u32, *mut c_void);
    type DeviceHandler = extern "C" fn(u32);

    struct Api {
        set_handlers: extern "C" fn(MessageHandler, DeviceHandler, DeviceHandler, bool) -> i16,
        cleanup_handlers: extern "C" fn(),
        register: extern "C" fn(u32, *const u8, u16, u32) -> u16,
        unregister: extern "C" fn(u16),
        set_button_mask: extern "C" fn(u16, u32),
        control: extern "C" fn(u16, u32, i32, *mut i32) -> i16,
    }

    fn api() -> Option<&'static Api> {
        static API: OnceLock<Option<Api>> = OnceLock::new();
        API.get_or_init(|| unsafe {
            let library = dlopen(FRAMEWORK.as_ptr(), RTLD_LAZY);
            if library.is_null() {
                return None;
            }
            let symbol = |name: &std::ffi::CStr| {
                let pointer = dlsym(library, name.as_ptr());
                (!pointer.is_null()).then_some(pointer)
            };
            Some(Api {
                set_handlers: std::mem::transmute(symbol(c"SetConnexionHandlers")?),
                cleanup_handlers: std::mem::transmute(symbol(c"CleanupConnexionHandlers")?),
                register: std::mem::transmute(symbol(c"RegisterConnexionClient")?),
                unregister: std::mem::transmute(symbol(c"UnregisterConnexionClient")?),
                set_button_mask: std::mem::transmute(symbol(c"SetConnexionClientButtonMask")?),
                control: std::mem::transmute(symbol(c"ConnexionClientControl")?),
            })
        })
        .as_ref()
    }

    /// Driver callbacks carry no context, so they reach the app through this.
    static ACTIVE: Mutex<Option<(Arc<Mutex<Shared>>, egui::Context)>> = Mutex::new(None);

    fn with_active(update: impl FnOnce(&mut Shared) -> bool) {
        let active = ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some((shared, ctx)) = active.as_ref() {
            if update(&mut shared.lock().unwrap_or_else(|poisoned| poisoned.into_inner())) {
                ctx.request_repaint();
            }
        }
    }

    extern "C" fn message(_product: u32, kind: u32, argument: *mut c_void) {
        if kind != MSG_DEVICE_STATE || argument.is_null() {
            return;
        }
        // SAFETY: the driver passes a ConnexionDeviceState for this message,
        // at an address that is not necessarily aligned.
        let state = unsafe { std::ptr::read_unaligned(argument as *const DeviceState) };
        let (command, axis, buttons) = (state.command, state.axis, state.buttons);
        with_active(|shared| shared.driver_state(command, axis, buttons));
    }

    extern "C" fn added(product: u32) {
        with_active(|shared| {
            shared.device = Some(super::product_name(product).to_owned());
            true
        });
    }

    extern "C" fn removed(_product: u32) {
        with_active(|shared| {
            *shared = Shared::default();
            true
        });
    }

    pub struct Client {
        id: u16,
        focused: Option<bool>,
    }

    impl Client {
        pub fn start(ctx: egui::Context, shared: Arc<Mutex<Shared>>) -> Option<Self> {
            let api = api()?;
            *ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((shared, ctx));
            if (api.set_handlers)(message, added, removed, true) != 0 {
                *ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                return None;
            }
            let id = (api.register)(MANUAL_CLIENT, c"\x07BenchUp".as_ptr().cast(), MODE_TAKE_OVER, MASK_ALL);
            if id == 0 {
                (api.cleanup_handlers)();
                *ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
                return None;
            }
            (api.set_button_mask)(id, MASK_ALL_BUTTONS);
            Some(Self { id, focused: None })
        }

        pub fn focus(&mut self, focused: bool) {
            if self.focused == Some(focused) {
                return;
            }
            self.focused = Some(focused);
            if let Some(api) = api() {
                let mut result = 0;
                (api.control)(self.id, if focused { CTL_ACTIVATE } else { CTL_DEACTIVATE }, 0, &mut result);
            }
            if !focused {
                // Stop moving when another app takes over the device.
                with_active(|shared| {
                    shared.axes = super::Axes::default();
                    true
                });
            }
        }
    }

    impl Drop for Client {
        fn drop(&mut self) {
            if let Some(api) = api() {
                (api.unregister)(self.id);
                (api.cleanup_handlers)();
            }
            *ACTIVE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
        }
    }
}

fn product_name(product: u32) -> &'static str {
    match product {
        0xc626 => "SpaceNavigator",
        0xc627 => "SpaceExplorer",
        0xc628 => "SpaceNavigator for Notebooks",
        0xc62b => "SpaceMouse Pro",
        0xc62e | 0xc62f => "SpaceMouse Wireless",
        0xc631 | 0xc632 => "SpaceMouse Pro Wireless",
        0xc633 => "SpaceMouse Enterprise",
        0xc635 => "SpaceMouse Compact",
        0xc652 => "3Dconnexion Universal Receiver",
        _ => "3Dconnexion device",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(id: u8, values: &[i16]) -> Vec<u8> {
        std::iter::once(id)
            .chain(values.iter().flat_map(|value| value.to_le_bytes()))
            .collect()
    }

    #[test]
    fn parses_split_and_combined_axis_reports() {
        let mut shared = Shared::default();
        assert!(shared.report(&report(1, &[350, -350, 0])));
        assert!(shared.report(&report(2, &[0, 0, 700])));
        assert_eq!(shared.axes.translation, DVec3::new(1., -1., 0.));
        assert_eq!(shared.axes.rotation, DVec3::new(0., 0., 1.));

        assert!(shared.report(&report(1, &[0, 0, -350, 350, 0, 0])));
        assert_eq!(shared.axes.translation, DVec3::new(0., 0., -1.));
        assert_eq!(shared.axes.rotation, DVec3::new(1., 0., 0.));
        assert!(!shared.report(&[7, 1, 2]));
        assert!(!shared.report(&[]));
    }

    #[test]
    fn clearing_browser_input_discards_held_axes_and_latched_buttons() {
        let mut shared = Shared::default();
        assert!(shared.report(&report(1, &[350, -350, 0, 350, 0, 0])));
        assert!(shared.report(&[3, 0b10]));
        assert!(!shared.axes.idle());
        assert_eq!(shared.pressed, FIT_BUTTON);
        shared.clear_input();
        assert!(shared.axes.idle());
        assert_eq!(shared.buttons, 0);
        assert_eq!(shared.pressed, 0);
    }

    #[test]
    fn button_presses_are_latched_until_read() {
        let mut shared = Shared::default();
        assert!(shared.report(&[3, 0b10, 0]));
        assert!(shared.report(&[3, 0, 0]));
        assert_eq!(shared.buttons, 0);
        assert_eq!(shared.pressed, FIT_BUTTON);
    }

    #[test]
    fn driver_states_update_axes_and_latch_buttons() {
        let mut shared = Shared::default();
        assert!(shared.driver_state(DRIVER_AXIS, [350, 0, -350, 0, 0, 350], 0));
        assert_eq!(shared.axes.translation, DVec3::new(1., 0., -1.));
        assert_eq!(shared.axes.rotation, DVec3::Z);
        assert!(shared.driver_state(DRIVER_BUTTONS, [0; 6], FIT_BUTTON));
        assert!(shared.driver_state(DRIVER_BUTTONS, [0; 6], 0));
        assert_eq!(shared.pressed, FIT_BUTTON);
        assert!(!shared.driver_state(0, [350; 6], 0));
        assert_eq!(shared.axes.translation, DVec3::new(1., 0., -1.));
    }

    #[test]
    fn spacenavd_events_use_device_axes_and_latch_buttons() {
        let mut shared = Shared::default();
        // spacenavd: x right, y up, z away from the user.
        assert!(shared.spnav_event([0, 350, -350, 350, 0, 350, 0, 8]));
        assert_eq!(shared.axes.translation, DVec3::new(1., -1., 1.));
        assert_eq!(shared.axes.rotation, DVec3::new(0., 0., -1.));
        assert!(shared.spnav_event([0, 0, 0, 0, i32::MIN, 0, i32::MAX, 8]));
        assert_eq!(shared.axes.translation, DVec3::ZERO);
        assert_eq!(shared.axes.rotation, DVec3::new(-1., -1., 0.));

        assert!(shared.spnav_event([1, 1, 0, 0, 0, 0, 0, 0]));
        assert_eq!(shared.buttons, FIT_BUTTON);
        assert!(shared.spnav_event([2, 1, 0, 0, 0, 0, 0, 0]));
        assert_eq!(shared.buttons, 0);
        assert_eq!(shared.pressed, FIT_BUTTON);
        assert!(shared.spnav_event([1, 99, 0, 0, 0, 0, 0, 0]));
        assert!(shared.spnav_event([1, -1, 0, 0, 0, 0, 0, 0]));
        assert_eq!(shared.buttons, 0);
        assert!(!shared.spnav_event([7, 0, 0, 0, 0, 0, 0, 0]));
    }

    #[test]
    fn finds_the_3dconnexion_device_among_input_devices() {
        let devices = "I: Bus=0003 Vendor=046d Product=c52b Version=0111\n\
            N: Name=\"Logitech USB Receiver\"\n\n\
            I: Bus=0003 Vendor=256f Product=c635 Version=0111\n\
            N: Name=\"3Dconnexion SpaceMouse Compact\"\n";
        assert_eq!(input_device_name(devices), Some("SpaceMouse Compact"));
        assert_eq!(input_device_name("I: Bus=0003 Vendor=046d Product=c52b Version=0111\n"), None);
        assert_eq!(input_device_name(""), None);
    }

    #[test]
    fn dead_zone_removes_drift_and_scaling_stays_monotonic() {
        assert_eq!(normalize_axis(0), 0.);
        assert_eq!(normalize_axis(10), 0.);
        assert_eq!(normalize_axis(-10), 0.);
        assert_eq!(normalize_axis(i16::MAX), 1.);
        assert_eq!(normalize_axis(i16::MIN), -1.);
        let mut previous = 0.;
        for raw in (15..=350).step_by(5) {
            let value = normalize_axis(raw);
            assert!(value > previous);
            assert_eq!(normalize_axis(-raw), -value);
            previous = value;
        }
    }

    #[test]
    fn wheel_input_is_discarded_while_the_cap_moves_and_briefly_after() {
        let wheel = || egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: egui::vec2(0., -40.),
            phase: egui::TouchPhase::Move,
            source: egui::MouseWheelSource::Unknown,
            modifiers: egui::Modifiers::NONE,
        };
        let frame = |space_mouse: &mut SpaceMouse, time: f64| {
            let mut raw = egui::RawInput {
                time: Some(time),
                events: vec![wheel(), egui::Event::Zoom(0.9), egui::Event::Text("a".into())],
                ..Default::default()
            };
            space_mouse.filter_input(&mut raw);
            raw.events.len()
        };
        let mut space_mouse = SpaceMouse::default();
        assert_eq!(frame(&mut space_mouse, 1.), 3, "an idle cap keeps mouse wheel input");
        space_mouse.inject(Axes { rotation: DVec3::X * 0.5, ..Default::default() }, 0);
        assert_eq!(frame(&mut space_mouse, 2.), 1);
        space_mouse.inject(Axes::default(), 0);
        assert_eq!(frame(&mut space_mouse, 2. + WHEEL_GRACE / 2.), 1);
        assert_eq!(frame(&mut space_mouse, 2. + WHEEL_GRACE * 2.), 3);
    }

    const VIEWPORT: DVec2 = DVec2::new(1200., 800.);

    fn moved(axes: Axes, dt: f64, settings: &Settings) -> Camera {
        let mut camera = Camera::default();
        apply(&mut camera, axes, dt, settings, DVec3::ZERO, VIEWPORT);
        camera
    }

    #[test]
    fn pulling_toward_the_user_zooms_in_and_invert_reverses_it() {
        let axes = Axes { translation: DVec3::Y * 0.5, ..Default::default() };
        let start = Camera::default();
        let camera = moved(axes, 0.1, &Settings::default());
        assert!(camera.distance < start.distance);
        assert_eq!((camera.yaw, camera.pitch), (start.yaw, start.pitch));
        let inverted = Settings { invert_zoom: true, ..Default::default() };
        assert!(moved(axes, 0.1, &inverted).distance > start.distance);
    }

    #[test]
    fn sliding_pans_without_rotating_or_zooming() {
        let start = Camera::default();
        let axes = Axes { translation: DVec3::new(0.5, 0., -0.5), ..Default::default() };
        let camera = moved(axes, 0.1, &Settings::default());
        assert_ne!(camera.target, start.target);
        assert_eq!((camera.yaw, camera.pitch), (start.yaw, start.pitch));
        assert!((camera.distance - start.distance).abs() < 1e-12);
        // Object mode: sliding right moves the model right, so the camera moves left.
        assert!((camera.target - start.target).dot(start.right()) < 0.);
        // Lifting the cap lifts the model, so the camera moves down.
        assert!((camera.target - start.target).dot(start.up()) < 0.);
    }

    #[test]
    fn tilt_and_twist_orbit_with_a_level_horizon_and_clamped_pitch() {
        let start = Camera::default();
        let axes = Axes { rotation: DVec3::new(0.5, 0.8, 0.), ..Default::default() };
        let camera = moved(axes, 0.1, &Settings::default());
        assert!(camera.pitch > start.pitch);
        assert_eq!(camera.yaw, start.yaw, "roll must not change the view");
        let twist = Axes { rotation: DVec3::Z * 0.5, ..Default::default() };
        assert!(moved(twist, 0.1, &Settings::default()).yaw > start.yaw);
        let held = moved(axes, 30., &Settings::default());
        assert!(held.pitch <= std::f64::consts::FRAC_PI_2);
        assert!(held.valid());
    }

    #[test]
    fn idle_cap_leaves_camera_unchanged_and_speed_scales_with_time() {
        assert_eq!(moved(Axes::default(), 0.1, &Settings::default()), Camera::default());
        let axes = Axes { rotation: DVec3::X * 0.2, ..Default::default() };
        let start = Camera::default().pitch;
        let short = moved(axes, 0.01, &Settings::default()).pitch - start;
        let long = moved(axes, 0.02, &Settings::default()).pitch - start;
        assert!((long - 2. * short).abs() < 1e-9);
        let fast = Settings { orbit_speed: 2., ..Default::default() };
        assert!((moved(axes, 0.01, &fast).pitch - start - 2. * short).abs() < 1e-9);
    }

    #[test]
    fn settings_normalize_invalid_speeds_and_load_older_data() {
        let mut settings = Settings { orbit_speed: f64::NAN, pan_speed: 10., zoom_speed: 0., ..Default::default() };
        settings.normalize();
        assert_eq!((settings.orbit_speed, settings.pan_speed, settings.zoom_speed), (1., 4., 0.1));
        let older: Settings = serde_json::from_str(r#"{"orbit_speed":2.0}"#).unwrap();
        assert_eq!(older, Settings { orbit_speed: 2., ..Default::default() });
    }
}
