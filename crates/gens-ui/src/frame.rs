//! Pure frame and event data types for the UI core.
//!
//! Everything here is plain data, deliberately free of wgpu AND of egui
//! types: a [`UiFrame`] is the complete, CPU-side description of what a
//! UI pass painted (tessellated meshes + texture changes). The wgpu
//! backend (next sub-item) consumes frames without knowing egui exists,
//! and replay tests can hash frames byte-for-byte.

/// Identifies an egui-managed texture inside a [`UiFrame`].
///
/// `Managed(0)` is egui's font atlas (verified against epaint 0.36:
/// "The first texture (`TextureId::Managed(0)`) is used for the font
/// data"); every managed id arrives through a [`TextureUpload`] before
/// or on the pass that first samples it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TextureRef {
    /// egui-managed texture (font atlas, `ui.image` uploads, ...).
    Managed(u64),
    /// User-registered texture.
    User(u64),
}

/// One CPU-side UI vertex in pixel space.
///
/// - `pos` is in logical pixels, origin top-left (+Y down — the egui
///   convention; the render backend maps to clip space);
/// - `uv` is normalized texture coordinates (0,0 top-left);
/// - `color` is **premultiplied** RGBA with channels normalized from the
///   source bytes (`byte / 255`, kept in the source space — the blending
///   convention is pinned by the render backend's pixel tests).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct UiVertex {
    /// Position in pixels (top-left origin, +Y down).
    pub pos: [f32; 2],
    /// Normalized texture coordinates.
    pub uv: [f32; 2],
    /// Premultiplied RGBA color, channels in 0..=1.
    pub color: [f32; 4],
}

/// One tessellated UI mesh with its texture and clip rectangle.
#[derive(Debug, Clone, PartialEq)]
pub struct UiMesh {
    /// Vertices in pixel space.
    pub vertices: Vec<UiVertex>,
    /// Triangle indices (u32, as egui emits).
    pub indices: Vec<u32>,
    /// Texture all vertices sample.
    pub texture: TextureRef,
    /// Clip rectangle in pixels `[x, y, w, h]` — the render backend
    /// applies it as a scissor rect.
    pub clip_rect_px: [f32; 4],
}

/// A full-texture upload the backend must perform before drawing.
#[derive(Debug, Clone, PartialEq)]
pub struct TextureUpload {
    /// Which texture this (re)defines.
    pub id: TextureRef,
    /// Pixel width.
    pub width: u32,
    /// Pixel height.
    pub height: u32,
    /// RGBA8 bytes, row-major, `width * height * 4` long.
    pub rgba: Vec<u8>,
}

/// A texture the backend may release.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TextureFree {
    /// The texture to drop.
    pub id: TextureRef,
}

/// The complete CPU-side result of one UI pass.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct UiFrame {
    /// Tessellated meshes in paint order.
    pub meshes: Vec<UiMesh>,
    /// Texture uploads to apply before drawing (sorted by texture id —
    /// egui's delta map is a `HashMap`, so sorting here is what makes
    /// frames replayable).
    pub uploads: Vec<TextureUpload>,
    /// Texture frees to apply after drawing (sorted).
    pub frees: Vec<TextureFree>,
    /// Pixels-per-point used for this pass (pinned to 1.0 for
    /// determinism; exposed for diagnostics).
    pub pixels_per_point: f32,
    /// `Primitive::Callback` items skipped during tessellation (none are
    /// produced by this crate's widgets; counted for honesty).
    pub skipped_callbacks: usize,
}

/// Modifier keys held at the time of a UI event.
///
/// Pure data — the platform layer (windowing backend) owns the real
/// device state and stamps each event with the snapshot it observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct UiModifiers {
    /// Either alt key held.
    pub alt: bool,
    /// Either control key held.
    pub ctrl: bool,
    /// Either shift key held.
    pub shift: bool,
    /// The Mac ⌘ Command key (always false on other platforms).
    pub logo: bool,
}

impl UiModifiers {
    /// No modifiers held.
    pub const NONE: Self = Self {
        alt: false,
        ctrl: false,
        shift: false,
        logo: false,
    };

    /// `ctrl` (and `command` on Mac) held — the cross-platform shortcut
    /// modifier egui matches with `Modifiers::command()`.
    #[must_use]
    pub fn ctrl() -> Self {
        Self {
            ctrl: true,
            ..Self::NONE
        }
    }
}

/// Keyboard key vocabulary for the UI event stream.
///
/// Deliberately engine-owned (the same philosophy as `gens-input`'s
/// action vocabulary, ADR-0015): the windowing backend translates
/// native key codes into this enum, and the UI core never sees a
/// platform type. `Character` is uppercase-normalized (`'a'` and `'A'`
/// both arrive as `'A'`) so shortcuts are layout-case independent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UiKey {
    /// A character key, normalized uppercase (letters, digits,
    /// punctuation). The egui mapping covers the Latin set; characters
    /// outside it are dropped by the egui translation and counted.
    Character(char),
    /// Enter / Return.
    Enter,
    /// Tab.
    Tab,
    /// Backspace.
    Backspace,
    /// Delete (forward delete on Mac).
    Delete,
    /// Escape.
    Escape,
    /// Space.
    Space,
    /// Insert.
    Insert,
    /// Home.
    Home,
    /// End.
    End,
    /// Page up.
    PageUp,
    /// Page down.
    PageDown,
    /// Arrow up.
    ArrowUp,
    /// Arrow down.
    ArrowDown,
    /// Arrow left.
    ArrowLeft,
    /// Arrow right.
    ArrowRight,
    /// A key with no egui mapping (delivered so key state stays
    /// truthful; the egui translation drops it and the pass counts it).
    Other,
}

/// An input event for one UI pass.
///
/// Mirrors the [`crate`] input philosophy: the platform layer (here:
/// tests and later the windowing backend) feeds a neutral event stream;
/// nothing in the UI core reads devices directly.
#[derive(Debug, Clone, PartialEq)]
pub enum UiEvent {
    /// Pointer moved to a pixel position.
    PointerMove {
        /// Pixel x.
        x: f32,
        /// Pixel y.
        y: f32,
    },
    /// Pointer button state change.
    PointerButton {
        /// Pixel position.
        x: f32,
        /// Pixel y.
        y: f32,
        /// Which button (0 = primary).
        button: u8,
        /// `true` = pressed, `false` = released.
        pressed: bool,
    },
    /// Pointer left the UI surface.
    PointerGone,
    /// Text input (already-decoded characters).
    Text(String),
    /// Scroll delta in points.
    Scroll {
        /// Horizontal delta.
        dx: f32,
        /// Vertical delta.
        dy: f32,
    },
    /// A key was pressed or released.
    Key {
        /// Which key.
        key: UiKey,
        /// `true` = pressed, `false` = released.
        pressed: bool,
        /// OS auto-repeat press (ignored by shortcut consumers).
        repeat: bool,
        /// Modifier state observed at the time of the event.
        modifiers: UiModifiers,
    },
    /// The set of held modifiers changed (delivered without a key
    /// press — egui tracks shortcut state from this).
    ModifiersChanged(UiModifiers),
}

/// Per-pass input to [`crate::UiContext::run`].
#[derive(Debug, Clone, PartialEq)]
pub struct UiInput {
    /// Deterministic clock in seconds (the engine's simulation time —
    /// never a wall clock; drives egui animations deterministically).
    pub time_seconds: f64,
    /// Delta time in seconds.
    pub dt_seconds: f32,
    /// Surface size in pixels.
    pub size_px: [u32; 2],
    /// Events accumulated since the last pass.
    pub events: Vec<UiEvent>,
}

impl UiInput {
    /// A no-event input at the given size.
    #[must_use]
    pub fn new(size_px: [u32; 2], time_seconds: f64, dt_seconds: f32) -> Self {
        Self {
            time_seconds,
            dt_seconds,
            size_px,
            events: Vec::new(),
        }
    }

    /// Convenience: pointer press at a pixel.
    #[must_use]
    pub fn press(x: f32, y: f32) -> Self {
        Self::with_events(
            [640, 480],
            vec![UiEvent::PointerButton {
                x,
                y,
                button: 0,
                pressed: true,
            }],
        )
    }

    /// Convenience: pointer release at a pixel.
    #[must_use]
    pub fn release(x: f32, y: f32) -> Self {
        Self::with_events(
            [640, 480],
            vec![UiEvent::PointerButton {
                x,
                y,
                button: 0,
                pressed: false,
            }],
        )
    }

    /// Convenience: input with just the given events.
    #[must_use]
    pub fn with_events(size_px: [u32; 2], events: Vec<UiEvent>) -> Self {
        Self {
            time_seconds: 0.0,
            dt_seconds: 1.0 / 60.0,
            size_px,
            events,
        }
    }
}
