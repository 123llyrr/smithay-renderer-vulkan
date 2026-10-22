# smithay-renderer-vulkan

[![Crates.io](https://img.shields.io/badge/crates.io-v0.1.1-orange.svg)](https://crates.io/)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/License-MIT%20OR%20Apache--2.0-blue.svg)](LICENSE-MIT)
[![Vulkan: 1.3+](https://img.shields.io/badge/Vulkan-1.3+-red.svg)](https://www.vulkan.org/)
[![Smithay: 0.7+](https://img.shields.io/badge/Smithay-0.7+-green.svg)](https://github.com/Smithay/smithay)

**Enhanced native Vulkan 1.3 rendering backend and Smithay `Renderer` adapter for Wayland compositors** — the **"Vulkan SceneFX"** for modern Rust Wayland desktop environments.

Combines zero-copy direct DRM scanout (`Bind<Dmabuf>`), full-chain **HDR10 / scRGB color management**, hardware-accelerated **3D LUT (`VkImageType::TYPE_3D`) color grading**, rich compositor visual effects (Dual-Kawase blur, soft shadows, rounded corners, particle bursts), and out-of-the-box **in-process shader compilation** via optional `naga` or `shaderc` features.

Designed to serve as a drop-in high-performance Vulkan renderer for **COSMIC** (`cosmic-comp`), **Smithay**-based compositors, and custom Wayland displays.

---

## 🌟 Key Highlights

* **Hardware DRM Scanout Closed-Loop (`Bind<Dmabuf>`)**: Native implementation of `Bind<Dmabuf>` allows externally allocated DRM/GBM swapchain buffers to be directly bound and imported as Vulkan render target attachments, plugging seamlessly into Smithay's `DrmCompositor` without offscreen copies or window nesting.
* **Vulkan 1.3 Dynamic Rendering**: Complete elimination of legacy `VkRenderPass` and `VkFramebuffer` boilerplate; all rasterization binds directly to output attachments via `cmd_begin_rendering`.
* **Zero-Copy DMA-BUF Import & Modifiers**: Native support for `VK_EXT_image_drm_format_modifier` and `VK_KHR_external_memory_fd`. Seamlessly preserves GPU tile-compression (Intel Tile4, AMD DCC) across Wayland client and compositor boundaries.
* **Full-Chain HDR & Wide-Color Gamut**:
  * Native color space definitions for **BT.2020**, **Display-P3**, and **sRGB**.
  * Transfer functions: **SMPTE ST 2084 (PQ)**, sRGB, Gamma 2.2, and linear FP16.
  * Hardware-accelerated YUV420 / NV12 texture sampling via `vk::SamplerYcbcrConversion` (zero shader ALU cost).
* **Hardware 3D LUT Sampling (`VkImageType::TYPE_3D`)**:
  * First-class support for loading Adobe/DaVinci `.cube` and ICC 3D LUT files.
  * Uploads to native Vulkan 3D textures (`TYPE_3D`) with hardware trilinear filtering (`VK_FILTER_LINEAR`) and edge clamping for cinematic color grading and display calibration at zero ALU overhead.
* **In-Process Shader Compilation (Zero-Dependency Toolchain)**:
  * Optional Cargo features `naga` (pure Rust) or `shaderc` allow compiling user GLSL custom fragment shaders into SPIR-V inside the compositor process.
  * Eliminates external runtime command dependencies like `glslc` or `vulkan-shader-compiler`.
* **Compositor Visual Effects ("Vulkan SceneFX")**:
  * Hardware anti-aliased rounded corners (`rounding_alpha.frag`).
  * Smooth drop shadows with separable gaussian/exponential kernels.
  * Multi-pass Dual-Kawase blur with downsampling/upsampling scratch textures.
  * Particle explosion/implosion animations and extensible SPIR-V fragment animation pipelines.
* **Linux Explicit Synchronization**: Deep integration between Linux `drm_syncobj` and `VK_KHR_timeline_semaphore`. Non-blocking pipelined execution across client GPU submit, compositor rendering, and KMS scanout.
* **Asynchronous Device Recovery (`VulkanRecoveryWorker`)**: Gracefully intercepts `VK_ERROR_DEVICE_LOST` and asynchronously reconstructs logical devices without crashing the compositor process.

---

## 🚀 Quick Start

Add `smithay-renderer-vulkan` to your compositor's `Cargo.toml`:

```toml
[dependencies]
# Pure Rust in-process shader compilation via naga (recommended):
smithay-renderer-vulkan = { version = "0.1.1", features = ["naga"] }
# Or with Google shaderc bindings:
# smithay-renderer-vulkan = { version = "0.1.1", features = ["shaderc"] }

smithay = { version = "0.7.0", features = ["backend_drm", "backend_vulkan", "desktop"] }
```

### Initializing the Renderer & DRM Compositor

```rust
use smithay::backend::drm::DrmNode;
use smithay::backend::renderer::Bind;
use smithay_renderer_vulkan::{VulkanRenderer, NativeRenderer};

// 1. Discover primary DRM render node
let render_node = DrmNode::from_path("/dev/dri/renderD128")?;

// 2. Initialize Vulkan context on the node
let mut vk_renderer = VulkanRenderer::new();
vk_renderer.init_from_drm_node(Some(&render_node))?;

println!("Initialized Vulkan device: {}", vk_renderer.info.device_name);

// 3. Construct Smithay NativeRenderer adapter
let mut native_renderer = NativeRenderer::new(vk_renderer)?;
// native_renderer now implements:
//   Renderer, Frame, Bind<Dmabuf>, Bind<VulkanTexture>,
//   ImportDma, ImportMem, ImportDmaWl, ImportMemWl, ExportMem, Offscreen

// 4. Bind directly to external DRM/GBM swapchain scanout buffers:
// let mut framebuffer = native_renderer.bind(&mut scanout_dmabuf)?;
// DrmCompositor::render_frame seamlessly renders directly to scanout!
```

### Loading & Applying a 3D LUT (.cube)

```rust
use smithay_renderer_vulkan::{Lut3D, VulkanLut3D};

// Parse an Adobe/DaVinci .cube file (e.g. 17x17x17 or 33x33x33 or 65x65x65)
let cube_data = include_str!("../assets/warm_cinematic.cube");
let lut = Lut3D::from_cube_str(cube_data)?;

// Upload directly to a VkImageType::TYPE_3D texture with trilinear filtering:
let vk_lut = native_renderer.create_3d_lut_from_cube(cube_data)?;

println!("Uploaded 3D LUT '{}' (size {}^3)", vk_lut.title().unwrap_or("LUT"), vk_lut.size());
```

---

## 🔬 Hardware Verification

The implementation has been verified directly on physical hardware and Linux DRM render nodes:

```text
============================================================
  smithay-renderer-vulkan: Minimal Compositor Example
============================================================
[1/3] Created uninitialized VulkanRenderer instance.
[2/3] Found primary DRM render node: DrmNode { dev: 57984, ty: Render }
[3/3] Vulkan physical & logical device initialized!
      Device: Intel(R) Iris(R) Xe Graphics (ADL GT2)
      Vulkan API Version: 1.4.354
      Dynamic Rendering (1.3): true
      DRM Format Modifiers: true
      Zero-Copy DMA-BUF Import: true
      Timeline Semaphores (Explicit Sync): true

Successfully constructed Smithay NativeRenderer adapter!
Ready to render Wayland frames with native Vulkan 1.3 & HDR!
```

---

## ⚡ Empirical Performance & Trajectory Benchmarks

Measured on physical hardware (`eDP-1` 16.66ms / 60-144Hz) with dual-GPU support (Intel Iris Xe ADL GT2 + NVIDIA GeForce RTX 2050):

| Metric | Traditional GLES/EGL Compositor | `smithay-renderer-vulkan` | Advantage |
| :--- | :--- | :--- | :--- |
| **CPU Frame Work P50** | 4.80 ms - 6.50 ms | **0.385 ms** (385 µs) | **-92% latency** 🚀 |
| **CPU Frame Work P95** | 7.20 ms - 11.50 ms | **0.608 ms** (608 µs) | **Zero overruns** |
| **GPU Command-Buffer P95** | 4.50 ms - 7.80 ms | **1.636 ms - 2.325 ms** | **-65% GPU time** |
| **Compositor VRAM Footprint** | 180 MB - 260 MB | **24 MB - 34 MB** | **-86.5% VRAM savings** ⚡ |
| **Interactive Temperature** | 66°C - 74°C | **46°C - 50°C** | **-18°C ~ -20°C cooler** ❄️ |
| **KMS Latch Wakeup Latency** | 0.80 ms - 1.50 ms | **0.030 ms** (30 µs) | **Sub-millisecond vblank sync** |

### Performance Trajectories

![Frame Latency Trajectory](benchmarks/trajectory-frame-timings.svg)
![VRAM & Thermal Trajectory](benchmarks/trajectory-vram-temperature.svg)

For the full benchmark methodology, raw session dataset JSONs, and reproduction instructions, see **[benchmarks/README.md](benchmarks/README.md)**.

---

## 📂 Architecture & Module Breakdown

```text
smithay-renderer-vulkan/
├── src/
│   ├── lib.rs              # Public exports, VulkanRenderer, backend capabilities
│   ├── device.rs           # VkInstance, DRM node matcher (VK_EXT_physical_device_drm)
│   ├── dmabuf.rs           # Zero-copy DMA-BUF importer with LRU cache & modifier negotiation
│   ├── frame.rs            # Smithay Frame trait implementation & command recording
│   ├── native.rs           # Smithay Renderer, Bind<Dmabuf>, ExportMem, Offscreen adapter
│   ├── color/
│   │   └── lut3d.rs        # Adobe .cube 3D LUT parser & VkImageType::TYPE_3D sampling
│   ├── color.rs            # ColorDescription, Primaries, PQ ST 2084 encode/decode
│   ├── color_pipeline.rs   # Tone mapping push constants & color conversion matrices
│   ├── hdr.rs              # HDR presentation parameters & swapchain formats
│   ├── sync.rs             # Explicit sync, timeline semaphores, fence timeout watchdogs
│   ├── pipeline.rs         # Dynamic rendering layouts, descriptor sets, YUV hardware samplers
│   ├── blur.rs             # Dual-Kawase blur pipeline with bounded scratch allocation
│   ├── custom_shader.rs    # User fragment animation engine (naga / shaderc in-process)
│   ├── lifecycle.rs        # Device recovery barriers & async worker threads
│   ├── memory.rs           # Host-visible memory quotas & staging buffers
│   └── wsi.rs              # Nested Winit VK_KHR_swapchain integration
└── examples/
    └── vulkan_minimal.rs   # Self-contained minimal compositor demo
```

---

## 🤝 Community & Upstream Alignment

This crate is dedicated to advancing the open-source Rust Wayland graphics stack. It directly bridges the gap identified in Smithay's `feat/vulkan-renderer` discussions and offers the wider community (including System76's COSMIC desktop and Niri) an immediate, battle-tested path to **Native Vulkan + HDR**.

Contributions, feedback, and RFC alignments with `Smithay/smithay` and `pop-os/cosmic-comp` are warmly welcomed!

---

## 📄 License

Dual-licensed under either:
* **MIT License** ([LICENSE-MIT](LICENSE-MIT))
* **Apache License, Version 2.0** ([LICENSE-APACHE](LICENSE-APACHE))

at your option.
