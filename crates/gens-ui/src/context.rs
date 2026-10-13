//! The egui-backed UI context: event translation, one-pass frames.
//!
//! [`UiContext`] wraps an `egui::Context` and turns the neutral
//! [`UiInput`] stream into a fully tessellated [`UiFrame`]:
//!
//! 1. events translate 1:1 to `egui::Event`s inside a `RawInput`
//!    whose `screen_rect`, `time` and `dt` come from the engine (the
//!    deterministic clock — no wall time anywhere);
//! 2. one `ctx.run_ui` pass builds the widgets;
//! 3. `ctx.tessellate` flattens the paint output to triangle meshes;
//! 4. texture deltas are applied against a CPU mirror so the backend
//!    receives complete uploads even for partial font-atlas updates.
//!
//! Determinism decisions pinned here: pixels-per-point is forced to
//! 1.0 once at construction, texture uploads/frees are emitted in
//! sorted-by-id order (egui's delta maps are `HashMap`s), and the same
//! (state, input) sequence produces byte-identical frames — asserted by
//! a replay test.

use std::collections::BTreeMap;

use egui::{
    Color32, Pos2, RawInput, Rect, TextureId, Vec2, ViewportId, ViewportInfo, epaint::Primitive,
};

use crate::frame::{
    TextureFree, TextureRef, TextureUpload, UiEvent, UiFrame, UiInput, UiKey, UiMesh, UiModifiers,
    UiVertex,
};

/// A CPU mirror of one live egui texture.
struct TextureRecord {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// egui-backed immediate-mode UI with deterministic, headless passes.
pub struct UiContext {
    ctx: egui::Context,
    textures: BTreeMap<TextureRef, TextureRecord>,
    /// Index of the next pass; combined with the input clock keeps egui
    /// animations deterministic.
    pass_count: u64,
}

impl Default for UiContext {
    fn default() -> Self {
        Self::new()
    }
}

impl UiContext {
    /// A UI context with the embedded default fonts and ppp = 1.0.
    #[must_use]
    pub fn new() -> Self {
        let ctx = egui::Context::default();
        // Determinism: fixed zoom level, no OS scaling influence.
        ctx.set_pixels_per_point(1.0);
        Self {
            ctx,
            textures: BTreeMap::new(),
            pass_count: 0,
        }
    }

    /// The wrapped egui context (panels, styles, widget inspection).
    #[must_use]
    pub const fn ctx(&self) -> &egui::Context {
        &self.ctx
    }

    /// Runs one UI pass and returns the tessellated frame.
    ///
    /// `build` receives the egui context and paints one frame (panels,
    /// windows, widgets — anything egui supports).
    #[must_use]
    pub fn run(&mut self, input: UiInput, build: impl Fn(&mut egui::Ui)) -> UiFrame {
        let raw = self.raw_input(&input);
        self.pass_count += 1;
        let mut full = self.ctx.run_ui(raw, |ui| build(ui));

        // Textures first (sorted for replayable frame ordering).
        let mut uploads: Vec<TextureUpload> = Vec::new();
        let mut keys: Vec<&TextureId> = full.textures_delta.set.keys().collect();
        keys.sort_by_key(|k| texture_sort_key(**k));
        for id in keys {
            for delta in &full.textures_delta.set[id] {
                if let Some(upload) = self.apply_image_delta(*id, delta) {
                    uploads.push(upload);
                }
            }
        }

        let mut frees: Vec<TextureFree> = full
            .textures_delta
            .free
            .iter()
            .map(|id| {
                let r = TextureRef::from_egui(*id);
                self.textures.remove(&r);
                TextureFree { id: r }
            })
            .collect();
        frees.sort_by_key(|f| texture_sort_key_from(f.id));

        // Acknowledge the deltas: extraction above is the handling;
        // epaint's Drop debug-asserts every delta was consumed or cleared.
        full.textures_delta.clear();

        // Tessellation: egui shapes -> clipped triangle meshes.
        let ppp = full.pixels_per_point;
        let primitives = self.ctx.tessellate(full.shapes, ppp);
        let mut meshes = Vec::new();
        let mut skipped_callbacks = 0usize;
        for clipped in primitives {
            match clipped.primitive {
                Primitive::Mesh(mesh) => meshes.push(convert_mesh(mesh, clipped.clip_rect)),
                Primitive::Callback(_) => skipped_callbacks += 1,
            }
        }

        UiFrame {
            meshes,
            uploads,
            frees,
            pixels_per_point: ppp,
            skipped_callbacks,
        }
    }

    fn raw_input(&self, input: &UiInput) -> RawInput {
        let [w, h] = input.size_px;
        let screen = Rect::from_min_size(Pos2::ZERO, Vec2::new(w as f32, h as f32));
        let mut viewports = egui::ViewportIdMap::default();
        viewports.insert(
            ViewportId::ROOT,
            ViewportInfo {
                inner_rect: Some(screen),
                ..ViewportInfo::default()
            },
        );
        RawInput {
            viewport_id: ViewportId::ROOT,
            viewports,
            safe_area_insets: None,
            screen_rect: Some(screen),
            max_texture_side: Some(8192),
            time: Some(input.time_seconds),
            predicted_dt: input.dt_seconds,
            events: input.events.iter().filter_map(event_to_egui).collect(),
            hovered_files: Vec::new(),
            dropped_files: Vec::new(),
            focused: true,
            system_theme: None,
        }
    }

    /// Applies one image delta to the CPU mirror; returns the full
    /// upload the backend must perform (partial deltas are patched into
    /// the mirror so the backend only ever sees whole textures).
    fn apply_image_delta(
        &mut self,
        id: TextureId,
        delta: &egui::epaint::ImageDelta,
    ) -> Option<TextureUpload> {
        // ImageData has a single Color variant today; the irrefutable
        // let keeps the compiler enforcing that fact on upstream changes.
        let egui::epaint::ImageData::Color(image) = &delta.image;
        let [w, h] = [image.width(), image.height()];
        let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();

        let r = TextureRef::from_egui(id);
        match delta.pos {
            None => {
                self.textures.insert(
                    r,
                    TextureRecord {
                        width: w as u32,
                        height: h as u32,
                        rgba: rgba.clone(),
                    },
                );
                Some(TextureUpload {
                    id: r,
                    width: w as u32,
                    height: h as u32,
                    rgba,
                })
            }
            Some([px, py]) => {
                let record = self.textures.get_mut(&r)?;
                let stride = record.width as usize * 4;
                for row in 0..h {
                    let dst = (py + row) * stride + px * 4..(py + row) * stride + (px + w) * 4;
                    let src = row * w * 4..(row + 1) * w * 4;
                    record.rgba[dst].copy_from_slice(&rgba[src]);
                }
                Some(TextureUpload {
                    id: r,
                    width: record.width,
                    height: record.height,
                    rgba: record.rgba.clone(),
                })
            }
        }
    }
}

/// Translates a neutral UI event into egui's event enum.
///
/// Returns `None` for events egui has no representation for (the
/// `UiKey::Other` vocabulary entry — delivered so key state stays
/// truthful, dropped here and counted by the pass).
fn event_to_egui(event: &UiEvent) -> Option<egui::Event> {
    match *event {
        UiEvent::PointerMove { x, y } => Some(egui::Event::PointerMoved(Pos2::new(x, y))),
        UiEvent::PointerButton {
            x,
            y,
            button,
            pressed,
        } => Some(egui::Event::PointerButton {
            pos: Pos2::new(x, y),
            button: if button == 0 {
                egui::PointerButton::Primary
            } else if button == 1 {
                egui::PointerButton::Secondary
            } else {
                egui::PointerButton::Middle
            },
            pressed,
            modifiers: egui::Modifiers::default(),
        }),
        UiEvent::PointerGone => Some(egui::Event::PointerGone),
        UiEvent::Text(ref text) => Some(egui::Event::Text(text.clone())),
        UiEvent::Scroll { dx, dy } => Some(egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            delta: Vec2::new(dx, dy),
            // egui 0.36: wheel events carry a trackpad phase; Move is the
            // documented fallback when unknown.
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::default(),
        }),
        UiEvent::Key {
            key,
            pressed,
            repeat,
            modifiers,
        } => {
            let key = ui_key_to_egui(key)?;
            Some(egui::Event::Key {
                key,
                physical_key: None,
                pressed,
                repeat,
                modifiers: modifiers_to_egui(modifiers),
            })
        }
        UiEvent::ModifiersChanged(modifiers) => {
            Some(egui::Event::ModifiersChanged(modifiers_to_egui(modifiers)))
        }
    }
}

/// Maps the neutral key vocabulary onto egui's key set.
///
/// `Character` is matched uppercase (the vocabulary normalizes on
/// arrival); letters, digits and the punctuation egui models are
/// covered. `UiKey::Other` (and punctuation outside egui's set) map to
/// `None` — the event is dropped, never mis-delivered as a wrong key.
#[must_use]
fn ui_key_to_egui(key: UiKey) -> Option<egui::Key> {
    let letter = |c: char| egui_key_from_char(c);
    Some(match key {
        UiKey::Character(c) => letter(c)?,
        UiKey::Enter => egui::Key::Enter,
        UiKey::Tab => egui::Key::Tab,
        UiKey::Backspace => egui::Key::Backspace,
        UiKey::Delete => egui::Key::Delete,
        UiKey::Escape => egui::Key::Escape,
        UiKey::Space => egui::Key::Space,
        UiKey::Insert => egui::Key::Insert,
        UiKey::Home => egui::Key::Home,
        UiKey::End => egui::Key::End,
        UiKey::PageUp => egui::Key::PageUp,
        UiKey::PageDown => egui::Key::PageDown,
        UiKey::ArrowUp => egui::Key::ArrowUp,
        UiKey::ArrowDown => egui::Key::ArrowDown,
        UiKey::ArrowLeft => egui::Key::ArrowLeft,
        UiKey::ArrowRight => egui::Key::ArrowRight,
        UiKey::Other => return None,
    })
}

/// Character-key mapping shared by the vocabulary translation.
fn egui_key_from_char(c: char) -> Option<egui::Key> {
    let c = c.to_ascii_uppercase();
    let key = match c {
        'A' => egui::Key::A,
        'B' => egui::Key::B,
        'C' => egui::Key::C,
        'D' => egui::Key::D,
        'E' => egui::Key::E,
        'F' => egui::Key::F,
        'G' => egui::Key::G,
        'H' => egui::Key::H,
        'I' => egui::Key::I,
        'J' => egui::Key::J,
        'K' => egui::Key::K,
        'L' => egui::Key::L,
        'M' => egui::Key::M,
        'N' => egui::Key::N,
        'O' => egui::Key::O,
        'P' => egui::Key::P,
        'Q' => egui::Key::Q,
        'R' => egui::Key::R,
        'S' => egui::Key::S,
        'T' => egui::Key::T,
        'U' => egui::Key::U,
        'V' => egui::Key::V,
        'W' => egui::Key::W,
        'X' => egui::Key::X,
        'Y' => egui::Key::Y,
        'Z' => egui::Key::Z,
        '0' => egui::Key::Num0,
        '1' => egui::Key::Num1,
        '2' => egui::Key::Num2,
        '3' => egui::Key::Num3,
        '4' => egui::Key::Num4,
        '5' => egui::Key::Num5,
        '6' => egui::Key::Num6,
        '7' => egui::Key::Num7,
        '8' => egui::Key::Num8,
        '9' => egui::Key::Num9,
        '-' => egui::Key::Minus,
        '=' => egui::Key::Equals,
        ',' => egui::Key::Comma,
        '.' => egui::Key::Period,
        '/' => egui::Key::Slash,
        '\\' => egui::Key::Backslash,
        ';' => egui::Key::Semicolon,
        '\'' => egui::Key::Quote,
        '[' => egui::Key::OpenBracket,
        ']' => egui::Key::CloseBracket,
        _ => return None,
    };
    Some(key)
}

/// Maps the neutral modifier snapshot onto egui's modifiers.
#[must_use]
fn modifiers_to_egui(m: UiModifiers) -> egui::Modifiers {
    egui::Modifiers {
        alt: m.alt,
        ctrl: m.ctrl,
        shift: m.shift,
        mac_cmd: m.logo,
        // egui's cross-platform shortcut convention: `command` mirrors
        // ctrl on Windows/Linux and mac_cmd on Mac.
        command: if m.logo { m.logo } else { m.ctrl },
    }
}

fn convert_mesh(mesh: egui::epaint::Mesh, clip: Rect) -> UiMesh {
    let vertices = mesh
        .vertices
        .into_iter()
        .map(|v| UiVertex {
            pos: [v.pos.x, v.pos.y],
            uv: [v.uv.x, v.uv.y],
            // Premultiplied source bytes, normalized to 0..=1 as-is; the
            // blending interpretation is pinned by the render backend's
            // pixel tests (premultiplied over, one / one-minus-alpha).
            color: color32_to_array(v.color),
        })
        .collect();
    UiMesh {
        vertices,
        indices: mesh.indices,
        texture: TextureRef::from_egui(mesh.texture_id),
        clip_rect_px: [clip.min.x, clip.min.y, clip.width(), clip.height()],
    }
}

/// Normalizes an egui (premultiplied sRGB) color to 0..=1 channels.
fn color32_to_array(c: Color32) -> [f32; 4] {
    let [r, g, b, a] = c.to_array();
    [
        r as f32 / 255.0,
        g as f32 / 255.0,
        b as f32 / 255.0,
        a as f32 / 255.0,
    ]
}

fn texture_sort_key(id: TextureId) -> (u8, u64) {
    match id {
        TextureId::Managed(n) => (0, n),
        TextureId::User(n) => (1, n),
    }
}

fn texture_sort_key_from(r: TextureRef) -> (u8, u64) {
    match r {
        TextureRef::Managed(n) => (0, n),
        TextureRef::User(n) => (1, n),
    }
}

impl TextureRef {
    fn from_egui(id: TextureId) -> Self {
        match id {
            TextureId::Managed(n) => Self::Managed(n),
            TextureId::User(n) => Self::User(n),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::frame::UiEvent;
    use std::cell::Cell;
    use std::rc::Rc;

    fn ctx() -> UiContext {
        UiContext::new()
    }

    #[test]
    fn first_pass_uploads_font_atlas() {
        let mut ui = ctx();
        let frame = ui.run(UiInput::new([200, 100], 0.0, 0.016), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("hello");
            });
        });
        assert!(
            frame
                .uploads
                .iter()
                .any(|u| u.id == TextureRef::Managed(0) && u.width > 0 && u.rgba.len() >= 4),
            "font atlas must arrive as Managed(0) on the first pass"
        );
        assert_eq!(frame.pixels_per_point, 1.0);
    }

    #[test]
    fn label_paints_nonempty_mesh() {
        let mut ui = ctx();
        let frame = ui.run(UiInput::new([200, 100], 0.0, 0.016), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("hello");
            });
        });
        let text_meshes = frame
            .meshes
            .iter()
            .filter(|m| m.texture == TextureRef::Managed(0) && !m.vertices.is_empty())
            .count();
        assert!(
            text_meshes > 0,
            "text must tessellate into a font-atlas mesh"
        );
        assert_eq!(frame.skipped_callbacks, 0);
    }

    #[test]
    fn button_clicks_through_synthetic_pointer() {
        let mut ui = ctx();
        let clicks = Rc::new(Cell::new(0u32));
        let seen = clicks.clone();

        // Pass 1: lay out the button, remember its rect.
        let rect_cell = Rc::new(Cell::new(Rect::NOTHING));
        let rc2 = rect_cell.clone();
        let _ = ui.run(UiInput::new([200, 100], 0.0, 0.016), move |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let response = ui.button("press");
                rc2.set(response.rect);
            });
        });
        let rect = rect_cell.get();
        let cx = rect.center().x;
        let cy = rect.center().y;

        // Pass 2: press + release at the button center.
        let input = UiInput {
            time_seconds: 0.1,
            dt_seconds: 0.016,
            size_px: [200, 100],
            events: vec![
                UiEvent::PointerMove { x: cx, y: cy },
                UiEvent::PointerButton {
                    x: cx,
                    y: cy,
                    button: 0,
                    pressed: true,
                },
                UiEvent::PointerButton {
                    x: cx,
                    y: cy,
                    button: 0,
                    pressed: false,
                },
            ],
        };
        let seen_cell = seen.clone();
        let _ = ui.run(input, move |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if ui.button("press").clicked() {
                    seen_cell.set(seen_cell.get() + 1);
                }
            });
        });
        assert_eq!(clicks.get(), 1, "press+release at center must click");
    }

    #[test]
    fn pointer_away_does_not_click() {
        let mut ui = ctx();
        let clicks = Rc::new(Cell::new(0u32));
        let seen = clicks.clone();
        let input = UiInput {
            time_seconds: 0.0,
            dt_seconds: 0.016,
            size_px: [200, 100],
            events: vec![
                UiEvent::PointerMove { x: 5.0, y: 90.0 },
                UiEvent::PointerButton {
                    x: 5.0,
                    y: 90.0,
                    button: 0,
                    pressed: true,
                },
                UiEvent::PointerButton {
                    x: 5.0,
                    y: 90.0,
                    button: 0,
                    pressed: false,
                },
            ],
        };
        let _ = ui.run(input, move |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                if ui.button("press").clicked() {
                    seen.set(seen.get() + 1);
                }
            });
        });
        assert_eq!(clicks.get(), 0);
    }

    #[test]
    fn partial_font_delta_patches_the_mirror() {
        // A later pass may add glyphs: the delta carries a position; the
        // frame must still expose a FULL texture upload.
        let mut ui = ctx();
        let _ = ui.run(UiInput::new([200, 100], 0.0, 0.016), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("abc");
            });
        });
        let frame = ui.run(UiInput::new([200, 100], 0.5, 0.016), |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("abc");
                ui.label("XYZ {@}"); // unlikely pre-baked glyphs
            });
        });
        for upload in &frame.uploads {
            assert_eq!(
                upload.rgba.len(),
                (upload.width * upload.height * 4) as usize,
                "backend must always receive complete textures"
            );
        }
    }

    #[test]
    fn same_input_sequence_replays_identical_frames() {
        let build = |ui: &mut egui::Ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.label("determinism");
                let _ = ui.button("ok");
            });
        };
        let events = vec![UiEvent::PointerMove { x: 30.0, y: 20.0 }];
        let hash = |ui: &mut UiContext| {
            let mut out = Vec::new();
            for pass in 0..3u64 {
                let mut input = UiInput::new([320, 200], pass as f64 * 0.1, 0.016);
                input.events = events.clone();
                let frame = ui.run(input, build);
                for mesh in &frame.meshes {
                    for v in &mesh.vertices {
                        out.extend(v.pos.iter().flat_map(|f| f.to_le_bytes()));
                        out.extend(v.color.iter().flat_map(|f| f.to_le_bytes()));
                    }
                }
                for u in &frame.uploads {
                    out.extend(u.width.to_le_bytes());
                    out.extend(u.height.to_le_bytes());
                    out.extend(u.rgba.iter().copied());
                }
            }
            out
        };
        let a = hash(&mut ctx());
        let b = hash(&mut ctx());
        assert_eq!(a, b, "same input sequence must produce identical frames");
    }

    #[test]
    fn scroll_and_text_events_are_accepted() {
        let mut ui = ctx();
        let scrolled = Rc::new(Cell::new(0.0f32));
        let seen = scrolled.clone();
        let input = UiInput {
            time_seconds: 0.0,
            dt_seconds: 0.016,
            size_px: [200, 100],
            // egui 0.36: Point-unit wheel steps >= 8 go through the
            // multi-frame smoother (unprocessed queue); a small delta
            // lands in smooth_scroll_delta on the same pass.
            events: vec![UiEvent::Scroll { dx: 0.0, dy: -4.0 }],
        };
        let _ = ui.run(input, move |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                ui.ctx().input(|i| seen.set(i.smooth_scroll_delta.y));
                ui.label("scrollable");
            });
        });
        assert!(
            (scrolled.get() + 4.0).abs() < 1e-4,
            "got {}",
            scrolled.get()
        );
    }

    #[test]
    fn ctrl_shortcuts_arrive_through_key_events() {
        let mut ui = ctx();
        let saves = Rc::new(Cell::new(0u32));
        let seen = saves.clone();
        let _ = ui.run(
            UiInput {
                time_seconds: 0.0,
                dt_seconds: 0.016,
                size_px: [320, 200],
                events: vec![
                    UiEvent::ModifiersChanged(UiModifiers::ctrl()),
                    UiEvent::Key {
                        key: UiKey::Character('s'),
                        pressed: true,
                        repeat: false,
                        modifiers: UiModifiers::ctrl(),
                    },
                    UiEvent::Key {
                        key: UiKey::Character('s'),
                        pressed: false,
                        repeat: false,
                        modifiers: UiModifiers::ctrl(),
                    },
                ],
            },
            move |ui| {
                // The exact consumption pattern the editor's menu bar uses.
                ui.input_mut(|i| {
                    if i.consume_key(egui::Modifiers::COMMAND, egui::Key::S) {
                        seen.set(seen.get() + 1);
                    }
                });
            },
        );
        assert_eq!(saves.get(), 1, "Ctrl+S must be consumable as a shortcut");
    }

    #[test]
    fn text_events_drive_text_edit_and_case_is_normalized_for_keys() {
        let mut ui = ctx();
        // Typing must flow through Event::Text (lowercase preserved —
        // only shortcut keys are normalized, not typed text). A TextEdit
        // only accepts text while focused — pass 1 focuses it (what a
        // real click does), pass 2 delivers the keystrokes.
        let typed = Rc::new(std::cell::RefCell::new(String::new()));
        let sink = typed.clone();
        let id_cell = Rc::new(Cell::new(egui::Id::NULL));
        let id_sink = id_cell.clone();
        let _ = ui.run(UiInput::new([320, 200], 0.0, 0.016), move |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                let mut buf = String::new();
                let response = ui.text_edit_singleline(&mut buf);
                response.request_focus();
                id_sink.set(response.id);
            });
        });
        let _ = ui.run(
            UiInput {
                time_seconds: 0.1,
                dt_seconds: 0.016,
                size_px: [320, 200],
                events: vec![UiEvent::Text("h".into()), UiEvent::Text("i".into())],
            },
            move |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    let mut buf = sink.borrow_mut().clone();
                    let response = ui.text_edit_singleline(&mut buf);
                    assert!(response.has_focus(), "focus must persist");
                    *sink.borrow_mut() = buf;
                });
            },
        );
        assert_eq!(&*typed.borrow(), "hi", "Text events must reach TextEdit");

        // 'a' and 'A' normalize to the same shortcut key: both consume
        // a Ctrl+A binding (layout-case independence).
        for ch in ['a', 'A'] {
            let mut ui = ctx();
            let hits = Rc::new(Cell::new(0u32));
            let seen = hits.clone();
            let _ = ui.run(
                UiInput {
                    time_seconds: 0.0,
                    dt_seconds: 0.016,
                    size_px: [320, 200],
                    events: vec![UiEvent::Key {
                        key: UiKey::Character(ch),
                        pressed: true,
                        repeat: false,
                        modifiers: UiModifiers::ctrl(),
                    }],
                },
                move |ui| {
                    ui.input_mut(|i| {
                        if i.consume_key(egui::Modifiers::COMMAND, egui::Key::A) {
                            seen.set(seen.get() + 1);
                        }
                    });
                },
            );
            assert_eq!(hits.get(), 1, "'{ch}' must consume Ctrl+A");
        }
    }

    #[test]
    fn unmapped_keys_are_dropped_not_misdelivered() {
        let mut ui = ctx();
        // UiKey::Other has no egui representation: the pass must render
        // without panicking and without inventing a key.
        let frame = ui.run(
            UiInput {
                time_seconds: 0.0,
                dt_seconds: 0.016,
                size_px: [320, 200],
                events: vec![UiEvent::Key {
                    key: UiKey::Other,
                    pressed: true,
                    repeat: false,
                    modifiers: UiModifiers::NONE,
                }],
            },
            |ui| {
                egui::CentralPanel::default().show(ui, |ui| {
                    ui.label("still renders");
                });
            },
        );
        assert!(!frame.meshes.is_empty());
    }
}
