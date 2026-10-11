//! Type-safe event bus (Phase 1).
//!
//! Design goals and contract:
//! - **Type-safe**: events are addressed by Rust type, never by string id.
//! - **Zero-cost**: no allocation per event beyond the backing `Vec`, no
//!   dynamic dispatch per event — the registry lookup happens once per
//!   frame per event type, then everything is monomorphized.
//! - **Double-buffered**: `send` writes into the *next* queue while `drain`
//!   is consuming the *current* queue, so an event emitted while handling
//!   events is readable on the next drain — never this one (no re-entrancy
//!   surprises), never lost.
//!
//! The bus lives outside the ECS (engine-level events: lifecycle, input,
//! audio commands). ECS-internal messaging will use gens-ecs facilities.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Marker trait for event payloads. Any `'static + Send + Sync` type qualifies.
pub trait Event: Any + Send + Sync {}
impl<T: Any + Send + Sync> Event for T {}

/// Double-buffered queue for one event type.
///
/// Typical frame flow: systems `send` into it; exactly one consumer `drain`s
/// it per frame (drain order == send order).
#[derive(Debug, Default)]
pub struct Events<E: Event> {
    current: Vec<E>,
    next: Vec<E>,
}

impl<E: Event> Events<E> {
    /// Empty queue.
    pub fn new() -> Events<E> {
        Events {
            current: Vec::new(),
            next: Vec::new(),
        }
    }

    /// Queues an event. Visible to the *next* drain (including if that drain
    /// happens after this frame's).
    pub fn send(&mut self, event: E) {
        self.next.push(event);
    }

    /// Number of events waiting in the current queue.
    pub fn pending(&self) -> usize {
        self.current.len()
    }

    /// True when the current queue is empty.
    pub fn is_empty(&self) -> bool {
        self.current.is_empty()
    }

    /// Drains the current queue in send order. Events sent during the drain
    /// land in the next queue and become readable on the next call. The
    /// returned iterator empties the queue lazily in O(n) total.
    pub fn drain(&mut self) -> Drain<E> {
        // Promote next -> current: what was queued since the last drain is
        // exactly what this drain must deliver, in send order.
        std::mem::swap(&mut self.current, &mut self.next);
        self.next.clear();
        let mut pending = std::mem::take(&mut self.current);
        pending.reverse(); // pop() then yields in original send order
        Drain { inner: pending }
    }
}

/// Iterator returned by [`Events::drain`].
pub struct Drain<E: Event> {
    inner: Vec<E>, // reversed: pop() yields send order
}

impl<E: Event> Iterator for Drain<E> {
    type Item = E;

    fn next(&mut self) -> Option<E> {
        self.inner.pop()
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.inner.len(), Some(self.inner.len()))
    }
}

impl<E: Event> Drop for Drain<E> {
    fn drop(&mut self) {
        // Retire un-consumed events explicitly; the next drain must not see them.
        self.inner.clear();
    }
}

/// Registry mapping event types to their [`Events`] queues.
///
/// One lookup per (frame, event type); all per-event work is generic and
/// monomorphized.
#[derive(Default)]
pub struct EventRegistry {
    queues: HashMap<TypeId, Box<dyn Any + Send + Sync>>,
}

impl EventRegistry {
    /// New, empty registry.
    pub fn new() -> EventRegistry {
        EventRegistry {
            queues: HashMap::new(),
        }
    }

    /// Explicitly creates the queue for `E` (idempotent).
    pub fn register<E: Event>(&mut self) {
        self.queues
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(Events::<E>::new()));
    }

    /// Typed access, auto-registering on first use.
    pub fn get_mut<E: Event>(&mut self) -> &mut Events<E> {
        self.queues
            .entry(TypeId::of::<E>())
            .or_insert_with(|| Box::new(Events::<E>::new()))
            .downcast_mut::<Events<E>>()
            .expect("queue for this TypeId is always Events<E>")
    }

    /// Convenience: queue an event of type `E`.
    pub fn send<E: Event>(&mut self, event: E) {
        self.get_mut::<E>().send(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq, Eq)]
    struct Damage {
        target: u32,
        amount: i32,
    }

    #[derive(Debug, PartialEq, Eq)]
    struct Unrelated(u8);

    #[test]
    fn drain_preserves_send_order_and_empties_queue() {
        let mut events = Events::<Damage>::new();
        events.send(Damage {
            target: 1,
            amount: 5,
        });
        events.send(Damage {
            target: 2,
            amount: 7,
        });
        events.send(Damage {
            target: 3,
            amount: 9,
        });
        assert_eq!(events.pending(), 3);

        let drained: Vec<Damage> = events.drain().collect();
        assert_eq!(
            drained,
            vec![
                Damage {
                    target: 1,
                    amount: 5
                },
                Damage {
                    target: 2,
                    amount: 7
                },
                Damage {
                    target: 3,
                    amount: 9
                }
            ]
        );
        assert_eq!(events.pending(), 0);
        // A second drain delivers nothing new.
        assert_eq!(events.drain().count(), 0);
    }

    #[test]
    fn sends_during_drain_land_next_frame() {
        let mut events = Events::<u32>::new();
        events.send(1);
        let mut drained = events.drain();
        assert_eq!(drained.next(), Some(1));
        // Consumer sends a follow-up while still draining.
        events.send(2);
        drop(drained);
        // This frame's queue is done...
        assert_eq!(events.drain().count(), 0);
        // ...the follow-up arrives on the next drain.
        let next: Vec<u32> = events.drain().collect();
        assert_eq!(next, vec![2]);
    }

    #[test]
    fn partially_consumed_drain_does_not_leak_into_next_frame() {
        let mut events = Events::<u8>::new();
        for i in 0..10 {
            events.send(i);
        }
        let mut drain = events.drain();
        assert_eq!(drain.next(), Some(0));
        drop(drain); // abandon after 1 of 10
        assert_eq!(events.drain().count(), 0); // rest retired, not re-delivered
    }

    #[test]
    fn registry_isolates_event_types() {
        let mut registry = EventRegistry::new();
        registry.send(Damage {
            target: 1,
            amount: 5,
        });
        registry.send(Unrelated(9));
        registry.send(Unrelated(8));

        assert_eq!(registry.get_mut::<Damage>().pending(), 1);
        assert_eq!(registry.get_mut::<Unrelated>().pending(), 2);
        assert_eq!(
            registry
                .get_mut::<Damage>()
                .drain()
                .collect::<Vec<Damage>>()[0]
                .amount,
            5
        );
    }

    #[test]
    fn registry_register_is_idempotent_and_send_is_convenient() {
        let mut registry = EventRegistry::new();
        registry.register::<Damage>();
        registry.register::<Damage>();
        registry.send(Damage {
            target: 7,
            amount: 1,
        });
        assert_eq!(registry.get_mut::<Damage>().pending(), 1);
    }

    #[test]
    fn events_default_and_empty_state() {
        let mut events = Events::<Unrelated>::new();
        assert!(events.is_empty());
        assert_eq!(events.pending(), 0);
        events.send(Unrelated(1));
        assert!(!events.is_empty());
    }
}
