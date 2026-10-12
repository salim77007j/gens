//! Allocator-level memory tracking (P8-2).
//!
//! [`TrackingAllocator`] wraps [`System`](std::alloc::System) and updates
//! [`STATS`] on every allocation, reallocation and deallocation: current
//! and peak live bytes, allocation counts, and lifetime totals. Linked in,
//! it becomes the process `#[global_allocator]` — the honest way to obtain
//! the reference design's "Memory" metric headless, because it counts what
//! the process actually allocates rather than estimating.
//!
//! Overhead (ADR-0027 / R-030): two atomic updates per allocation
//! (`fetch_add` + `fetch_max`), none per read. Measurement never changes
//! *what* is allocated — byte-identical replay contracts are untouched.
//!
//! Embedder note: because the allocator is installed on link, an
//! application that needs its own global allocator must not link
//! `gens-profiler` binaries-side (a workspace `#[global_allocator]`
//! conflict is a compile error, i.e. loud, not silent).

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

/// Process-wide allocation statistics. Every field is a monotonic or
/// weakly-monotonic counter updated with `Relaxed` atomics; readers see
/// self-consistent-enough snapshots for dashboards and reports (exact
/// cross-thread consistency is neither promised nor needed for an
/// observation-only metric).
#[derive(Debug, Default)]
pub struct MemoryStats {
    current_bytes: AtomicUsize,
    peak_bytes: AtomicUsize,
    allocations: AtomicUsize,
    deallocations: AtomicUsize,
    total_allocated_bytes: AtomicUsize,
    total_deallocated_bytes: AtomicUsize,
}

impl MemoryStats {
    /// Live bytes currently allocated through the tracker.
    pub fn current_bytes(&self) -> usize {
        self.current_bytes.load(Ordering::Relaxed)
    }

    /// High-water mark of live bytes since process start.
    pub fn peak_bytes(&self) -> usize {
        self.peak_bytes.load(Ordering::Relaxed)
    }

    /// Number of successful allocations since process start.
    pub fn allocations(&self) -> usize {
        self.allocations.load(Ordering::Relaxed)
    }

    /// Number of deallocations since process start.
    pub fn deallocations(&self) -> usize {
        self.deallocations.load(Ordering::Relaxed)
    }

    /// Lifetime total bytes allocated.
    pub fn total_allocated_bytes(&self) -> usize {
        self.total_allocated_bytes.load(Ordering::Relaxed)
    }

    /// Lifetime total bytes deallocated.
    pub fn total_deallocated_bytes(&self) -> usize {
        self.total_deallocated_bytes.load(Ordering::Relaxed)
    }

    /// A copied snapshot (JSON-exportable).
    pub fn snapshot(&self) -> MemorySnapshot {
        MemorySnapshot {
            format: MEMORY_FORMAT,
            current_bytes: self.current_bytes(),
            peak_bytes: self.peak_bytes(),
            allocations: self.allocations(),
            deallocations: self.deallocations(),
            total_allocated_bytes: self.total_allocated_bytes(),
            total_deallocated_bytes: self.total_deallocated_bytes(),
        }
    }

    /// Compact JSON of a snapshot (byte-stable field order via serde).
    pub fn snapshot_json(&self) -> String {
        serde_json::to_string(&self.snapshot()).expect("MemorySnapshot serializes")
    }
}

/// The memory snapshot's format tag.
pub const MEMORY_FORMAT: &str = "gens-mem-1";

/// A copied view of [`MemoryStats`] at one instant.
#[derive(Debug, Clone, serde::Serialize)]
pub struct MemorySnapshot {
    /// Format tag (`gens-mem-1`), versioned like every engine format.
    pub format: &'static str,
    /// Live bytes at snapshot time.
    pub current_bytes: usize,
    /// High-water mark since process start.
    pub peak_bytes: usize,
    /// Successful allocations since process start.
    pub allocations: usize,
    /// Deallocations since process start.
    pub deallocations: usize,
    /// Lifetime bytes allocated.
    pub total_allocated_bytes: usize,
    /// Lifetime bytes deallocated.
    pub total_deallocated_bytes: usize,
}

/// The tracker's stats singleton.
pub static STATS: MemoryStats = MemoryStats {
    current_bytes: AtomicUsize::new(0),
    peak_bytes: AtomicUsize::new(0),
    allocations: AtomicUsize::new(0),
    deallocations: AtomicUsize::new(0),
    total_allocated_bytes: AtomicUsize::new(0),
    total_deallocated_bytes: AtomicUsize::new(0),
};

/// Wraps [`System`], forwarding every call and counting it. Installed as
/// the process global allocator whenever this crate is linked.
pub struct TrackingAllocator;

unsafe impl GlobalAlloc for TrackingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller upholds GlobalAlloc contract; System handles it.
        let ptr = unsafe { System.alloc(layout) };
        if !ptr.is_null() {
            track_alloc(layout.size());
        }
        ptr
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: caller upholds GlobalAlloc contract; System handles it.
        unsafe { System.dealloc(ptr, layout) };
        STATS
            .current_bytes
            .fetch_sub(layout.size(), Ordering::Relaxed);
        STATS
            .total_deallocated_bytes
            .fetch_add(layout.size(), Ordering::Relaxed);
        STATS.deallocations.fetch_add(1, Ordering::Relaxed);
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: caller upholds GlobalAlloc contract; System handles it.
        let ptr = unsafe { System.alloc_zeroed(layout) };
        if !ptr.is_null() {
            track_alloc(layout.size());
        }
        ptr
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: caller upholds GlobalAlloc contract; System handles it.
        let new_ptr = unsafe { System.realloc(ptr, layout, new_size) };
        if !new_ptr.is_null() {
            if new_size >= layout.size() {
                track_grow(new_size - layout.size());
            } else {
                STATS
                    .current_bytes
                    .fetch_sub(layout.size() - new_size, Ordering::Relaxed);
                STATS
                    .total_deallocated_bytes
                    .fetch_add(layout.size() - new_size, Ordering::Relaxed);
            }
        }
        new_ptr
    }
}

fn track_alloc(size: usize) {
    STATS.allocations.fetch_add(1, Ordering::Relaxed);
    track_grow(size);
}

fn track_grow(size: usize) {
    STATS
        .total_allocated_bytes
        .fetch_add(size, Ordering::Relaxed);
    let live = STATS.current_bytes.fetch_add(size, Ordering::Relaxed) + size;
    STATS.peak_bytes.fetch_max(live, Ordering::Relaxed);
}

/// `#[global_allocator]` for the whole engine workspace: every binary and
/// test that links `gens-profiler` tracks its allocations.
#[global_allocator]
static GLOBAL_ALLOCATOR: TrackingAllocator = TrackingAllocator;

#[cfg(test)]
mod tests {
    use super::*;

    /// All assertions are monotonic (>=) because tests run in parallel
    /// threads and other tests' allocations shift `current` between reads;
    /// only the lifetime totals and counts are guaranteed to move.
    #[test]
    fn allocating_increases_lifetime_totals_and_counts() {
        let before_total = STATS.total_allocated_bytes();
        let before_count = STATS.allocations();
        let v = vec![7u8; 1024 * 1024]; // 1 MiB
        assert!(!v.is_empty());
        assert!(
            STATS.total_allocated_bytes() >= before_total + 1024 * 1024,
            "a 1 MiB allocation must raise the lifetime total by >= 1 MiB"
        );
        assert!(STATS.allocations() > before_count);
        drop(v);
        assert!(STATS.deallocations() >= 1);
    }

    #[test]
    fn peak_is_never_below_current() {
        let v = vec![1u8; 4096];
        let snap = STATS.snapshot();
        assert!(snap.peak_bytes >= snap.current_bytes);
        assert!(snap.current_bytes >= v.len());
    }

    #[test]
    fn totals_are_self_consistent() {
        let snap = STATS.snapshot();
        assert!(snap.total_allocated_bytes >= snap.total_deallocated_bytes);
        assert_eq!(snap.format, "gens-mem-1");
        // Snapshot fields must mirror the accessor methods exactly.
        assert_eq!(snap.current_bytes, STATS.current_bytes());
        assert_eq!(snap.peak_bytes, STATS.peak_bytes());
        assert_eq!(snap.allocations, STATS.allocations());
    }

    #[test]
    fn snapshot_serializes_with_format_tag() {
        let json = STATS.snapshot_json();
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        assert_eq!(parsed["format"], "gens-mem-1");
        assert!(parsed["current_bytes"].as_u64().is_some());
        assert!(parsed["peak_bytes"].as_u64().is_some());
    }

    #[test]
    fn vec_growth_exercises_realloc_path() {
        let before = STATS.total_allocated_bytes();
        let mut v = Vec::with_capacity(1024);
        for i in 0..4096u32 {
            v.push(i);
        }
        assert_eq!(v.len(), 4096);
        assert!(STATS.total_allocated_bytes() >= before + 4096 * 4);
    }
}
