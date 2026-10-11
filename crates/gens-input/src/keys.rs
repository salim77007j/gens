//! Device-level vocabulary: keys, mouse buttons, and the event stream.
//!
//! These enums are the engine's own abstraction, deliberately independent of
//! any windowing library so the action model (and every test) survives
//! backend changes. Platform backends translate their native events into
//! [`DeviceEvent`] values and feed [`crate::state::InputState`].

/// A keyboard key the engine recognizes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum KeyCode {
    /// Letter keys A through Z.
    KeyA,
    /// B.
    KeyB,
    /// C.
    KeyC,
    /// D.
    KeyD,
    /// E.
    KeyE,
    /// F.
    KeyF,
    /// G.
    KeyG,
    /// H.
    KeyH,
    /// I.
    KeyI,
    /// J.
    KeyJ,
    /// K.
    KeyK,
    /// L.
    KeyL,
    /// M.
    KeyM,
    /// N.
    KeyN,
    /// O.
    KeyO,
    /// P.
    KeyP,
    /// Q.
    KeyQ,
    /// R.
    KeyR,
    /// S.
    KeyS,
    /// T.
    KeyT,
    /// U.
    KeyU,
    /// V.
    KeyV,
    /// W.
    KeyW,
    /// X.
    KeyX,
    /// Y.
    KeyY,
    /// Z.
    KeyZ,
    /// Arrow up.
    ArrowUp,
    /// Arrow down.
    ArrowDown,
    /// Arrow left.
    ArrowLeft,
    /// Arrow right.
    ArrowRight,
    /// Space bar.
    Space,
    /// Enter / return.
    Enter,
    /// Escape.
    Escape,
    /// Tab.
    Tab,
    /// Left shift.
    LShift,
    /// Right shift.
    RShift,
    /// Left control.
    LControl,
    /// Right control.
    RControl,
    /// Left alt.
    LAlt,
    /// Right alt.
    RAlt,
}

/// A mouse button.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum MouseButton {
    /// Primary button.
    Left,
    /// Middle button (often wheel click).
    Middle,
    /// Secondary button.
    Right,
    /// Backward navigation button.
    Back,
    /// Forward navigation button.
    Forward,
}

/// One raw device event, the only thing a platform backend has to produce.
#[derive(Debug, Clone, Copy, PartialEq)]
#[non_exhaustive]
pub enum DeviceEvent {
    /// Key was pressed (repeat presses are ignored by the state).
    KeyDown(KeyCode),
    /// Key was released.
    KeyUp(KeyCode),
    /// Mouse button was pressed.
    MouseDown(MouseButton),
    /// Mouse button was released.
    MouseUp(MouseButton),
    /// Cursor moved to window-space position `(x, y)`.
    MouseMoved {
        /// Horizontal window-space coordinate.
        x: f32,
        /// Vertical window-space coordinate.
        y: f32,
    },
    /// Wheel tick delta (positive = scroll up/away).
    MouseWheel(
        /// Scroll delta this event carries.
        f32,
    ),
}
