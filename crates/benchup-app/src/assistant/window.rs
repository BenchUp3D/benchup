//! Latch / unlatch hosting for the panel.
//!
//! Latched: a normal inspector section. Unlatched on desktop: its own OS
//! window (an immediate viewport, so it can borrow the app), which can be
//! dragged to another monitor. Unlatched where the backend cannot open
//! windows (web): a movable, resizable window inside the canvas. Position and
//! size are remembered in `Preferences::window`.
use super::super::{sidebar, BenchUpApp};
use super::panel::Host;
use crate::natural_text::TextUi as _;
#[cfg(not(target_arch = "wasm32"))]
use crate::theme::Palette;
use benchup_core::tr;
use eframe::egui;

pub(crate) const DEFAULT_SIZE: [f32; 2] = [440., 640.];
pub(crate) const MIN_SIZE: [f32; 2] = [340., 300.];
/// Geometry must hold still this long before it is saved, so dragging a
/// window does not rewrite preferences every frame.
const SETTLE_SECONDS: f64 = 0.4;
const SECTION_KEY: &str = "assistant";

fn embedded_id() -> egui::Id {
    egui::Id::unique("benchup.assistant.window")
}
/// Layer of the embedded `egui::Window` (web), which "Bring to front" raises.
pub(crate) fn embedded_layer() -> egui::LayerId {
    egui::LayerId::new(egui::Order::Middle, embedded_id())
}

pub(crate) fn viewport_id() -> egui::ViewportId {
    egui::ViewportId::from_hash_of("benchup.assistant")
}

/// Last seen window geometry, for debounced saving.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Geometry {
    seen: Option<([f32; 2], [f32; 2])>,
    since: f64,
    /// Frames since the native window opened; used for the one-time position check.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    frames: u32,
}

/// Whether a saved window position can be trusted. Egui exposes only the
/// monitor under a window, so this is a heuristic: coordinates must be sane
/// and within a few screens of the main window, which catches a monitor that
/// has since been unplugged.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn position_usable(
    position: [f32; 2],
    size: [f32; 2],
    main: Option<egui::Rect>,
    monitor: Option<egui::Vec2>,
) -> bool {
    if !position.iter().chain(&size).all(|v| v.is_finite()) {
        return false;
    }
    if position.iter().any(|v| v.abs() > 30_000.) {
        return false;
    }
    let (Some(main), Some(monitor)) = (main, monitor) else {
        return true;
    };
    let window = egui::Rect::from_min_size(egui::pos2(position[0], position[1]), egui::vec2(size[0], size[1]));
    let reach = egui::vec2(monitor.x.max(800.) * 2.5, monitor.y.max(600.) * 2.5);
    main.expand2(reach).intersects(window)
}

/// Where to open when there is no usable saved position: near the main window.
#[cfg(any(not(target_arch = "wasm32"), test))]
pub(crate) fn default_position(main: Option<egui::Rect>, size: [f32; 2]) -> Option<egui::Pos2> {
    main.map(|rect| {
        egui::pos2((rect.max.x - size[0] - 48.).max(rect.min.x), rect.min.y + 96.)
    })
}

impl BenchUpApp {
    /// Show the panel when it is the inspector's turn: a placeholder while detached.
    pub(crate) fn assistant_section_contents(&mut self, ui: &mut egui::Ui) {
        if !self.assistant.settings.window.detached {
            self.assistant_panel(ui, Host::Docked);
            return;
        }
        let ctx = ui.ctx().clone();
        ui.text_label(if ctx.embed_viewports() {
            tr!("assistant.detached_floating")
        } else {
            tr!("assistant.detached_window")
        });
        ui.add_space(4.);
        ui.horizontal(|ui| {
            if ui.button(tr!("assistant.latch_back")).clicked() {
                self.assistant_set_detached(&ctx, false);
            }
            if ui.button(tr!("assistant.bring_to_front")).clicked() {
                self.assistant_raise(&ctx);
            }
        });
    }

    pub(crate) fn assistant_set_detached(&mut self, ctx: &egui::Context, detached: bool) {
        if self.assistant.settings.window.detached == detached {
            return;
        }
        self.assistant.settings.window.detached = detached;
        self.assistant.ui.geometry = Geometry::default();
        if !detached {
            self.inspector = true;
            sidebar::set_section_open(ctx, SECTION_KEY, true);
        }
        self.assistant.ui.focus_prompt = true;
        ctx.request_repaint();
    }

    fn assistant_raise(&self, ctx: &egui::Context) {
        if ctx.embed_viewports() {
            // Web and other single-window backends: lift the embedded window
            // above the other layers. The prompt gets focus from `focus_prompt`.
            ctx.move_to_top(embedded_layer());
        } else {
            ctx.send_viewport_cmd_to(viewport_id(), egui::ViewportCommand::Focus);
        }
        ctx.request_repaint();
    }

    /// Whether the panel is on screen right now.
    pub(crate) fn assistant_visible(&self, ctx: &egui::Context) -> bool {
        self.assistant.settings.window.detached
            || (self.inspector && sidebar::section_open(ctx, SECTION_KEY, false))
    }

    /// Bring the panel forward: raise the window, or open and scroll to the section.
    pub(crate) fn assistant_reveal(&mut self, ctx: &egui::Context) {
        if self.assistant.settings.window.detached {
            self.assistant_raise(ctx);
        } else {
            self.inspector = true;
            sidebar::set_section_open(ctx, SECTION_KEY, true);
            self.assistant.ui.scroll_to_panel = true;
        }
        self.assistant.ui.focus_prompt = true;
        ctx.request_repaint();
    }

    /// View menu / shortcut: show the panel, or collapse it when already open.
    pub(crate) fn assistant_toggle(&mut self, ctx: &egui::Context) {
        if self.assistant.settings.window.detached {
            self.assistant_raise(ctx);
        } else if self.assistant_visible(ctx) {
            sidebar::set_section_open(ctx, SECTION_KEY, false);
        } else {
            self.assistant_reveal(ctx);
        }
        ctx.request_repaint();
    }

    /// Ctrl/Cmd+Shift+<key>; the key is remappable in Settings > Shortcuts.
    pub(crate) fn assistant_shortcut(&self) -> egui::KeyboardShortcut {
        let key = egui::Key::from_name(&self.general_settings.assistant_key).unwrap_or(egui::Key::M);
        egui::KeyboardShortcut::new(egui::Modifiers::COMMAND | egui::Modifiers::SHIFT, key)
    }

    /// Returns true when the shortcut fired and was consumed.
    pub(crate) fn assistant_toggle_shortcut(&mut self, ctx: &egui::Context) -> bool {
        let shortcut = self.assistant_shortcut();
        let fired = ctx.input_mut(|i| i.consume_shortcut(&shortcut));
        // Off: the key combination stays reserved but does nothing.
        if fired && self.assistant_available() {
            self.assistant_toggle(ctx);
        }
        fired
    }

    pub(crate) fn assistant_view_menu_item(&mut self, ui: &mut egui::Ui) {
        if !self.assistant_available() {
            return;
        }
        let shortcut = ui.ctx().format_shortcut(&self.assistant_shortcut());
        let button = Self::menu_item_button(ui, tr!("assistant.name"), Some("assistant"))
            .shortcut_text(shortcut);
        if ui.add(button).on_hover_text(tr!("assistant.menu_help")).clicked() {
            self.assistant_toggle(&ui.ctx().clone());
            ui.close();
        }
    }

    /// Called once per frame. Owns the detached window and the panel's dialogs.
    pub(crate) fn assistant_window(&mut self, ctx: &egui::Context) {
        if !self.assistant_available() {
            return;
        }
        if !self.assistant.settings.window.detached {
            self.assistant.ui.geometry = Geometry::default();
            self.assistant_dialogs(ctx);
            return;
        }
        let size = self.assistant.settings.window.size.unwrap_or(DEFAULT_SIZE);
        #[cfg(not(target_arch = "wasm32"))]
        if !ctx.embed_viewports() {
            self.assistant_native_window(ctx, size);
            return;
        }
        self.assistant_embedded_window(ctx, size);
        self.assistant_dialogs(ctx);
    }

    /// A window inside the canvas: web, or any backend without extra windows.
    fn assistant_embedded_window(&mut self, ctx: &egui::Context, size: [f32; 2]) {
        let screen = ctx.content_rect();
        let mut window = egui::Window::new(tr!("assistant.name"))
            .id(embedded_id())
            .collapsible(false)
            .resizable(true)
            .default_size(size)
            .min_size(MIN_SIZE);
        let saved = self.assistant.settings.window.position;
        let start = saved
            .map(|[x, y]| egui::pos2(x, y))
            .filter(|p| screen.shrink(8.).contains(*p))
            .unwrap_or_else(|| screen.right_top() + egui::vec2(-size[0] - 24., 96.));
        window = window.default_pos(start);
        let mut open = true;
        let shown = window.open(&mut open).show(ctx, |ui| {
            self.assistant_panel(ui, Host::Window);
        });
        if let Some(shown) = shown {
            self.track_geometry(ctx, shown.response.rect.min, shown.response.rect.size());
        }
        if !open {
            self.assistant_set_detached(ctx, false);
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn assistant_native_window(&mut self, ctx: &egui::Context, size: [f32; 2]) {
        let (main, monitor, root_closing) = ctx.input(|i| {
            let info = i.viewport();
            (info.outer_rect, info.monitor_size, info.close_requested())
        });
        let mut builder = egui::ViewportBuilder::default()
            .with_title(tr!("assistant.name"))
            .with_inner_size(size)
            .with_min_inner_size(MIN_SIZE);
        let saved = self
            .assistant
            .settings
            .window
            .position
            .filter(|p| position_usable(*p, size, main, monitor));
        if let Some(position) = saved
            .map(|[x, y]| egui::pos2(x, y))
            .or_else(|| default_position(main, size))
        {
            builder = builder.with_position(position);
        }
        let mut latch = false;
        ctx.show_viewport_immediate(viewport_id(), builder, |ui, _class| {
            let ctx = ui.ctx().clone();
            let colors = Palette::for_ui(ui);
            egui::CentralPanel::default()
                .frame(egui::Frame::new().fill(colors.paper).inner_margin(10))
                .show(ui, |ui| {
                    self.assistant_panel(ui, Host::Window);
                });
            self.assistant_dialogs(&ctx);
            let (outer, inner, monitor_size, close) = ctx.input(|i| {
                let info = i.viewport();
                (info.outer_rect, info.inner_rect, info.monitor_size, info.close_requested())
            });
            if let Some(outer) = outer {
                let inner_size = inner.map_or(outer.size(), |rect| rect.size());
                self.track_geometry(&ctx, outer.min, inner_size);
                // A window that opened on no monitor is brought back near the main one.
                let geometry = &mut self.assistant.ui.geometry;
                geometry.frames = geometry.frames.saturating_add(1);
                if geometry.frames == 3
                    && monitor_size.is_none_or(|m| m.x <= 0. || m.y <= 0.)
                    && let Some(position) = default_position(main, [inner_size.x, inner_size.y])
                {
                    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(position));
                }
            }
            latch |= close;
        });
        // Closing the window latches the panel back, unless the whole app is quitting.
        if latch && !root_closing {
            self.assistant_set_detached(ctx, false);
        }
    }

    /// Remember the window's rectangle once it has stopped moving.
    fn track_geometry(&mut self, ctx: &egui::Context, position: egui::Pos2, size: egui::Vec2) {
        let now = ctx.input(|i| i.time);
        let current = ([position.x, position.y], [size.x, size.y]);
        let geometry = &mut self.assistant.ui.geometry;
        if geometry.seen != Some(current) {
            geometry.seen = Some(current);
            geometry.since = now;
        }
        let window = &mut self.assistant.settings.window;
        let changed = window.position != Some(current.0) || window.size != Some(current.1);
        if !changed {
            return;
        }
        if now - geometry.since >= SETTLE_SECONDS {
            window.position = Some(current.0);
            window.size = Some(current.1);
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
    }
}
