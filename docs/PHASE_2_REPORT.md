# Phase 2 Report — wgpu Renderer (offscreen-first, 3-OS GPU CI)

- **Branch:** `phase2-renderer` (merge target: `main`)
- **Date:** 2026-10-12 (UTC)
- **Toolchain:** rustc/cargo 1.99.0 stable, edition 2024; **wgpu 30.0.1** + pollster 1.0.1 (pinned in `[workspace.dependencies]`, every API verified against registry source before use)

## 1. What was delivered

### 1.1 GPU context + adapter policy (P2-1)
`gens-renderer/context.rs`: `AdapterClass` taxonomy (`Discrete > Integrated > Virtual > Unknown > Software`) with pure `classify`/`pick_best` functions implementing the ADR-0003 selection policy, and `GpuContext` — instance → enumerate → classify → device+queue with **no surface anywhere** (ADR-0010). wgpu 30 removed `InstanceDescriptor::default()`; headless instances use `new_without_display_handle()`. Six policy unit tests run everywhere; one `#[ignore]` GPU test runs in CI.

### 1.2 Offscreen render target with readback (P2-2)
`target.rs`: color (`Rgba8UnormSrgb`, `RENDER_ATTACHMENT|COPY_SRC`) + depth (`Depth24Plus`) textures with a padded staging buffer. `copy_color_to_staging` + `read_pixels` give CPU-readable frames with no window system. The WebGPU 256-byte row-alignment contract lives in two pure functions (`bytes_per_row`, `unpack_padded_rgba`) with unit tests covering the alignment matrix and overflow rejection (which caught a real u64-multiply overflow in `validate_rgba8` the same day it was written).

### 1.3 Cameras + math facade growth (P2-4)
`camera.rs`: look-at perspective and centered `orthographic_2d` cameras; `uniform::bytes` serializes `view_proj` into the exact 64-byte WGSL `mat4x4<f32>` block. glam 0.34 moved camera math into `camera::rh::*`, so **gens-math grew the engine facade** (`look_at_rh`, `perspective_rh`, `orthographic_rh`) — engine crates still never call glam directly. All projections emit **WebGPU NDC** (z ∈ [0,1], Y-up) on every backend, pinned by a depth-convention test.

### 1.4 Pipelines, resources, Renderer facade (P2-5/P2-6)
- `shaders/colored.wgsl` + `shaders/textured.wgsl` (group 0 = camera uniform; group 1 = model matrix via **dynamic offset**, sampler + texture join the group in the textured pipeline; fragment = `vertex_color * texture_sample`).
- `ColoredPipeline` / `TexturedPipeline` — depth-tested, back-face culled, alpha-blended; shader compile failures surface as `EngineError`, never panics.
- `GpuMesh` (validated upload) and `ModelUniforms` — a 256-slot, 256-byte-aligned uniform ring bound once and selected per draw via dynamic offsets.
- `Renderer` — the facade: `set_camera`/`set_model` stage uniform writes; `draw_and_read` records the pass, copies to staging, submits, and returns packed RGBA8. `Draw::{Colored,Textured}` dispatches pipelines inside one pass.
- `GpuTexture::from_rgba8` with linear/clamp-to-edge sampling (NPOT-legal) and a pure `validate_rgba8` contract.
- `clear_color_rgba8` / `srgb_byte_to_linear` — engine code never constructs `wgpu::Color` (ADR-0001), and clear semantics follow the WebGPU spec (see §4.2).

### 1.5 `gpu_hello` — the full engine through the GPU (P2-7)
ECS scene (grouping `Root` + orbiting `Hero` with the `player` tag) driven by the Phase 1 fixed-timestep loop over a deterministic `FixedSource`; transforms propagated; **every frame rendered offscreen**: tinted background quad, trail dots every 8th step, and a 2×2-checker textured hero sprite positioned by its propagated `WorldTransform`. The final frame is PNG-encoded and verified in-process (border == background bytes via the fragment path; hero pixel distinct from background/trail; trail present). `--allow-no-gpu` provides an explicit `GPU-SKIP` exit for adapter-less machines; without it the run fails with actionable guidance. Depth layering is documented in code: background −0.4 < trail −0.2 < hero +0.1 (larger view z = nearer under WebGPU depth).

### 1.6 CI: real GPU gates on three operating systems (P2-8, ADR-0011/0012)
- Linux installs `mesa-vulkan-drivers` + `vulkan-tools` and logs a `vulkaninfo` probe; the loader auto-discovers lavapipe.
- New `Tests (GPU integration)` step runs the `#[ignore]`d renderer tests where an adapter is guaranteed (Linux + macOS).
- The smoke test runs the CPU example on all 3 OS **and** the GPU example — required to render on Linux/macOS, best-effort with printed `GPU-SKIP` evidence on Windows (which, in the event, rendered anyway — see §3).
- Artifacts `smoke-snapshots` (`smoke_cpu.png`, `smoke_gpu.png`) upload from Linux and are committed under `docs/evidence/phase2/`.
- Binary size gate recalibrated by **ADR-0012**: gate target = `gpu_hello` (wgpu-linked), 350 MiB vs measured 171.3 MiB Linux baseline.
- `scripts/verify.sh` mirrors every gate 1:1 (7 gates).

## 2. Success criteria scorecard (honest)

| Criterion | Status | Evidence |
|---|---|---|
| Renderer renders via wgpu on 3 platforms | **Met — all 3 OS rendered GPU frames** | run #31 logs; `smoke_gpu.png` per-OS sizes (2084/2090/2088 B) |
| Adapter selection with software fallback | **Met** | llvmpipe (Linux), Metal (macOS), Basic Render Driver DX12 (Windows) |
| Headless (no window) GPU screenshot in CI | **Met** | `smoke_gpu.png` committed; structural in-process verification passed on 3 OS |
| Zero crashes across adapters | **Met** | no panics/validation errors on any backend; wgpu's default handler would have panicked |
| WGSL-only shaders | **Met** | 2 WGSL files, `include_str!`, compile-checked by pipeline creation |
| 20+ new unit tests | **Met — workspace now 80 unit + 6 GPU integration** | run #31; per-crate counts in §3 |
| Binary size gate maintained | **Met — recalibrated by ADR-0012 with evidence** | 174171072 B vs 350 MiB gate |

## 3. Numbers

| Metric | Value |
|---|---|
| Workspace unit tests | **80 passed, 0 failed** (+6 GPU integration, all passing in CI) |
| Test distribution (renderer) | context 7 (6 policy + 1 GPU), target 6 (4 + 2 GPU), mesh 7, camera 5, resources 1, texture 1, renderer 2 + 3 GPU |
| Rendered adapters (CI) | llvmpipe/Vulkan (Linux), Apple Paravirtual/Metal (macOS), Microsoft Basic Render Driver/DX12 (Windows) |
| Cross-backend determinism | triangle interpolation test byte-identical on llvmpipe AND Metal ([135,143,185] at the same sample point) |
| `gpu_hello` debug binary (Linux) | 174171072 B ≈ 166.1 MiB (gate 350 MiB) |
| `hello_world` debug binary | 80.0 MiB (unchanged CPU path) |
| Smoke PNGs | cpu 15353 B; gpu 2084 B (Linux) — border pixel exactly (24,26,33,255) |

## 4. Issues found and fixed during the phase (honest log)

1. **wgpu 30 API drift (multiple)** — `InstanceDescriptor::default()` removed (`new_without_display_handle`), `create_shader_module` takes the descriptor by value, `bind_group_layouts`/`depth_ops`/`depth_write_enabled`/`depth_compare` Option-wrapped, `PipelineLayoutDescriptor.immediate_size` replaced `push_constant_ranges`, `BindingResource::TextureView` rename, `MipmapFilterMode`, `DeviceDescriptor { experimental_features, trace }`. All verified against registry source, none guessed (commits `4d3aa71`..`462d391`).
2. **WebGPU clear semantics on sRGB targets** — run #28's Windows leg stored a requested `[24,26,33]` clear as `[86,90,101]` = `srgb_encode(24/255…)`. WebGPU defines `LoadOp::Clear` values for `*unorm-srgb` targets as **linear**; `clear_color_rgba8` now sRGB-decodes bytes to linear and a round-trip test pins the contract. This is exactly the cross-backend difference the GPU gate exists to catch.
3. **Test geometry, not renderer bugs** — runs #28/#29 produced clear-color-only frames because the GPU tests drew unit-sized shapes (1 pixel) under a 1-unit = 1-pixel camera; fixed with an explicit ortho(2,2) camera. The renderer itself was correct — proven by the byte-identical cross-backend barycentric result after the fix.
4. **Pixel-center ≠ centroid** — the triangle test initially expected the triangle's centroid color at pixel (32,32); the actual sample point is that pixel's center (world (0.015625, −0.015625)). The expectation is now computed from real barycentric weights; tolerance 2/255; result byte-identical on Vulkan and Metal.
5. **Camera-window bug in the example** — `gpu_hello` added `cx/cy` offsets inside an already-centered camera, pushing the hero to ortho x = 220 > 160 (invisible). Models now use pure world coordinates; verification maps world→pixels explicitly.
6. **White background** — run #30: the background quad used default white vertex colors. `tinted_quad` builds vertex colors from sRGB bytes decoded to linear, so the fragment path stores exact background bytes on every backend.
7. **Real overflow bug caught pre-commit** — `validate_rgba8`'s size math overflowed u64 at extreme u32 extents; found by its own unit test the day it was written; fixed with checked arithmetic (`462d391`).
8. **One clippy-1.99 lint** (`manual_is_multiple_of`) fixed rather than allowed (`ea54ec9`).

## 5. CI runs

| Run | SHA | Result |
|---|---|---|
| [#28](https://github.com/salim77007j/gens/actions/runs/38168850605) | 2f80aa3 | failure — first GPU run; caught §4.2/§4.3 |
| [#29](https://github.com/salim77007j/gens/actions/runs/38170332738) | a437540 | failure — caught §4.4/§4.5 |
| [#30](https://github.com/salim77007j/gens/actions/runs/38170673803) | 35bbcef | failure — caught §4.6 |
| [#31](https://github.com/salim77007j/gens/actions/runs/38170942161) | 72f0561 | **success — full matrix, GPU rendering + GPU tests on 3 OS** |

Full ledger with per-OS adapters, artifact sizes and byte-level verification: `docs/evidence/phase2/ci-runs.txt`. Committed visual evidence: `docs/evidence/phase2/smoke_gpu.png` (GPU-rendered scene), `smoke_cpu.png` (CPU path).

## 6. Decisions & risks delta

- **ADR-0010** (offscreen-first rendering) and **ADR-0011** (CI software-render policy) added at phase start; **ADR-0012** (size-gate recalibration to 350 MiB on the wgpu-linked binary) added with measurements.
- **ADR-0009 duplicate entry removed** (governance defect caught in the Phase 2 review, disclosed in the P2-0 worklog entry).
- **R-004** (software render unrepresentative) → note: CI now renders real GPU frames on three adapters; performance claims still await Phase 8 benchmarks. Status stays Open with updated mitigation.
- **R-002** (unpushed work window) → push discipline was broken at the start of this phase (sandbox env reset dropped the credential) and restored mid-phase once the user re-exported the token; all work landed granularly and every commit is CI-verified. Status stays Monitoring.
- **R-005/R-010** (runner variance) → GPU smoke adds a per-OS adapter variable; the Windows leg remains honestly best-effort by ADR-0011 even though it rendered this time.
- **R-013** unchanged (physics/networking determinism — Phase 3).

## 7. What Phase 3 can build on

A proven, CI-verified GPU render path with camera/model/texture plumbing, a facade that keeps wgpu contained in one crate, 86 green tests, and honest evidence of cross-backend behavior. Physics (rapier), audio (kira), and input (winit/gilrs) land on top of that in Phase 3 — with cross-platform fixed-step determinism tests (R-013) now possible against a real, rendering game loop.
