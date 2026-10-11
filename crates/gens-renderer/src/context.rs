//! GPU context: instance creation, adapter selection, device + queue.
//!
//! Adapter selection implements ADR-0003: enumerate everything, classify by
//! [`DeviceType`], and prefer **discrete > integrated > virtual > software**
//! hardware. The classification and the choice itself are pure functions of
//! [`AdapterInfo`], so the policy is unit-tested without any GPU present;
//! creating a real [`GpuContext`] is the only part that needs an adapter.

use gens_core::error::{EngineError, Result};
use wgpu::{AdapterInfo, Backends, DeviceType};

/// Class of graphics hardware an adapter belongs to.
///
/// Ordering is the selection preference (ADR-0003): higher wins.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AdapterClass {
    /// CPU software rasterizer (lavapipe, llvmpipe, WARP). Last resort.
    Software,
    /// Unknown / unclassified device type.
    Unknown,
    /// Virtual or hosted GPU.
    Virtual,
    /// Integrated GPU with shared memory.
    Integrated,
    /// Discrete GPU with dedicated memory. Always preferred.
    Discrete,
}

/// Classifies an adapter from its reported info. Pure and total: every
/// `DeviceType` maps to exactly one class.
#[must_use]
pub fn classify(info: &AdapterInfo) -> AdapterClass {
    match info.device_type {
        DeviceType::DiscreteGpu => AdapterClass::Discrete,
        DeviceType::IntegratedGpu => AdapterClass::Integrated,
        DeviceType::VirtualGpu => AdapterClass::Virtual,
        DeviceType::Cpu => AdapterClass::Software,
        DeviceType::Other => AdapterClass::Unknown,
    }
}

/// Picks the best adapter index out of `infos` under the ADR-0003 policy.
///
/// Higher [`AdapterClass`] wins; ties keep the earliest candidate (driver
/// enumeration order, which mirrors the platform's own preference). Returns
/// `None` when the list is empty.
#[must_use]
pub fn pick_best(infos: &[AdapterInfo]) -> Option<usize> {
    let mut best: Option<(AdapterClass, usize)> = None;
    for (idx, info) in infos.iter().enumerate() {
        let class = classify(info);
        let better = match best {
            None => true,
            Some((current, _)) => class > current,
        };
        if better {
            best = Some((class, idx));
        }
    }
    best.map(|(_, idx)| idx)
}

/// Owns the GPU device and queue, plus everything worth reporting about the
/// adapter they came from.
///
/// Creation never requires a window or surface (ADR-0010), so the same code
/// path works on a developer workstation and on a headless CI runner.
pub struct GpuContext {
    /// Reported info for the adapter in use.
    pub adapter_info: AdapterInfo,
    /// Classification the selector assigned to that adapter.
    pub adapter_class: AdapterClass,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

impl GpuContext {
    /// Creates a context using all backends wgpu was built with.
    ///
    /// # Errors
    /// Returns [`EngineError`] when no adapter matches the backends, or when
    /// the device request on the selected adapter fails.
    pub fn new() -> Result<GpuContext> {
        Self::with_backends(Backends::all())
    }

    /// Creates a context restricted to `backends` (tests force specific
    /// backend masks; production callers use [`GpuContext::new`]).
    ///
    /// # Errors
    /// See [`GpuContext::new`].
    pub fn with_backends(backends: Backends) -> Result<GpuContext> {
        // Headless-first (ADR-0010): no display handle, ever — the offscreen
        // path needs none, and absence keeps GL/Wayland surprises away.
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = backends;
        let instance = wgpu::Instance::new(desc);
        let adapters = pollster::block_on(instance.enumerate_adapters(backends));

        let infos: Vec<AdapterInfo> = adapters.iter().map(wgpu::Adapter::get_info).collect();
        let best = pick_best(&infos).ok_or_else(|| {
            EngineError::invalid(
                "renderer",
                format!(
                    "no GPU adapter found (backends {backends:?}). \
                     On headless CI machines install a software Vulkan driver, \
                     e.g. `apt-get install mesa-vulkan-drivers` (lavapipe)"
                ),
            )
        })?;
        let adapter = &adapters[best];
        let adapter_info = infos[best].clone();
        let adapter_class = classify(&adapter_info);

        let device_desc = wgpu::DeviceDescriptor {
            label: Some("gens-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            experimental_features: wgpu::ExperimentalFeatures::disabled(),
            memory_hints: wgpu::MemoryHints::Performance,
            trace: wgpu::Trace::Off,
        };
        let (device, queue) =
            pollster::block_on(adapter.request_device(&device_desc)).map_err(|e| {
                EngineError::invalid(
                    "renderer",
                    format!(
                        "device request failed on '{}' ({:?}, {:?}): {e}",
                        adapter_info.name, adapter_info.backend, adapter_class
                    ),
                )
            })?;

        Ok(GpuContext {
            adapter_info,
            adapter_class,
            device,
            queue,
        })
    }

    /// The GPU device.
    #[must_use]
    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    /// The command queue.
    #[must_use]
    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    /// One-line human-readable adapter summary for logs and CI evidence,
    /// e.g. `lavapipe (Software, Vulkan, Mesa 24.3)`.
    #[must_use]
    pub fn adapter_summary(&self) -> String {
        format!(
            "{} ({:?}, {:?}, {})",
            self.adapter_info.name,
            self.adapter_class,
            self.adapter_info.backend,
            if self.adapter_info.driver_info.is_empty() {
                self.adapter_info.driver.as_str()
            } else {
                self.adapter_info.driver_info.as_str()
            }
        )
    }
}

impl std::fmt::Debug for GpuContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuContext")
            .field("adapter", &self.adapter_info.name)
            .field("class", &self.adapter_class)
            .field("backend", &self.adapter_info.backend)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::Backend;

    /// Builds a synthetic `AdapterInfo` for policy tests (no GPU needed).
    fn synthetic(name: &str, device_type: DeviceType) -> AdapterInfo {
        AdapterInfo {
            name: name.to_string(),
            vendor: 0x1002,
            device: 0x747e,
            device_type,
            device_pci_bus_id: "0000:03:00.0".to_string(),
            driver: "synthetic".to_string(),
            driver_info: "0.0".to_string(),
            backend: Backend::Vulkan,
            subgroup_min_size: 4,
            subgroup_max_size: 128,
            transient_saves_memory: None,
            limit_bucket: None,
        }
    }

    #[test]
    fn classify_maps_every_device_type() {
        assert_eq!(
            classify(&synthetic("d", DeviceType::DiscreteGpu)),
            AdapterClass::Discrete
        );
        assert_eq!(
            classify(&synthetic("i", DeviceType::IntegratedGpu)),
            AdapterClass::Integrated
        );
        assert_eq!(
            classify(&synthetic("v", DeviceType::VirtualGpu)),
            AdapterClass::Virtual
        );
        assert_eq!(
            classify(&synthetic("s", DeviceType::Cpu)),
            AdapterClass::Software
        );
        assert_eq!(
            classify(&synthetic("o", DeviceType::Other)),
            AdapterClass::Unknown
        );
    }

    #[test]
    fn selection_prefers_discrete_over_everything() {
        let infos = vec![
            synthetic("lavapipe", DeviceType::Cpu),
            synthetic("intel-igpu", DeviceType::IntegratedGpu),
            synthetic("rtx", DeviceType::DiscreteGpu),
        ];
        assert_eq!(pick_best(&infos), Some(2));
    }

    #[test]
    fn selection_prefers_integrated_over_software() {
        let infos = vec![
            synthetic("lavapipe", DeviceType::Cpu),
            synthetic("igpu", DeviceType::IntegratedGpu),
        ];
        assert_eq!(pick_best(&infos), Some(1));
    }

    #[test]
    fn selection_keeps_first_on_tie() {
        let infos = vec![
            synthetic("first", DeviceType::IntegratedGpu),
            synthetic("second", DeviceType::IntegratedGpu),
        ];
        assert_eq!(pick_best(&infos), Some(0));
    }

    #[test]
    fn selection_is_empty_safe() {
        assert_eq!(pick_best(&[]), None);
    }

    #[test]
    fn class_ordering_matches_adr_policy() {
        assert!(AdapterClass::Discrete > AdapterClass::Integrated);
        assert!(AdapterClass::Integrated > AdapterClass::Virtual);
        assert!(AdapterClass::Virtual > AdapterClass::Unknown);
        assert!(AdapterClass::Unknown > AdapterClass::Software);
    }

    /// Real GPU test: requires any adapter (lavapipe on headless CI — ADR-0011).
    /// Marked `#[ignore]`; executed by the dedicated CI step and by
    /// `cargo test -- --ignored` locally where a driver exists.
    #[test]
    #[ignore = "requires a GPU adapter (real or software)"]
    fn gpu_context_reports_and_creates_device() {
        let ctx = GpuContext::new().expect("GPU context creation");
        // The reported summary must name the adapter and classify it.
        let summary = ctx.adapter_summary();
        assert!(
            summary.contains(&ctx.adapter_info.name),
            "summary: {summary}"
        );
        // Whatever the environment provides must classify cleanly.
        assert_ne!(ctx.adapter_class, AdapterClass::Unknown);
    }
}
