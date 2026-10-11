//! Navigation, pathfinding and behavior trees (Phase 4).
//!
//! `gens-ai` is a **pure, deterministic AI core**:
//!
//! - [`bt`] — behavior trees: a status trio, an ordered [`bt::Blackboard`]
//!   as the tree's shared memory, and the classic node set (Sequence,
//!   Selector, Parallel, Inverter, Repeat, Wait, Action, Condition)
//!   with memory-based composites.
//!
//! Grid navigation + A* + steering land in the next sub-item, staying
//! in this crate and staying pure.
//!
//! Everything is a function of (state, tick): no RNG, no wall clocks,
//! no hidden ordering (ordered maps + declaration-order iteration), so
//! identical tick sequences replay identically within a process (see
//! `docs/RISKS.md` R-013). Mesh navigation and hierarchical pathing are
//! future work built on the same contracts.

pub mod bt;

pub use bt::{Behavior, Blackboard, BlackboardValue, BoxedBehavior, NodeStatus};
