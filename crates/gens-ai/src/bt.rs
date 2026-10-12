//! Behavior trees: status codes, blackboard and node set.
//!
//! A minimal, deterministic behavior-tree core:
//!
//! - [`NodeStatus`] — the classic Success / Failure / Running triple;
//! - [`Blackboard`] — the tree's shared memory (ordered map, typed
//!   values), so agent state is inspectable and replay-friendly;
//! - [`Behavior`] — the node trait (`tick` + `reset`);
//! - [`nodes`] — composites (Sequence, Selector, Parallel), decorators
//!   (Inverter, Repeat), leaves (Action, Condition, Wait).
//!
//! Semantics pinned here:
//!
//! - composites are **memory-based**: a Sequence/Selector that gets
//!   `Running` resumes at the same child on the next tick (classic BT
//!   memory semantics; a *reactive* restart is one explicit `reset`
//!   call away);
//! - iteration order is declaration order, the blackboard is an ordered
//!   map — same tick sequence, same statuses, every replay (R-013).

use std::collections::BTreeMap;

/// Result of ticking one behavior node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeStatus {
    /// The node finished successfully.
    Success,
    /// The node finished unsuccessfully.
    Failure,
    /// The node needs more ticks to finish.
    Running,
}

/// The shared memory a behavior tree reads and writes while ticking.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Blackboard {
    values: BTreeMap<String, BlackboardValue>,
}

/// A blackboard entry value.
#[derive(Debug, Clone, PartialEq)]
pub enum BlackboardValue {
    /// Floating-point quantity (distances, timers, scores...).
    Number(f32),
    /// Boolean fact (has target, in range...).
    Flag(bool),
    /// Opaque label (current target name, state tag...).
    Text(String),
}

impl Blackboard {
    /// An empty blackboard.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Writes a value, replacing any previous entry.
    pub fn set(&mut self, key: impl Into<String>, value: BlackboardValue) {
        self.values.insert(key.into(), value);
    }

    /// Reads a number, if present and numeric.
    #[must_use]
    pub fn number(&self, key: &str) -> Option<f32> {
        match self.values.get(key) {
            Some(BlackboardValue::Number(n)) => Some(*n),
            _ => None,
        }
    }

    /// Reads a flag, if present and boolean.
    #[must_use]
    pub fn flag(&self, key: &str) -> Option<bool> {
        match self.values.get(key) {
            Some(BlackboardValue::Flag(b)) => Some(*b),
            _ => None,
        }
    }

    /// Reads a text entry, if present.
    #[must_use]
    pub fn text(&self, key: &str) -> Option<&str> {
        match self.values.get(key) {
            Some(BlackboardValue::Text(s)) => Some(s),
            _ => None,
        }
    }

    /// Removes an entry, returning the old value.
    pub fn clear(&mut self, key: &str) -> Option<BlackboardValue> {
        self.values.remove(key)
    }

    /// `true` when the key exists (any type).
    #[must_use]
    pub fn contains(&self, key: &str) -> bool {
        self.values.contains_key(key)
    }

    /// Number of entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// `true` when the blackboard has no entries.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }
}

/// One tickable node of a behavior tree.
pub trait Behavior {
    /// Advances the node by one tick (`dt` in seconds).
    fn tick(&mut self, blackboard: &mut Blackboard, dt: f32) -> NodeStatus;

    /// Returns the node to its untouched state (composite cursors,
    /// timers, repeat counters). A tree reset between episodes keeps
    /// replays aligned.
    fn reset(&mut self);
}

/// Convenience alias for boxed behavior nodes.
pub type BoxedBehavior = Box<dyn Behavior>;

/// The reusable node set (`Sequence`, `Selector`, `Action`, ...).
pub mod nodes {
    use super::{Behavior, Blackboard, BoxedBehavior, NodeStatus};

    /// Runs children in order until one fails; succeeds when all succeed.
    ///
    /// Memory-based: resumes at the child that was `Running`.
    #[derive(Default)]
    pub struct Sequence {
        children: Vec<BoxedBehavior>,
        cursor: usize,
    }

    impl Sequence {
        /// Builds a sequence from the given children.
        #[must_use]
        pub fn new(children: Vec<BoxedBehavior>) -> Self {
            Self {
                children,
                cursor: 0,
            }
        }
    }

    impl Behavior for Sequence {
        fn tick(&mut self, blackboard: &mut Blackboard, dt: f32) -> NodeStatus {
            while self.cursor < self.children.len() {
                match self.children[self.cursor].tick(blackboard, dt) {
                    NodeStatus::Success => self.cursor += 1,
                    NodeStatus::Running => return NodeStatus::Running,
                    NodeStatus::Failure => {
                        self.cursor = 0;
                        return NodeStatus::Failure;
                    }
                }
            }
            self.cursor = 0;
            NodeStatus::Success
        }

        fn reset(&mut self) {
            self.cursor = 0;
            for child in &mut self.children {
                child.reset();
            }
        }
    }

    /// Tries children in order until one succeeds; fails when all fail.
    ///
    /// Memory-based: resumes at the child that was `Running`.
    #[derive(Default)]
    pub struct Selector {
        children: Vec<BoxedBehavior>,
        cursor: usize,
    }

    impl Selector {
        /// Builds a fallback from the given children.
        #[must_use]
        pub fn new(children: Vec<BoxedBehavior>) -> Self {
            Self {
                children,
                cursor: 0,
            }
        }
    }

    impl Behavior for Selector {
        fn tick(&mut self, blackboard: &mut Blackboard, dt: f32) -> NodeStatus {
            while self.cursor < self.children.len() {
                match self.children[self.cursor].tick(blackboard, dt) {
                    NodeStatus::Success => {
                        self.cursor = 0;
                        return NodeStatus::Success;
                    }
                    NodeStatus::Running => return NodeStatus::Running,
                    NodeStatus::Failure => self.cursor += 1,
                }
            }
            self.cursor = 0;
            NodeStatus::Failure
        }

        fn reset(&mut self) {
            self.cursor = 0;
            for child in &mut self.children {
                child.reset();
            }
        }
    }

    /// How [`Parallel`] combines child results.
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub enum SuccessPolicy {
        /// Succeed when **all** children succeeded (default).
        #[default]
        All,
        /// Succeed when **any** child succeeded; the rest keep ticking.
        Any,
    }

    /// Ticks every child on every visit; combines with a policy.
    #[derive(Default)]
    pub struct Parallel {
        children: Vec<BoxedBehavior>,
        policy: SuccessPolicy,
    }

    impl Parallel {
        /// Builds a parallel node.
        #[must_use]
        pub fn new(children: Vec<BoxedBehavior>, policy: SuccessPolicy) -> Self {
            Self { children, policy }
        }
    }

    impl Behavior for Parallel {
        fn tick(&mut self, blackboard: &mut Blackboard, dt: f32) -> NodeStatus {
            let mut successes = 0usize;
            for child in &mut self.children {
                if child.tick(blackboard, dt) == NodeStatus::Success {
                    successes += 1;
                }
            }
            match self.policy {
                SuccessPolicy::All => {
                    if successes == self.children.len() {
                        NodeStatus::Success
                    } else {
                        NodeStatus::Running
                    }
                }
                SuccessPolicy::Any => {
                    if successes > 0 {
                        NodeStatus::Success
                    } else {
                        NodeStatus::Running
                    }
                }
            }
        }

        fn reset(&mut self) {
            for child in &mut self.children {
                child.reset();
            }
        }
    }

    /// Flips Success <-> Failure; Running passes through.
    pub struct Inverter {
        child: BoxedBehavior,
    }

    impl Inverter {
        /// Wraps `child`.
        #[must_use]
        pub fn new(child: BoxedBehavior) -> Self {
            Self { child }
        }
    }

    impl Behavior for Inverter {
        fn tick(&mut self, blackboard: &mut Blackboard, dt: f32) -> NodeStatus {
            match self.child.tick(blackboard, dt) {
                NodeStatus::Success => NodeStatus::Failure,
                NodeStatus::Failure => NodeStatus::Success,
                NodeStatus::Running => NodeStatus::Running,
            }
        }

        fn reset(&mut self) {
            self.child.reset();
        }
    }

    /// Re-runs the child `count` times per activation, then succeeds.
    ///
    /// A child failure aborts the repeat with `Failure`.
    pub struct Repeat {
        child: BoxedBehavior,
        count: u32,
        done: u32,
    }

    impl Repeat {
        /// Repeats the child exactly `count` times.
        #[must_use]
        pub fn times(child: BoxedBehavior, count: u32) -> Self {
            Self {
                child,
                count,
                done: 0,
            }
        }
    }

    impl Behavior for Repeat {
        fn tick(&mut self, blackboard: &mut Blackboard, dt: f32) -> NodeStatus {
            while self.done < self.count {
                match self.child.tick(blackboard, dt) {
                    NodeStatus::Success => self.done += 1,
                    NodeStatus::Running => return NodeStatus::Running,
                    NodeStatus::Failure => {
                        self.done = 0;
                        return NodeStatus::Failure;
                    }
                }
            }
            self.done = 0;
            NodeStatus::Success
        }

        fn reset(&mut self) {
            self.done = 0;
            self.child.reset();
        }
    }

    /// Runs for `duration` seconds (accumulated across ticks), then succeeds.
    pub struct Wait {
        duration: f32,
        elapsed: f32,
    }

    impl Wait {
        /// Waits `duration` seconds of accumulated tick time.
        #[must_use]
        pub fn for_seconds(duration: f32) -> Self {
            Self {
                duration,
                elapsed: 0.0,
            }
        }

        /// Elapsed time so far (diagnostics and tests).
        #[must_use]
        pub fn elapsed(&self) -> f32 {
            self.elapsed
        }
    }

    impl Behavior for Wait {
        fn tick(&mut self, _blackboard: &mut Blackboard, dt: f32) -> NodeStatus {
            self.elapsed += dt;
            if self.elapsed >= self.duration {
                NodeStatus::Success
            } else {
                NodeStatus::Running
            }
        }

        fn reset(&mut self) {
            self.elapsed = 0.0;
        }
    }

    /// Leaf that runs a closure every tick.
    pub struct Action<F>
    where
        F: FnMut(&mut Blackboard, f32) -> NodeStatus,
    {
        function: F,
    }

    impl<F> Action<F>
    where
        F: FnMut(&mut Blackboard, f32) -> NodeStatus,
    {
        /// Wraps a stateful closure as a leaf node.
        #[must_use]
        pub fn new(function: F) -> Self {
            Self { function }
        }
    }

    impl<F> Behavior for Action<F>
    where
        F: FnMut(&mut Blackboard, f32) -> NodeStatus,
    {
        fn tick(&mut self, blackboard: &mut Blackboard, dt: f32) -> NodeStatus {
            (self.function)(blackboard, dt)
        }

        fn reset(&mut self) {}
    }

    /// Leaf that maps a blackboard predicate to Success/Failure.
    pub struct Condition<F>
    where
        F: Fn(&Blackboard) -> bool,
    {
        function: F,
    }

    impl<F> Condition<F>
    where
        F: Fn(&Blackboard) -> bool,
    {
        /// Wraps a read-only predicate as a leaf node.
        #[must_use]
        pub fn new(function: F) -> Self {
            Self { function }
        }
    }

    impl<F> Behavior for Condition<F>
    where
        F: Fn(&Blackboard) -> bool,
    {
        fn tick(&mut self, blackboard: &mut Blackboard, _dt: f32) -> NodeStatus {
            if (self.function)(blackboard) {
                NodeStatus::Success
            } else {
                NodeStatus::Failure
            }
        }

        fn reset(&mut self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nodes::*;

    /// Returns a fixed status every tick - the workhorse probe.
    fn probe(status: NodeStatus) -> BoxedBehavior {
        Box::new(Action::new(move |_, _| status))
    }

    #[test]
    fn blackboard_typed_round_trip() {
        let mut bb = Blackboard::new();
        bb.set("speed", BlackboardValue::Number(3.5));
        bb.set("grounded", BlackboardValue::Flag(true));
        bb.set("target", BlackboardValue::Text("tower".to_owned()));
        assert_eq!(bb.number("speed"), Some(3.5));
        assert_eq!(bb.flag("grounded"), Some(true));
        assert_eq!(bb.text("target"), Some("tower"));
        assert_eq!(bb.number("grounded"), None, "type mismatch reads as None");
        assert_eq!(bb.number("missing"), None);
        assert!(bb.contains("target"));
        assert_eq!(bb.len(), 3);
        assert!(bb.clear("target").is_some());
        assert!(!bb.contains("target"));
    }

    #[test]
    fn sequence_succeeds_only_when_all_children_succeed() {
        let mut bb = Blackboard::new();
        let mut s = Sequence::new(vec![probe(NodeStatus::Success), probe(NodeStatus::Success)]);
        assert_eq!(s.tick(&mut bb, 0.016), NodeStatus::Success);

        let mut s = Sequence::new(vec![probe(NodeStatus::Success), probe(NodeStatus::Failure)]);
        assert_eq!(s.tick(&mut bb, 0.016), NodeStatus::Failure);
    }

    #[test]
    fn sequence_has_memory_and_resumes_at_running_child() {
        let mut bb = Blackboard::new();
        // Child 1 succeeds and COUNTS its ticks: memory semantics mean it
        // must tick exactly once even though the sequence takes two ticks.
        let first_ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let f1 = first_ticks.clone();
        let first: BoxedBehavior = Box::new(Action::new(move |_, _| {
            f1.set(f1.get() + 1);
            NodeStatus::Success
        }));
        // Second child runs for two ticks, then succeeds.
        let ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let t2 = ticks.clone();
        let second: BoxedBehavior = Box::new(Action::new(move |_, _| {
            let n = t2.get() + 1;
            t2.set(n);
            if n >= 2 {
                NodeStatus::Success
            } else {
                NodeStatus::Running
            }
        }));
        let mut s = Sequence::new(vec![first, second]);

        assert_eq!(
            s.tick(&mut bb, 0.016),
            NodeStatus::Running,
            "child 2 running"
        );
        assert_eq!(s.tick(&mut bb, 0.016), NodeStatus::Success, "child 2 done");
        assert_eq!(ticks.get(), 2, "second child ticked exactly twice");
        assert_eq!(
            first_ticks.get(),
            1,
            "memory composites must NOT re-tick finished children"
        );
    }

    #[test]
    fn selector_takes_first_success_and_stops() {
        let mut bb = Blackboard::new();
        // A failing first child falls through to the succeeding second;
        // a third child must not be able to change the outcome.
        let mut sel = Selector::new(vec![
            probe(NodeStatus::Failure),
            probe(NodeStatus::Success),
            probe(NodeStatus::Failure),
        ]);
        assert_eq!(sel.tick(&mut bb, 0.016), NodeStatus::Success);
    }

    #[test]
    fn selector_fails_when_every_child_fails() {
        let mut bb = Blackboard::new();
        let mut sel = Selector::new(vec![probe(NodeStatus::Failure), probe(NodeStatus::Failure)]);
        assert_eq!(sel.tick(&mut bb, 0.016), NodeStatus::Failure);
    }

    #[test]
    fn inverter_flips_status_but_passes_running() {
        let mut bb = Blackboard::new();
        let mut inv = Inverter::new(probe(NodeStatus::Success));
        assert_eq!(inv.tick(&mut bb, 0.016), NodeStatus::Failure);
        let mut inv = Inverter::new(probe(NodeStatus::Failure));
        assert_eq!(inv.tick(&mut bb, 0.016), NodeStatus::Success);
        let mut inv = Inverter::new(probe(NodeStatus::Running));
        assert_eq!(inv.tick(&mut bb, 0.016), NodeStatus::Running);
    }

    #[test]
    fn repeat_runs_child_n_times_then_succeeds() {
        let mut bb = Blackboard::new();
        let ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let t = ticks.clone();
        let mut rep = Repeat::times(
            Box::new(Action::new(move |_, _| {
                t.set(t.get() + 1);
                NodeStatus::Success
            })),
            3,
        );
        assert_eq!(rep.tick(&mut bb, 0.016), NodeStatus::Success);
        assert_eq!(ticks.get(), 3);
    }

    #[test]
    fn wait_accumulates_dt_until_duration() {
        let mut bb = Blackboard::new();
        let mut w = Wait::for_seconds(0.5);
        assert_eq!(w.tick(&mut bb, 0.2), NodeStatus::Running);
        assert_eq!(w.tick(&mut bb, 0.2), NodeStatus::Running);
        assert!((w.elapsed() - 0.4).abs() < 1e-6);
        assert_eq!(w.tick(&mut bb, 0.1), NodeStatus::Success);
        w.reset();
        assert_eq!(w.elapsed(), 0.0);
    }

    #[test]
    fn parallel_policy_all_and_any() {
        let mut bb = Blackboard::new();
        let mut all = Parallel::new(
            vec![probe(NodeStatus::Success), probe(NodeStatus::Running)],
            SuccessPolicy::All,
        );
        assert_eq!(all.tick(&mut bb, 0.016), NodeStatus::Running);

        let mut any = Parallel::new(
            vec![probe(NodeStatus::Success), probe(NodeStatus::Running)],
            SuccessPolicy::Any,
        );
        assert_eq!(any.tick(&mut bb, 0.016), NodeStatus::Success);
    }

    #[test]
    fn condition_maps_blackboard_predicate() {
        let mut bb = Blackboard::new();
        let mut near =
            Condition::new(|bb: &Blackboard| bb.number("distance").is_some_and(|d| d < 2.0));
        bb.set("distance", BlackboardValue::Number(5.0));
        assert_eq!(near.tick(&mut bb, 0.016), NodeStatus::Failure);
        bb.set("distance", BlackboardValue::Number(1.0));
        assert_eq!(near.tick(&mut bb, 0.016), NodeStatus::Success);
    }

    #[test]
    fn action_writes_blackboard_for_downstream_nodes() {
        let mut bb = Blackboard::new();
        let mut pick = Sequence::new(vec![
            Box::new(Action::new(|bb, _| {
                bb.set("target", BlackboardValue::Text("well".to_owned()));
                NodeStatus::Success
            })),
            Box::new(Condition::new(|bb| bb.text("target") == Some("well"))),
        ]);
        assert_eq!(pick.tick(&mut bb, 0.016), NodeStatus::Success);
    }

    #[test]
    fn guard_patrol_scenario_selector_with_memory() {
        // selector[ sequence[ enemy_visible -> attack ],
        //           sequence[ waypoint_set -> patrol ] ]
        let mut bb = Blackboard::new();
        bb.set("enemy_visible", BlackboardValue::Flag(false));
        bb.set("waypoint", BlackboardValue::Text("gate".to_owned()));

        let attack_ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let patrol_ticks = std::rc::Rc::new(std::cell::Cell::new(0u32));
        let attack_seen = attack_ticks.clone();
        let patrol_seen = patrol_ticks.clone();

        let attack_branch = Sequence::new(vec![
            Box::new(Condition::new(|bb| {
                bb.flag("enemy_visible").unwrap_or(false)
            })),
            Box::new(Action::new(move |_, _| {
                attack_seen.set(attack_seen.get() + 1);
                NodeStatus::Success
            })),
        ]);
        let patrol_branch = Sequence::new(vec![
            Box::new(Condition::new(|bb| bb.contains("waypoint"))),
            Box::new(Action::new(move |bb, _| {
                patrol_seen.set(patrol_seen.get() + 1);
                bb.set("waypoint", BlackboardValue::Text("tower".to_owned()));
                NodeStatus::Success
            })),
        ]);
        let mut tree = Selector::new(vec![Box::new(attack_branch), Box::new(patrol_branch)]);

        // No enemy: patrol branch fires.
        assert_eq!(tree.tick(&mut bb, 0.016), NodeStatus::Success);
        assert_eq!(attack_ticks.get(), 0);
        assert_eq!(patrol_ticks.get(), 1);

        // Enemy appears: attack branch wins.
        bb.set("enemy_visible", BlackboardValue::Flag(true));
        assert_eq!(tree.tick(&mut bb, 0.016), NodeStatus::Success);
        assert_eq!(attack_ticks.get(), 1);
        assert_eq!(patrol_ticks.get(), 1);
    }

    #[test]
    fn same_tree_and_blackboard_replays_identically() {
        let build = || {
            let mut bb = Blackboard::new();
            bb.set("fuel", BlackboardValue::Number(2.0));
            let mut tree = Sequence::new(vec![
                Box::new(Condition::new(|bb| {
                    bb.number("fuel").is_some_and(|f| f > 0.0)
                })),
                Box::new(Action::new(|bb, _| {
                    let f = bb.number("fuel").unwrap_or(0.0) - 0.5;
                    bb.set("fuel", BlackboardValue::Number(f));
                    if f > 0.0 {
                        NodeStatus::Running
                    } else {
                        NodeStatus::Success
                    }
                })),
            ]);
            let mut trace = Vec::new();
            for _ in 0..6 {
                let status = tree.tick(&mut bb, 0.016);
                let code = match status {
                    NodeStatus::Success => 1u32,
                    NodeStatus::Failure => 2,
                    NodeStatus::Running => 3,
                };
                trace.push(code);
                trace.push(bb.number("fuel").unwrap_or(0.0).to_bits());
            }
            trace
        };
        assert_eq!(
            build(),
            build(),
            "identical tick sequences must replay identically"
        );
    }
}
