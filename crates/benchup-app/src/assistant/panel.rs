//! The "Build with MIRA" chat panel: connect flow, transcript with change
//! cards, and the prompt composer with `@` references. The same code renders
//! docked in the inspector and detached in its own window (see `window`).
use super::super::{
    searchable_selector::{matches_query, Choice, SearchableSelector},
    sidebar::{self, OutlinerDrag},
    BenchUpApp,
};
use super::{client, credentials, Changes, Connection, Entry, EntryRole, Prompt};
use crate::{
    icons,
    modal::{primary_button, Modal},
    natural_text::TextUi as _,
    theme::Palette,
};
use benchup_core::{tr, Model, ObjectKind};
use eframe::egui::{self, Key, RichText, Stroke};
use std::collections::{BTreeMap, BTreeSet};

/// Where the panel is being drawn; windows fill their height.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Host {
    Docked,
    Window,
}

/// A reference to one part of the design. The id is authoritative: the label
/// is only what the designer saw when picking it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Chip {
    pub label: String,
    pub id: u64,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct ConnectDialog {
    pub key: String,
    pub base_url: String,
    /// Connect was pressed; close the dialog once the server accepts the key.
    pub submitted: bool,
}

/// Session-only panel state. Nothing here is persisted except through
/// `Preferences` (model, include-selection, privacy flag, window).
#[derive(Default)]
pub(crate) struct UiState {
    pub text: String,
    pub chips: Vec<Chip>,
    pub last_prompt: String,
    pub notice: Option<String>,
    pub connect: Option<ConnectDialog>,
    pub model_filter: String,
    pub mention_index: usize,
    pub mention_dismissed: Option<String>,
    pub focus_prompt: bool,
    pub scroll_to_panel: bool,
    pub key_checked: bool,
    pub models_requested: bool,
    pub geometry: super::window::Geometry,
}

const COMPOSER_ID: &str = "assistant_composer";
const MENTION_LIMIT: usize = 8;
const CARD_ROWS: usize = 12;
const CONNECT_ADVANCED_KEY: &str = "assistant_connect_advanced";

/// Display name and kind of any entity, usable for rows and chips.
pub(crate) fn entity_label(model: &Model, id: u64) -> (String, &'static str) {
    let kind = super::scene::kind_of(model, id);
    let name = match model.objects.get(&id) {
        Some(object) => object.name.clone(),
        None => format!("{kind} #{id}"),
    };
    (name, kind)
}

/// Host part of the server address, for the privacy notice.
pub(crate) fn host_of(base_url: &str) -> String {
    base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// If the caret sits inside an `@query`, its start (byte index) and the query.
pub(crate) fn mention_query(text: &str, cursor_chars: usize) -> Option<(usize, String)> {
    let end = text
        .char_indices()
        .nth(cursor_chars)
        .map_or(text.len(), |(index, _)| index);
    let before = &text[..end];
    let at = before.rfind('@')?;
    let preceded_ok = before[..at].chars().next_back().is_none_or(char::is_whitespace);
    let query = &before[at + 1..];
    (preceded_ok && !query.contains('\n') && query.chars().count() <= 40)
        .then(|| (at, query.to_owned()))
}

/// Rewrite `@Name` mentions to `@Name [#id]` and gather every referenced id.
/// Chips that were never written into the text (dragged, or added from the
/// context menu) are listed at the end so the model still sees them.
pub(crate) fn rewrite_prompt(text: &str, chips: &[Chip]) -> (String, BTreeSet<u64>) {
    let mut by_length: Vec<&Chip> = chips.iter().collect();
    by_length.sort_by_key(|chip| std::cmp::Reverse(chip.label.chars().count()));
    let mut used = BTreeSet::new();
    let mut out = String::new();
    let mut previous: Option<char> = None;
    let mut index = 0;
    while index < text.len() {
        let rest = &text[index..];
        let ch = rest.chars().next().unwrap();
        if ch == '@' && previous.is_none_or(char::is_whitespace) {
            let matched = by_length.iter().find(|chip| {
                !used.contains(&chip.id)
                    && rest[1..].starts_with(&chip.label)
                    && rest[1 + chip.label.len()..]
                        .chars()
                        .next()
                        .is_none_or(|next| !next.is_alphanumeric())
            });
            if let Some(chip) = matched {
                out.push_str(&format!("@{} [#{}]", chip.label, chip.id));
                used.insert(chip.id);
                index += 1 + chip.label.len();
                previous = chip.label.chars().next_back();
                continue;
            }
        }
        out.push(ch);
        previous = Some(ch);
        index += ch.len_utf8();
    }
    let unmentioned: Vec<String> = chips
        .iter()
        .filter(|chip| !used.contains(&chip.id))
        .map(|chip| format!("@{} [#{}]", chip.label, chip.id))
        .collect();
    if !unmentioned.is_empty() {
        out.push_str(&format!("\n\n{}", tr!("assistant.prompt.also_referenced", list = unmentioned.join(", "))));
    }
    (out, chips.iter().map(|chip| chip.id).collect())
}

enum CardAction {
    Focus(u64),
    Undo,
}

impl BenchUpApp {
    /// Add references for `ids`, skipping ones already attached.
    pub(crate) fn assistant_add_chips(&mut self, ids: impl IntoIterator<Item = u64>) {
        for id in ids {
            if !self.doc.model.objects.contains_key(&id) {
                continue;
            }
            let ui = &mut self.assistant.ui;
            if ui.chips.iter().any(|chip| chip.id == id) {
                continue;
            }
            let (label, _) = entity_label(&self.doc.model, id);
            ui.chips.push(Chip { label, id });
        }
    }

    /// Select an entity and frame it, as clicking a change-card row does.
    pub(crate) fn assistant_focus_entity(&mut self, id: u64, ctx: &egui::Context) {
        if !self.doc.model.contains_entity(&id) {
            return;
        }
        self.selected = BTreeSet::from([id]);
        self.fit_selection(ctx);
    }

    /// Text sent for this prompt: chips become `@Name [#id]`, plus the id set.
    pub(crate) fn assistant_compose(&self) -> (String, BTreeSet<u64>) {
        let ui = &self.assistant.ui;
        let live: Vec<Chip> = ui
            .chips
            .iter()
            .filter(|chip| self.doc.model.objects.contains_key(&chip.id))
            .cloned()
            .collect();
        rewrite_prompt(&ui.text, &live)
    }

    pub(crate) fn assistant_submit(&mut self, ctx: &egui::Context) {
        let (prompt, references) = self.assistant_compose();
        if prompt.trim().is_empty() {
            return;
        }
        // A tool in the middle of a gesture must not collide with MIRA's edits.
        self.assistant_cancel_interactions();
        match self.assistant_send(ctx, Prompt { prompt: prompt.clone(), references }) {
            Ok(()) => {
                let ui = &mut self.assistant.ui;
                ui.last_prompt = ui.text.clone();
                ui.text.clear();
                ui.chips.clear();
                ui.notice = None;
                ui.focus_prompt = true;
            }
            Err(error) => self.assistant.ui.notice = Some(error),
        }
    }

    pub(crate) fn assistant_cancel_interactions(&mut self) {
        self.last_move_copy = None;
        self.length_input.clear();
        self.measurement = None;
        self.drawing = None;
        self.modeling = None;
        self.gesture = None;
        self.line_start = None;
        self.line_group = None;
        self.snap = None;
        self.paste_preview = None;
        self.solid_first = None;
    }

    /// The whole panel body. Safe to call from the dock or a detached window.
    pub(crate) fn assistant_panel(&mut self, ui: &mut egui::Ui, host: Host) {
        if std::mem::take(&mut self.assistant.ui.scroll_to_panel) {
            ui.scroll_to_cursor(Some(egui::Align::Center));
        }
        if !self.assistant.ui.key_checked {
            self.assistant.ui.key_checked = true;
            self.assistant.load_key();
        }
        let ctx = ui.ctx().clone();
        if self.assistant.key.is_some()
            && self.assistant.models.is_empty()
            && !self.assistant.ui.models_requested
            && self.assistant.connection != Connection::Connecting
        {
            self.assistant.ui.models_requested = true;
            self.assistant.refresh_models(&ctx);
        }
        self.assistant_toolbar(ui, host);
        if self.assistant.key.is_none() {
            self.assistant_disconnected(ui);
            return;
        }
        let composer_height = 170.;
        let transcript_height = match host {
            Host::Docked => 280.,
            Host::Window => (ui.available_height() - composer_height).max(120.),
        };
        self.assistant_transcript(ui, transcript_height, &ctx);
        ui.add_space(6.);
        self.assistant_composer(ui);
    }

    fn assistant_toolbar(&mut self, ui: &mut egui::Ui, host: Host) {
        let colors = Palette::for_ui(ui);
        let ctx = ui.ctx().clone();
        let connected = self.assistant.key.is_some();
        ui.horizontal(|ui| {
            // Right-aligned actions first so the picker takes the remaining width.
            let mut latch = false;
            let mut new_chat = false;
            let mut forget = false;
            let mut endpoint = false;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (label, icon) = match host {
                    Host::Docked => (tr!("assistant.unlatch"), "unlatch"),
                    Host::Window => (tr!("assistant.latch"), "latch"),
                };
                if icon_button(ui, icon, label, colors.muted) {
                    latch = true;
                }
                if connected {
                    let menu = ui
                        .add(
                            egui::Button::new(icons::ellipsis_atom(colors.muted))
                                .min_size(egui::vec2(28., 24.)),
                        )
                        .on_hover_text(tr!("assistant.more"));
                    egui::Popup::menu(&menu)
                        .align(egui::emath::RectAlign::BOTTOM_END)
                        .style(Self::menu_style)
                        .show(|ui| {
                            if ui.button(tr!("assistant.change_endpoint")).clicked() {
                                endpoint = true;
                                ui.close();
                            }
                            if ui.button(tr!("assistant.forget_key")).clicked() {
                                forget = true;
                                ui.close();
                            }
                        });
                    if ui
                        .add_enabled(
                            !self.assistant.busy() && !self.assistant.transcript.is_empty(),
                            egui::Button::new(tr!("assistant.new_chat")),
                        )
                        .clicked()
                    {
                        new_chat = true;
                    }
                    self.assistant_model_picker(ui);
                } else {
                    ui.text_weak(tr!("assistant.not_connected"));
                }
            });
            if latch {
                let detach = host == Host::Docked;
                self.assistant_set_detached(&ctx, detach);
            }
            if new_chat {
                self.assistant.new_chat();
                self.assistant.ui.notice = None;
            }
            if forget
                && let Err(error) = self.assistant.forget_key()
            {
                self.assistant.ui.notice = Some(error);
            }
            if forget {
                self.assistant.ui.models_requested = false;
            }
            if endpoint {
                self.assistant_open_connect(&ctx, true);
            }
        });
        ui.add_space(4.);
    }

    fn assistant_model_picker(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let typed = self.assistant.ui.model_filter.trim().to_owned();
        let mut choices: Vec<Choice<String>> = self
            .assistant
            .models
            .iter()
            .map(|model| Choice::new(model.clone(), model.clone()))
            .collect();
        // Any model id can be typed, even when the server's list omits it.
        if !typed.is_empty() && !self.assistant.models.contains(&typed) {
            let mut choice = Choice::new(typed.clone(), tr!("assistant.use_model", model = typed));
            choice.always_visible = true;
            choices.push(choice);
        }
        let mut selected = self.assistant.settings.model.clone();
        let label = if selected.is_empty() {
            tr!("assistant.choose_model").to_owned()
        } else {
            selected.clone()
        };
        let before = selected.clone();
        let reload = ui
            .add_enabled(
                self.assistant.connection != Connection::Connecting,
                egui::Button::new("⟳").min_size(egui::vec2(24., 24.)),
            )
            .on_hover_text(tr!("assistant.reload_models"))
            .clicked();
        SearchableSelector {
            id_salt: egui::IdSalt::new("assistant_model"),
            scroll_key: "assistant_models",
            selected_label: &label,
            width: ui.available_width().max(80.),
            popup_width: 260.,
            hint: tr!("assistant.search_models"),
            empty_text: tr!("assistant.no_models_match"),
        }
        .show(ui, &mut selected, &mut self.assistant.ui.model_filter, choices);
        if selected != before && !self.assistant.busy() {
            self.assistant.settings.model = selected.trim().to_owned();
        }
        if reload {
            self.assistant.refresh_models(&ctx);
        }
    }

    fn assistant_disconnected(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        ui.text_label(tr!("assistant.explainer"));
        ui.add_space(6.);
        if let Connection::Failed(message) = &self.assistant.connection {
            let colors = Palette::for_ui(ui);
            ui.add(egui::Label::new(RichText::new(message).color(colors.accent)).wrap());
            ui.add_space(4.);
        }
        if let Some(error) = self.assistant.ui.notice.clone() {
            ui.text_weak(error);
        }
        if ui.button(tr!("assistant.connect")).clicked() {
            self.assistant_open_connect(&ctx, false);
        }
    }

    pub(crate) fn assistant_open_connect(&mut self, ctx: &egui::Context, show_endpoint: bool) {
        self.assistant.ui.connect = Some(ConnectDialog {
            key: String::new(),
            base_url: self.assistant.settings.base_url.clone(),
            submitted: false,
        });
        if show_endpoint {
            sidebar::set_section_open(ctx, CONNECT_ADVANCED_KEY, true);
        }
        ctx.request_repaint();
    }

    /// Modal dialogs belong to whichever window currently hosts the panel.
    pub(crate) fn assistant_dialogs(&mut self, ctx: &egui::Context) {
        let Some(mut dialog) = self.assistant.ui.connect.take() else {
            return;
        };
        let connecting = self.assistant.connection == Connection::Connecting;
        if dialog.submitted && self.assistant.connection == Connection::Connected {
            return; // accepted: leave the dialog closed
        }
        let mut cancel = false;
        let mut connect = false;
        let error = match &self.assistant.connection {
            Connection::Failed(message) if dialog.submitted => Some(message.clone()),
            _ => None,
        };
        let invalid_url = !client::valid_base_url(&client::normalize_base(&dialog.base_url));
        let response = Modal::new("assistant_connect", 460.).show(ctx, |modal| {
            modal.header(|ui| {
                ui.text_heading(tr!("assistant.connect_title"));
                ui.text_weak(tr!("assistant.connect_subtitle"));
            });
            modal.body(|ui| {
                ui.text_label(tr!("assistant.api_key"));
                let field = ui.add(
                    egui::TextEdit::singleline(&mut dialog.key)
                        .password(true)
                        .hint_text(tr!("assistant.api_key_hint"))
                        .desired_width(f32::INFINITY),
                );
                if field.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                    connect = true;
                }
                if !dialog.submitted && dialog.key.is_empty() {
                    field.request_focus();
                }
                ui.hyperlink_to(tr!("assistant.get_a_key"), client::KEY_PORTAL_URL);
                let open = sidebar::restored_header_open(ui, CONNECT_ADVANCED_KEY, false);
                let header = egui::CollapsingHeader::new(tr!("assistant.advanced"))
                    .id_salt(CONNECT_ADVANCED_KEY)
                    .open(open)
                    .show(ui, |ui| {
                        ui.text_label(tr!("assistant.base_url"));
                        ui.add(
                            egui::TextEdit::singleline(&mut dialog.base_url)
                                .hint_text(client::DEFAULT_BASE_URL)
                                .desired_width(f32::INFINITY),
                        );
                        if invalid_url {
                            let colors = Palette::for_ui(ui);
                            ui.add(
                                egui::Label::new(
                                    RichText::new(tr!("assistant.error.invalid_server_address"))
                                        .color(colors.accent),
                                )
                                .wrap(),
                            );
                        }
                    });
                sidebar::remember_section(ui.ctx(), header.header_response.id, CONNECT_ADVANCED_KEY);
                if connecting {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.text_weak(tr!("assistant.connecting"));
                    });
                } else if let Some(message) = &error {
                    let colors = Palette::for_ui(ui);
                    ui.add(egui::Label::new(RichText::new(message).color(colors.accent)).wrap());
                }
            });
            modal.footer(|ui| {
                // Least impactful first: the footer lays out right to left.
                cancel = ui.button(tr!("common.cancel")).clicked();
                let ready = !connecting && !invalid_url;
                connect |= primary_button(ui, tr!("assistant.connect"), ready).clicked();
            });
        });
        let url = client::normalize_base(&dialog.base_url);
        if connect && !connecting && client::valid_base_url(&url) {
            if url != self.assistant.settings.base_url {
                // Another server has its own key. Nothing about the current
                // server changes until this one accepts the key.
                let typed = dialog.key.trim();
                let key = if typed.is_empty() { credentials::load(&url).unwrap_or_default() } else { typed.to_owned() };
                self.assistant.connect_to(ctx, &url, &key);
                dialog.submitted = true;
            } else if dialog.key.trim().is_empty() && self.assistant.key.is_some() {
                // A key for this server is already stored on the device.
                self.assistant.refresh_models(ctx);
                return;
            } else {
                self.assistant.connect(ctx, &dialog.key);
                dialog.submitted = true;
            }
        }
        if cancel || response.should_close() {
            if matches!(self.assistant.connection, Connection::Failed(_)) {
                self.assistant.connection = if self.assistant.key.is_some() {
                    Connection::Connected
                } else {
                    Connection::Disconnected
                };
            }
            return;
        }
        self.assistant.ui.connect = Some(dialog);
    }

    fn assistant_transcript(&mut self, ui: &mut egui::Ui, height: f32, ctx: &egui::Context) {
        let colors = Palette::for_ui(ui);
        let history = &self.doc.history;
        let latest = history.labels().count();
        let position = history.position();
        let mut actions = Vec::new();
        let model = &self.doc.model;
        egui::Frame::new()
            .fill(colors.canvas)
            .stroke(Stroke::new(1., colors.border))
            .corner_radius(8)
            .inner_margin(6)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                egui::ScrollArea::vertical()
                    .id_salt("assistant_transcript")
                    .max_height(height)
                    .min_scrolled_height(height.min(120.))
                    .auto_shrink([false, false])
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if self.assistant.transcript.is_empty() {
                            ui.text_weak(tr!("assistant.empty_transcript"));
                        }
                        for (index, entry) in self.assistant.transcript.iter().enumerate() {
                            ui.push_id(index, |ui| {
                                transcript_entry(ui, entry, model, position, latest, &mut actions);
                            });
                            ui.add_space(4.);
                        }
                    });
            });
        for action in actions {
            match action {
                CardAction::Focus(id) => self.assistant_focus_entity(id, ctx),
                CardAction::Undo => self.undo(ctx, false),
            }
        }
    }

    /// Entities offered after `@`, selected ones first.
    fn assistant_mention_candidates(&self, query: &str) -> Vec<(u64, String, &'static str)> {
        let model = &self.doc.model;
        let mut found: Vec<(u64, String, &'static str)> = model
            .objects
            .values()
            .filter_map(|object| {
                let kind = match object.kind {
                    ObjectKind::Group => tr!("assistant.kind_group"),
                    ObjectKind::Component { .. } => tr!("assistant.kind_component"),
                };
                matches_query(&format!("{} {kind}", object.name), query)
                    .then(|| (object.id, object.name.clone(), kind))
            })
            .collect();
        found.sort_by(|a, b| {
            let selected = |id: &u64| !self.selected.contains(id);
            (selected(&a.0), a.1.to_lowercase(), a.0).cmp(&(selected(&b.0), b.1.to_lowercase(), b.0))
        });
        found.truncate(MENTION_LIMIT);
        found
    }

    fn assistant_composer(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let colors = Palette::for_ui(ui);
        let busy = self.assistant.busy();
        let id = egui::Id::unique(COMPOSER_ID);
        let mut state = std::mem::take(&mut self.assistant.ui);

        // References as removable pills, then the implicit selection chip.
        let live_selection = self
            .selected
            .iter()
            .filter(|id| self.doc.model.objects.contains_key(id))
            .count();
        let show_selection = self.assistant.settings.include_selection && live_selection > 0;
        if !state.chips.is_empty() || show_selection {
            ui.horizontal_wrapped(|ui| {
                let mut remove = None;
                for (index, chip) in state.chips.iter().enumerate() {
                    let exists = self.doc.model.objects.contains_key(&chip.id);
                    if chip_pill(ui, &format!("@{}", chip.label), exists, false, &colors) {
                        remove = Some(index);
                    }
                }
                if let Some(index) = remove {
                    state.chips.remove(index);
                }
                if show_selection {
                    ui.add(egui::Label::new(
                        RichText::new(tr!("assistant.selection_chip", count = live_selection))
                            .small()
                            .weak()
                            .italics(),
                    ))
                    .on_hover_text(tr!("assistant.selection_chip_help"));
                }
            });
            ui.add_space(2.);
        }

        // Keys that act on the popup or the prompt are taken before the editor sees them.
        let focused = ctx.memory(|m| m.has_focus(id));
        let cursor = egui::TextEdit::load_state(&ctx, id)
            .and_then(|s| s.cursor.char_range())
            .map_or(state.text.chars().count(), |r| r.primary.index.into());
        let mention = mention_query(&state.text, cursor)
            .filter(|_| state.mention_dismissed.as_deref() != Some(state.text.as_str()));
        let candidates = mention
            .as_ref()
            .map(|(_, query)| self.assistant_mention_candidates(query))
            .unwrap_or_default();
        let popup_open = focused && !candidates.is_empty();
        let mut chosen: Option<usize> = None;
        let mut submit = false;
        if popup_open {
            state.mention_index = state.mention_index.min(candidates.len() - 1);
            ctx.input_mut(|i| {
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowDown) {
                    state.mention_index = (state.mention_index + 1) % candidates.len();
                }
                if i.consume_key(egui::Modifiers::NONE, Key::ArrowUp) {
                    state.mention_index =
                        (state.mention_index + candidates.len() - 1) % candidates.len();
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Enter)
                    || i.consume_key(egui::Modifiers::NONE, Key::Tab)
                {
                    chosen = Some(state.mention_index);
                }
                if i.consume_key(egui::Modifiers::NONE, Key::Escape) {
                    state.mention_dismissed = Some(state.text.clone());
                }
            });
        } else if focused {
            ctx.input_mut(|i| {
                // egui's key matching ignores Shift, so look at the event itself:
                // Shift+Enter is left for the text box to insert a newline.
                if !busy && consume_plain_enter(i) {
                    submit = true;
                }
                if state.text.is_empty()
                    && !state.last_prompt.is_empty()
                    && i.consume_key(egui::Modifiers::NONE, Key::ArrowUp)
                {
                    state.text = state.last_prompt.clone();
                }
            });
        }

        let edit_rect;
        let output = egui::TextEdit::multiline(&mut state.text)
            .id(id)
            .desired_rows(3)
            .desired_width(f32::INFINITY)
            .hint_text(tr!("assistant.prompt_hint"))
            .lock_focus(true)
            .show(ui);
        edit_rect = output.response.rect;
        if std::mem::take(&mut state.focus_prompt) {
            output.response.request_focus();
        }
        // Dropping an Outliner row on the prompt attaches it as a reference.
        if let Some(drag) = output.response.dnd_hover_payload::<OutlinerDrag>() {
            ui.painter().rect_stroke(
                edit_rect.expand(1.),
                4,
                Stroke::new(2., colors.green),
                egui::StrokeKind::Outside,
            );
            let _ = drag;
        }
        let dropped: Option<Vec<u64>> = output
            .response
            .dnd_release_payload::<OutlinerDrag>()
            .map(|drag| drag.ids.iter().copied().collect());
        // Choose from the popup by mouse.
        if popup_open {
            let area = egui::Area::new(id.with("mentions"))
                .order(egui::Order::Foreground)
                .fixed_pos(edit_rect.left_top())
                .pivot(egui::Align2::LEFT_BOTTOM);
            area.show(&ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.set_width(edit_rect.width().clamp(180., 320.));
                    for (row, (entity, name, kind)) in candidates.iter().enumerate() {
                        let response = ui
                            .horizontal(|ui| {
                                let selected = row == state.mention_index;
                                let response = ui.selectable_label(selected, name.as_str());
                                ui.text_weak(format!("{kind} #{entity}"));
                                response
                            })
                            .inner;
                        if response.clicked() {
                            chosen = Some(row);
                        }
                    }
                });
            });
        }
        // Chip insertion: the mention text stays, tagged with the entity's id on send.
        if let (Some(row), Some((start, _))) = (chosen, mention.as_ref()) {
            let (entity, name, _) = candidates[row].clone();
            let end = state
                .text
                .char_indices()
                .nth(cursor)
                .map_or(state.text.len(), |(index, _)| index);
            state.text.replace_range(*start..end, &format!("@{name} "));
            if !state.chips.iter().any(|chip| chip.id == entity) {
                state.chips.push(Chip { label: name.clone(), id: entity });
            }
            let caret = state.text[..*start].chars().count() + name.chars().count() + 2;
            if let Some(mut edit) = egui::TextEdit::load_state(&ctx, id) {
                edit.cursor
                    .set_char_range(Some(egui::text::CCursorRange::one(egui::text::CCursor::new(caret))));
                edit.store(&ctx, id);
            }
            state.mention_index = 0;
            state.focus_prompt = true;
            ctx.request_repaint();
        }
        self.assistant.ui = state;
        if let Some(ids) = dropped {
            self.assistant_add_chips(ids);
        }

        // Privacy note, shown until dismissed for good.
        if !self.assistant.settings.privacy_dismissed {
            ui.horizontal_wrapped(|ui| {
                ui.text_weak(tr!(
                    "assistant.privacy",
                    host = host_of(&self.assistant.settings.base_url)
                ));
                if ui.small_button(tr!("assistant.dont_show_again")).clicked() {
                    self.assistant.settings.privacy_dismissed = true;
                }
            });
        }
        if let Some(notice) = self.assistant.ui.notice.clone() {
            ui.add(egui::Label::new(RichText::new(notice).color(colors.accent)).wrap());
        }
        ui.horizontal(|ui| {
            ui.checkbox(
                &mut self.assistant.settings.include_selection,
                tr!("assistant.include_selection"),
            )
            .on_hover_text(tr!("assistant.include_selection_help"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if busy {
                    if ui.button(tr!("assistant.stop")).clicked() {
                        self.assistant_stop(&ctx);
                    }
                    if let Some((tools, started)) = self.assistant.progress() {
                        let elapsed = (ctx.input(|i| i.time) - started).max(0.) as u64;
                        let mut text = tr!("assistant.working", seconds = elapsed);
                        if let Some(tool) = tools.last() {
                            text = format!("{text} · {tool}");
                        }
                        ui.add(egui::Label::new(RichText::new(text).small().weak()).truncate());
                        ui.spinner();
                    }
                } else {
                    let ready = !self.assistant.ui.text.trim().is_empty();
                    if primary_button(ui, tr!("assistant.send"), ready)
                        .on_hover_text(tr!("assistant.send_help"))
                        .clicked()
                    {
                        submit = true;
                    }
                }
            });
        });
        if submit {
            self.assistant_submit(&ctx);
        }
    }
}

fn icon_button(ui: &mut egui::Ui, icon: &str, tooltip: &str, color: egui::Color32) -> bool {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(28., 24.), egui::Sense::click());
    if response.hovered() {
        ui.painter()
            .rect_filled(rect, 4, ui.visuals().widgets.hovered.weak_bg_fill);
    }
    icons::line_icon(ui.painter(), rect.center(), icon, color);
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tooltip));
    response
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .on_hover_text(tooltip)
        .clicked()
}

/// A reference pill; returns true when its remove button is pressed.
fn chip_pill(ui: &mut egui::Ui, text: &str, exists: bool, dimmed: bool, colors: &Palette) -> bool {
    let mut remove = false;
    egui::Frame::new()
        .fill(colors.canvas)
        .stroke(Stroke::new(1., if exists { colors.green } else { colors.accent }))
        .corner_radius(10)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 4.;
                let mut label = RichText::new(text).small().color(colors.ink);
                if dimmed || !exists {
                    label = label.weak();
                }
                if !exists {
                    label = label.strikethrough();
                }
                ui.add(egui::Label::new(label));
                if ui
                    .add(egui::Button::new(RichText::new("×").small()).frame(false))
                    .on_hover_text(tr!("assistant.remove_reference"))
                    .clicked()
                {
                    remove = true;
                }
            });
        });
    remove
}

fn transcript_entry(
    ui: &mut egui::Ui,
    entry: &Entry,
    model: &Model,
    position: usize,
    latest: usize,
    actions: &mut Vec<CardAction>,
) {
    let colors = Palette::for_ui(ui);
    let (fill, stroke, text_color) = match entry.role {
        EntryRole::User => (colors.hovered, colors.border, colors.ink),
        EntryRole::Assistant => (colors.surface, colors.border, colors.ink),
        EntryRole::Error => (colors.selected, colors.accent, colors.accent),
    };
    egui::Frame::new()
        .fill(fill)
        .stroke(Stroke::new(1., stroke))
        .corner_radius(8)
        .inner_margin(8)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let who = match entry.role {
                EntryRole::User => tr!("assistant.you"),
                EntryRole::Assistant => tr!("assistant.name"),
                EntryRole::Error => tr!("assistant.error_label"),
            };
            ui.add(egui::Label::new(RichText::new(who).small().strong().color(colors.muted)));
            if !entry.text.is_empty() {
                ui.add(
                    egui::Label::new(RichText::new(&entry.text).color(text_color))
                        .wrap()
                        .selectable(true),
                );
            }
            if let Some(changes) = &entry.changes {
                change_card(ui, changes, model, position, latest, actions);
            }
        });
}

/// Remove and report an Enter key press that does not hold Shift.
fn consume_plain_enter(input: &mut egui::InputState) -> bool {
    let position = input.events.iter().position(|event| {
        matches!(event, egui::Event::Key { key: Key::Enter, pressed: true, modifiers, .. } if !modifiers.shift)
    });
    position.is_some_and(|index| {
        input.events.remove(index);
        true
    })
}

fn change_card(
    ui: &mut egui::Ui,
    changes: &Changes,
    model: &Model,
    position: usize,
    latest: usize,
    actions: &mut Vec<CardAction>,
) {
    let colors = Palette::for_ui(ui);
    ui.add_space(4.);
    egui::Frame::new()
        .fill(colors.canvas)
        .stroke(Stroke::new(1., colors.border))
        .corner_radius(6)
        .inner_margin(6)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let mut summary = tr!(
                "assistant.card.summary",
                created = changes.created.len(),
                modified = changes.modified.len(),
                erased = changes.erased.len(),
                joints = changes.joints.len()
            );
            if changes.stopped {
                summary = format!("{summary} · {}", tr!("assistant.card.stopped"));
            }
            ui.add(egui::Label::new(RichText::new(summary).small().strong().color(colors.ink)).wrap());
            let rows = changes
                .created
                .iter()
                .map(|id| (*id, tr!("assistant.card.created")))
                .chain(changes.modified.iter().map(|id| (*id, tr!("assistant.card.modified"))))
                .chain(changes.joints.iter().map(|id| (*id, tr!("assistant.card.joint"))));
            let total = changes.created.len() + changes.modified.len() + changes.joints.len() + changes.erased.len();
            let mut shown = 0;
            for (id, verb) in rows {
                if shown >= CARD_ROWS {
                    break;
                }
                shown += 1;
                let (name, kind) = entity_label(model, id);
                let exists = model.contains_entity(&id);
                let response = ui
                    .push_id(("card_row", id), |ui| {
                        ui.add_enabled(
                            exists,
                            egui::Button::new(
                                RichText::new(format!("{verb} · {name}")).small().color(colors.ink),
                            )
                            .frame(false)
                            .wrap(),
                        )
                    })
                    .inner
                    .on_hover_text(tr!("assistant.card.row_help", kind = kind));
                if response.clicked() {
                    actions.push(CardAction::Focus(id));
                }
            }
            for id in &changes.erased {
                if shown >= CARD_ROWS {
                    break;
                }
                shown += 1;
                let name = changes
                    .erased_names
                    .get(id)
                    .cloned()
                    .unwrap_or_else(|| format!("#{id}"));
                ui.text_weak(format!("{} · {name}", tr!("assistant.card.erased")));
            }
            // Every category counts, erased rows included.
            if total > shown {
                ui.text_weak(tr!("assistant.card.more", count = total - shown));
            }
            // Only the newest history step can be undone without disturbing later edits.
            let undoable = turn_undoable(changes, position, latest);
            if ui
                .add_enabled(undoable, egui::Button::new(tr!("assistant.card.undo_turn")))
                .on_hover_text(if undoable {
                    tr!("assistant.card.undo_turn_help")
                } else {
                    tr!("assistant.card.undo_turn_disabled")
                })
                .clicked()
            {
                actions.push(CardAction::Undo);
            }
        });
}

/// Only the newest history step can be undone without disturbing later edits:
/// the turn must sit exactly at the current position, with nothing redoable after it.
pub(crate) fn turn_undoable(changes: &Changes, position: usize, latest: usize) -> bool {
    changes.position.is_some_and(|p| p == position && position == latest)
}

/// Names of entities removed by a turn, for its card after they are gone.
pub(crate) fn erased_names(before: &Model, erased: &[u64]) -> BTreeMap<u64, String> {
    erased
        .iter()
        .map(|id| (*id, entity_label(before, *id).0))
        .collect()
}

impl BenchUpApp {
    /// A small overlay while MIRA edits: user edits are paused, and Stop is one click away.
    pub(crate) fn assistant_busy_banner(&mut self, ctx: &egui::Context) {
        if !self.assistant.busy() {
            return;
        }
        let anchor = self.viewport.center_top() + egui::vec2(0., 12.);
        let mut stop = false;
        egui::Area::new(egui::Id::unique("assistant_busy_banner"))
            .order(egui::Order::Foreground)
            .fixed_pos(anchor)
            .pivot(egui::Align2::CENTER_TOP)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.text_label(tr!("assistant.banner"));
                        stop = ui.button(tr!("assistant.stop")).clicked();
                    });
                });
            });
        if stop {
            self.assistant_stop(ctx);
        }
    }
}
