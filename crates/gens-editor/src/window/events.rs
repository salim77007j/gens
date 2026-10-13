//! winit → neutral [`UiEvent`] translation (v1.1 windowing shell).
//!
//! The platform layer's one job (ADR-0015's promise): translate native
//! device traffic into the engine's neutral event vocabulary. Everything
//! here is a pure function of winit's data types — no window, no event
//! loop — so the translation is unit-testable headlessly.
//!
//! Conventions pinned here:
//! - **positions are physical pixels fed as logical points**: the shell
//!   pins egui's `pixels_per_point` to 1.0 (the engine's determinism
//!   position, R-013), so physical == logical by definition; HiDPI
//!   displays render at native resolution with 1:1 UI sizing;
//! - **keys normalize uppercase** for shortcuts ('a' and 'A' are the
//!   same `UiKey::Character('A')`), while **typed text passes through
//!   untouched** (lowercase stays lowercase);
//! - **text is suppressed while ctrl/alt/logo are held** — those are
//!   shortcut chords, not typing;
//! - scroll arrives in point units (one wheel line ≈ 50 pt, the
//!   convention egui's own integration uses).

use gens_ui::{UiEvent, UiKey, UiModifiers};
use winit::event::MouseScrollDelta;
use winit::keyboard::{Key, ModifiersState, NamedKey};

/// Translates winit's modifier state into the neutral snapshot.
#[must_use]
pub fn to_ui_modifiers(m: ModifiersState) -> UiModifiers {
    UiModifiers {
        alt: m.alt_key(),
        ctrl: m.control_key(),
        shift: m.shift_key(),
        logo: m.super_key(),
    }
}

/// Translates a winit logical key into the neutral vocabulary.
///
/// Multi-character `Character` payloads (IME composites) and keys
/// outside the vocabulary map to [`UiKey::Other`] — delivered so key
/// state stays truthful, dropped later by the egui translation.
#[must_use]
pub fn to_ui_key(key: &Key) -> UiKey {
    match key {
        Key::Character(s) => {
            let mut chars = s.chars();
            match (chars.next(), chars.next()) {
                (Some(c), None) => UiKey::Character(c.to_ascii_uppercase()),
                _ => UiKey::Other,
            }
        }
        Key::Named(named) => to_ui_named(*named),
        Key::Unidentified(_) | Key::Dead(_) => UiKey::Other,
    }
}

/// Named-key mapping (the subset the editor UI consumes).
fn to_ui_named(named: NamedKey) -> UiKey {
    match named {
        NamedKey::Enter => UiKey::Enter,
        NamedKey::Tab => UiKey::Tab,
        NamedKey::Backspace => UiKey::Backspace,
        NamedKey::Delete => UiKey::Delete,
        NamedKey::Escape => UiKey::Escape,
        NamedKey::Space => UiKey::Space,
        NamedKey::Insert => UiKey::Insert,
        NamedKey::Home => UiKey::Home,
        NamedKey::End => UiKey::End,
        NamedKey::PageUp => UiKey::PageUp,
        NamedKey::PageDown => UiKey::PageDown,
        NamedKey::ArrowUp => UiKey::ArrowUp,
        NamedKey::ArrowDown => UiKey::ArrowDown,
        NamedKey::ArrowLeft => UiKey::ArrowLeft,
        NamedKey::ArrowRight => UiKey::ArrowRight,
        _ => UiKey::Other,
    }
}

/// Builds the neutral key event for a keyboard state change.
#[must_use]
pub fn key_event(key: &Key, pressed: bool, repeat: bool, modifiers: UiModifiers) -> UiEvent {
    UiEvent::Key {
        key: to_ui_key(key),
        pressed,
        repeat,
        modifiers,
    }
}

/// Builds the neutral text event for typed input.
///
/// `None` when ctrl/alt/logo are held (shortcut chord — egui must see
/// the `Key` event, not a text insertion) or when the payload is empty.
#[must_use]
pub fn text_event(text: &str, modifiers: UiModifiers) -> Option<UiEvent> {
    if modifiers.ctrl || modifiers.alt || modifiers.logo || text.is_empty() {
        return None;
    }
    Some(UiEvent::Text(text.to_string()))
}

/// Maps a mouse button to the neutral button id (0 primary, 1 secondary,
/// 2 middle); `None` for buttons the UI has no use for.
#[must_use]
pub fn button_id(button: winit::event::MouseButton) -> Option<u8> {
    match button {
        winit::event::MouseButton::Left => Some(0),
        winit::event::MouseButton::Right => Some(1),
        winit::event::MouseButton::Middle => Some(2),
        winit::event::MouseButton::Other(_)
        | winit::event::MouseButton::Back
        | winit::event::MouseButton::Forward => None,
    }
}

/// Converts a wheel delta into point units `(dx, dy)`.
///
/// `LineDelta` lines scale at 50 pt/line (egui's own integration
/// convention); `PixelDelta` physical pixels pass through as points
/// (the shell's pixels-per-point is 1.0).
#[must_use]
pub fn scroll_delta(delta: MouseScrollDelta) -> (f32, f32) {
    match delta {
        MouseScrollDelta::LineDelta(x, y) => (x * 50.0, y * 50.0),
        MouseScrollDelta::PixelDelta(px) => (px.x as f32, px.y as f32),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use winit::keyboard::NativeKey;
    use winit::keyboard::SmolStr;

    #[test]
    fn character_keys_normalize_uppercase() {
        assert_eq!(
            to_ui_key(&Key::Character(SmolStr::new("a"))),
            UiKey::Character('A')
        );
        assert_eq!(
            to_ui_key(&Key::Character(SmolStr::new("Z"))),
            UiKey::Character('Z')
        );
        assert_eq!(
            to_ui_key(&Key::Character(SmolStr::new("5"))),
            UiKey::Character('5')
        );
    }

    #[test]
    fn named_keys_map_and_unknowns_stay_other() {
        assert_eq!(to_ui_key(&Key::Named(NamedKey::Escape)), UiKey::Escape);
        assert_eq!(
            to_ui_key(&Key::Named(NamedKey::ArrowLeft)),
            UiKey::ArrowLeft
        );
        assert_eq!(to_ui_key(&Key::Named(NamedKey::F5)), UiKey::Other);
        assert_eq!(
            to_ui_key(&Key::Unidentified(NativeKey::Unidentified)),
            UiKey::Other
        );
    }

    #[test]
    fn text_is_suppressed_during_shortcut_chords() {
        let plain = UiModifiers::NONE;
        let ctrl = UiModifiers::ctrl();
        // Lowercase typing passes through untouched.
        assert_eq!(text_event("h", plain), Some(UiEvent::Text("h".into())));
        // Ctrl held: no text (the key event carries the shortcut).
        assert_eq!(text_event("s", ctrl), None);
        assert_eq!(text_event("", plain), None);
    }

    #[test]
    fn modifiers_translate_field_by_field() {
        let m = ModifiersState::CONTROL | ModifiersState::SHIFT;
        let ui = to_ui_modifiers(m);
        assert!(ui.ctrl && ui.shift && !ui.alt && !ui.logo);
    }

    #[test]
    fn wheel_lines_scale_to_points() {
        let (dx, dy) = scroll_delta(MouseScrollDelta::LineDelta(0.0, -3.0));
        assert_eq!((dx, dy), (0.0, -150.0));
        let (dx, dy) = scroll_delta(MouseScrollDelta::PixelDelta((12, -7).into()));
        assert_eq!((dx, dy), (12.0, -7.0));
    }

    #[test]
    fn buttons_map_to_neutral_ids() {
        assert_eq!(button_id(winit::event::MouseButton::Left), Some(0));
        assert_eq!(button_id(winit::event::MouseButton::Right), Some(1));
        assert_eq!(button_id(winit::event::MouseButton::Middle), Some(2));
        assert_eq!(
            button_id(winit::event::MouseButton::Other(9)),
            None,
            "extraneous buttons are dropped, never mis-mapped"
        );
    }
}
