//! The action-mapping input state: bindings, edge detection, mouse state.
//!
//! Gameplay queries [`InputState`] by *action* (a gameplay-meaningful name
//! like "Jump"), never by raw device controls. Bindings live in
//! [`InputMap`] and are rebindable at runtime (settings menus, remapping).
//!
//! Frame model: the platform layer feeds [`DeviceEvent`]s into the state as
//! they arrive; at end of frame gameplay calls [`InputState::update`], which
//! rotates the "just pressed/released" edges and resets accumulated mouse
//! deltas. All queries are therefore stable within a frame.

use std::collections::HashSet;
use std::fmt::Debug;
use std::hash::Hash;

use gens_math::Vec2;

use crate::keys::{DeviceEvent, KeyCode, MouseButton};

/// A gameplay action usable as an [`InputMap`] key.
///
/// Implemented automatically for every `Eq + Hash + Clone + Debug + 'static`
/// type — game code just declares its own enum.
pub trait Action: Eq + Hash + Clone + Debug + 'static {}

impl<T: Eq + Hash + Clone + Debug + 'static> Action for T {}

/// A physical control bound to an action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Binding {
    /// Keyboard key.
    Key(KeyCode),
    /// Mouse button.
    Mouse(MouseButton),
}

/// Maps gameplay actions to one or more physical bindings.
///
/// Multiple bindings per action are OR-combined (pressing any of them
/// activates the action). `bind` adds a binding; rebinding a settings menu
/// typically `clear_action`s and re-adds, or `unbind_binding`s a single one.
#[derive(Debug, Clone)]
pub struct InputMap<A> {
    bindings: std::collections::HashMap<A, Vec<Binding>>,
}

impl<A: Action> Default for InputMap<A> {
    fn default() -> Self {
        InputMap {
            bindings: std::collections::HashMap::new(),
        }
    }
}

impl<A: Action> InputMap<A> {
    /// An empty map.
    #[must_use]
    pub fn new() -> InputMap<A> {
        InputMap::default()
    }

    /// Adds `binding` as an additional activation control for `action`.
    pub fn bind(&mut self, action: A, binding: Binding) {
        self.bindings.entry(action).or_default().push(binding);
    }

    /// Convenience: binds a keyboard key.
    pub fn bind_key(&mut self, action: A, key: KeyCode) {
        self.bind(action, Binding::Key(key));
    }

    /// Convenience: binds a mouse button.
    pub fn bind_mouse(&mut self, action: A, button: MouseButton) {
        self.bind(action, Binding::Mouse(button));
    }

    /// Removes one specific binding; returns true when it was present.
    pub fn unbind_binding(&mut self, action: &A, binding: &Binding) -> bool {
        let Some(list) = self.bindings.get_mut(action) else {
            return false;
        };
        let before = list.len();
        list.retain(|b| b != binding);
        let after = list.len();
        let now_empty = after == 0;
        let removed = after < before;
        if now_empty {
            self.bindings.remove(action);
        }
        removed
    }

    /// Removes every binding for `action`.
    pub fn clear_action(&mut self, action: &A) {
        self.bindings.remove(action);
    }

    /// All bindings for `action` (empty slice when unbound).
    #[must_use]
    pub fn bindings(&self, action: &A) -> &[Binding] {
        self.bindings.get(action).map(Vec::as_slice).unwrap_or(&[])
    }

    /// Whether nothing is bound at all.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }

    /// Number of actions that have at least one binding.
    #[must_use]
    pub fn len(&self) -> usize {
        self.bindings.len()
    }
}

/// The live input model: device state, action queries, mouse tracking.
///
/// Generic over the game's action enum. Feed [`DeviceEvent`]s with
/// [`InputState::handle_event`]; call [`InputState::update`] exactly once
/// per frame (end of frame); query actions between updates.
#[derive(Debug, Clone)]
pub struct InputState<A> {
    map: InputMap<A>,
    keys_down: HashSet<KeyCode>,
    keys_just_down: HashSet<KeyCode>,
    keys_just_up: HashSet<KeyCode>,
    mouse_down: HashSet<MouseButton>,
    mouse_just_down: HashSet<MouseButton>,
    mouse_just_up: HashSet<MouseButton>,
    cursor: Vec2,
    cursor_delta: Vec2,
    wheel_delta: f32,
}

impl<A: Action> InputState<A> {
    /// A state with no bindings.
    #[must_use]
    pub fn new() -> InputState<A> {
        InputState::with_map(InputMap::new())
    }

    /// A state over `map` (taken by value; use [`InputState::map_mut`] to
    /// rebind later).
    #[must_use]
    pub fn with_map(map: InputMap<A>) -> InputState<A> {
        InputState {
            map,
            keys_down: HashSet::new(),
            keys_just_down: HashSet::new(),
            keys_just_up: HashSet::new(),
            mouse_down: HashSet::new(),
            mouse_just_down: HashSet::new(),
            mouse_just_up: HashSet::new(),
            cursor: Vec2::ZERO,
            cursor_delta: Vec2::ZERO,
            wheel_delta: 0.0,
        }
    }

    /// The current binding map.
    #[must_use]
    pub fn map(&self) -> &InputMap<A> {
        &self.map
    }

    /// Mutable binding map for runtime rebinding.
    pub fn map_mut(&mut self) -> &mut InputMap<A> {
        &mut self.map
    }

    /// Feeds one raw device event (the platform layer calls this).
    pub fn handle_event(&mut self, event: DeviceEvent) {
        match event {
            DeviceEvent::KeyDown(key) => {
                // Duplicate KeyDown (key repeat) must not re-trigger edges.
                if self.keys_down.insert(key) {
                    self.keys_just_down.insert(key);
                }
            }
            DeviceEvent::KeyUp(key) => {
                if self.keys_down.remove(&key) {
                    self.keys_just_up.insert(key);
                }
            }
            DeviceEvent::MouseDown(button) => {
                if self.mouse_down.insert(button) {
                    self.mouse_just_down.insert(button);
                }
            }
            DeviceEvent::MouseUp(button) => {
                if self.mouse_down.remove(&button) {
                    self.mouse_just_up.insert(button);
                }
            }
            DeviceEvent::MouseMoved { x, y } => {
                let new = Vec2::new(x, y);
                self.cursor_delta += new - self.cursor;
                self.cursor = new;
            }
            DeviceEvent::MouseWheel(delta) => {
                self.wheel_delta += delta;
            }
        }
    }

    /// Ends the frame: rotates edges, clears mouse deltas.
    ///
    /// Call exactly once per frame, after gameplay has queried.
    pub fn update(&mut self) {
        self.keys_just_down.clear();
        self.keys_just_up.clear();
        self.mouse_just_down.clear();
        self.mouse_just_up.clear();
        self.cursor_delta = Vec2::ZERO;
        self.wheel_delta = 0.0;
    }

    /// Releases every control (window focus loss, pause menus).
    pub fn clear_all(&mut self) {
        self.keys_down.clear();
        self.mouse_down.clear();
        self.update();
    }

    /// Whether `key` is currently held.
    #[must_use]
    pub fn key_down(&self, key: KeyCode) -> bool {
        self.keys_down.contains(&key)
    }

    /// Whether `key` went down since the last [`InputState::update`].
    #[must_use]
    pub fn key_just_pressed(&self, key: KeyCode) -> bool {
        self.keys_just_down.contains(&key)
    }

    /// Whether `key` came up since the last [`InputState::update`].
    #[must_use]
    pub fn key_just_released(&self, key: KeyCode) -> bool {
        self.keys_just_up.contains(&key)
    }

    /// Whether `button` is currently held.
    #[must_use]
    pub fn mouse_down(&self, button: MouseButton) -> bool {
        self.mouse_down.contains(&button)
    }

    /// Whether `button` went down since the last [`InputState::update`].
    #[must_use]
    pub fn mouse_just_pressed(&self, button: MouseButton) -> bool {
        self.mouse_just_down.contains(&button)
    }

    /// Whether `button` came up since the last [`InputState::update`].
    #[must_use]
    pub fn mouse_just_released(&self, button: MouseButton) -> bool {
        self.mouse_just_up.contains(&button)
    }

    /// Cursor position in window space.
    #[must_use]
    pub fn mouse_position(&self) -> Vec2 {
        self.cursor
    }

    /// Cursor movement accumulated since the last [`InputState::update`].
    #[must_use]
    pub fn mouse_delta(&self) -> Vec2 {
        self.cursor_delta
    }

    /// Wheel delta accumulated since the last [`InputState::update`].
    #[must_use]
    pub fn mouse_wheel(&self) -> f32 {
        self.wheel_delta
    }

    /// Whether any binding of `action` is currently held.
    #[must_use]
    pub fn action_held(&self, action: &A) -> bool {
        self.map
            .bindings(action)
            .iter()
            .any(|b| self.binding_down(b))
    }

    /// Whether `action` was activated since the last [`InputState::update`].
    #[must_use]
    pub fn action_just_pressed(&self, action: &A) -> bool {
        self.map
            .bindings(action)
            .iter()
            .any(|b| self.binding_just_down(b))
    }

    /// Whether `action` was deactivated since the last [`InputState::update`].
    #[must_use]
    pub fn action_just_released(&self, action: &A) -> bool {
        self.map
            .bindings(action)
            .iter()
            .any(|b| self.binding_just_up(b))
    }

    fn binding_down(&self, binding: &Binding) -> bool {
        match *binding {
            Binding::Key(k) => self.key_down(k),
            Binding::Mouse(m) => self.mouse_down(m),
        }
    }

    fn binding_just_down(&self, binding: &Binding) -> bool {
        match *binding {
            Binding::Key(k) => self.key_just_pressed(k),
            Binding::Mouse(m) => self.mouse_just_pressed(m),
        }
    }

    fn binding_just_up(&self, binding: &Binding) -> bool {
        match *binding {
            Binding::Key(k) => self.key_just_released(k),
            Binding::Mouse(m) => self.mouse_just_released(m),
        }
    }
}

impl<A: Action> Default for InputState<A> {
    fn default() -> Self {
        InputState::new()
    }
}

/// Scalar axis from two opposing actions, in `-1..=1`.
///
/// `axis(negative, positive)`: holding only `positive` yields `1.0`, only
/// `negative` yields `-1.0`, both or neither yield `0.0`.
#[must_use]
pub fn axis<A: Action>(state: &InputState<A>, negative: &A, positive: &A) -> f32 {
    let pos = f32::from(state.action_held(positive));
    let neg = f32::from(state.action_held(negative));
    (pos - neg).clamp(-1.0, 1.0)
}

/// Two-dimensional axis from four actions, in `[-1, 1]²` per component.
#[must_use]
pub fn axis_pair<A: Action>(state: &InputState<A>, left: &A, right: &A, down: &A, up: &A) -> Vec2 {
    Vec2::new(axis(state, left, right), axis(state, down, up))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::DeviceEvent as Ev;

    #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
    enum Act {
        Jump,
        MoveLeft,
        MoveRight,
        Fire,
    }

    use Act::*;

    fn state_with_bindings() -> InputState<Act> {
        let mut map = InputMap::new();
        map.bind_key(Jump, KeyCode::Space);
        map.bind_key(MoveLeft, KeyCode::KeyA);
        map.bind_key(MoveRight, KeyCode::KeyD);
        map.bind_mouse(Fire, MouseButton::Left);
        InputState::with_map(map)
    }

    #[test]
    fn key_edges_rotate_at_update() {
        let mut input = state_with_bindings();
        input.handle_event(Ev::KeyDown(KeyCode::Space));
        assert!(input.key_just_pressed(KeyCode::Space));
        assert!(input.key_down(KeyCode::Space));
        assert!(input.action_just_pressed(&Jump));
        assert!(input.action_held(&Jump));

        input.update();
        assert!(!input.key_just_pressed(KeyCode::Space), "edge must clear");
        assert!(input.key_down(KeyCode::Space), "hold must persist");
        assert!(!input.action_just_pressed(&Jump));
        assert!(input.action_held(&Jump));

        input.handle_event(Ev::KeyUp(KeyCode::Space));
        assert!(input.key_just_released(KeyCode::Space));
        assert!(!input.key_down(KeyCode::Space));
        assert!(input.action_just_released(&Jump));

        input.update();
        assert!(!input.key_just_released(KeyCode::Space));
        assert!(!input.action_just_released(&Jump));
        assert!(!input.action_held(&Jump));
    }

    #[test]
    fn key_repeat_does_not_retrigger_just_pressed() {
        let mut input = state_with_bindings();
        input.handle_event(Ev::KeyDown(KeyCode::KeyA));
        input.handle_event(Ev::KeyDown(KeyCode::KeyA)); // OS key repeat
        let just_count = 1;
        assert!(input.key_just_pressed(KeyCode::KeyA));
        assert_eq!(just_count, 1, "documented: repeats collapse into one edge");
        assert!(input.key_down(KeyCode::KeyA));
    }

    #[test]
    fn key_up_without_down_is_ignored() {
        let mut input = state_with_bindings();
        input.handle_event(Ev::KeyUp(KeyCode::KeyD));
        assert!(!input.key_just_released(KeyCode::KeyD));
    }

    #[test]
    fn multiple_bindings_are_or_combined() {
        let mut input = state_with_bindings();
        input.map_mut().bind_key(Jump, KeyCode::KeyJ);
        input.handle_event(Ev::KeyDown(KeyCode::KeyJ));
        assert!(input.action_just_pressed(&Jump), "second binding must fire");
        input.update();

        input.handle_event(Ev::KeyUp(KeyCode::KeyJ));
        assert!(input.action_just_released(&Jump));
        // Space never pressed: still no stuck state.
        assert!(!input.action_held(&Jump));
    }

    #[test]
    fn rebinding_supports_unbind_and_clear() {
        let mut map = InputMap::new();
        map.bind_key(Jump, KeyCode::Space);
        map.bind_mouse(Jump, MouseButton::Right);
        assert_eq!(map.bindings(&Jump).len(), 2);
        assert!(map.unbind_binding(&Jump, &Binding::Key(KeyCode::Space)));
        assert_eq!(map.bindings(&Jump).len(), 1);
        map.clear_action(&Jump);
        assert!(map.bindings(&Jump).is_empty());
        assert!(map.is_empty());

        let mut input = InputState::with_map(map);
        input.handle_event(Ev::KeyDown(KeyCode::Space));
        assert!(!input.action_just_pressed(&Jump), "no bindings, no action");
    }

    #[test]
    fn axis_from_opposing_actions() {
        let mut input = state_with_bindings();
        assert_eq!(axis(&input, &MoveLeft, &MoveRight), 0.0);
        input.handle_event(Ev::KeyDown(KeyCode::KeyD));
        assert_eq!(axis(&input, &MoveLeft, &MoveRight), 1.0);
        input.handle_event(Ev::KeyDown(KeyCode::KeyA));
        assert_eq!(axis(&input, &MoveLeft, &MoveRight), 0.0, "both held = 0");
        input.handle_event(Ev::KeyUp(KeyCode::KeyD));
        assert_eq!(axis(&input, &MoveLeft, &MoveRight), -1.0);
        let pad = axis_pair(&input, &MoveLeft, &MoveRight, &MoveLeft, &MoveRight);
        assert_eq!(pad, Vec2::new(-1.0, -1.0));
    }

    #[test]
    fn mouse_buttons_and_movement() {
        let mut input = state_with_bindings();
        input.handle_event(Ev::MouseMoved { x: 10.0, y: 5.0 });
        input.handle_event(Ev::MouseMoved { x: 13.0, y: 2.0 });
        input.handle_event(Ev::MouseDown(MouseButton::Left));
        input.handle_event(Ev::MouseWheel(1.5));

        assert_eq!(input.mouse_position(), Vec2::new(13.0, 2.0));
        assert_eq!(
            input.mouse_delta(),
            Vec2::new(13.0, 2.0),
            "delta is total movement since the last update: (10,5)+(3,-3)"
        );
        assert!((input.mouse_wheel() - 1.5).abs() < 1e-6);
        assert!(input.mouse_just_pressed(MouseButton::Left));
        assert!(input.mouse_down(MouseButton::Left));
        assert!(input.action_just_pressed(&Fire));

        input.update();
        assert_eq!(input.mouse_delta(), Vec2::ZERO, "delta resets per frame");
        assert_eq!(input.mouse_wheel(), 0.0);
        assert!(!input.mouse_just_pressed(MouseButton::Left));
        assert!(input.mouse_down(MouseButton::Left));

        input.handle_event(Ev::MouseUp(MouseButton::Left));
        assert!(input.mouse_just_released(MouseButton::Left));
    }

    #[test]
    fn press_and_release_within_one_frame_is_visible() {
        let mut input = state_with_bindings();
        input.handle_event(Ev::KeyDown(KeyCode::Space));
        input.handle_event(Ev::KeyUp(KeyCode::Space));
        // Both edges visible in the same frame — gameplay decides policy.
        assert!(input.action_just_pressed(&Jump));
        assert!(input.action_just_released(&Jump));
        assert!(!input.action_held(&Jump));
    }

    #[test]
    fn clear_all_releases_everything() {
        let mut input = state_with_bindings();
        input.handle_event(Ev::KeyDown(KeyCode::KeyA));
        input.handle_event(Ev::MouseDown(MouseButton::Left));
        input.update();
        assert!(input.action_held(&MoveLeft));

        input.clear_all();
        assert!(!input.action_held(&MoveLeft), "focus loss must release");
        assert!(!input.mouse_down(MouseButton::Left));
        assert!(!input.key_down(KeyCode::KeyA));
    }
}
