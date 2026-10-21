//! Minimal Wayland Compositor example demonstrating `smithay-renderer-vulkan`.
//!
//! This example demonstrates initializing the native Vulkan rendering context,
//! querying device capabilities, and creating a Vulkan-backed Smithay NativeRenderer.

use smithay::backend::drm::DrmNode;
use smithay_renderer_vulkan::native::NativeRenderer;
use smithay_renderer_vulkan::VulkanRenderer;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt().init();

    println!("============================================================");
    println!("  smithay-renderer-vulkan: Minimal Compositor Example");
    println!("============================================================");

    // 1. Instantiate the VulkanRenderer
    let mut renderer = VulkanRenderer::new();
    println!("[1/3] Created uninitialized VulkanRenderer instance.");

    // 2. Discover primary DRM render node if available on Linux
    if let Ok(node) = DrmNode::from_path("/dev/dri/renderD128") {
        println!("[2/3] Found primary DRM render node: {:?}", node);
        match renderer.init_from_drm_node(Some(&node)) {
            Ok(()) => {
                println!("[3/3] Vulkan physical & logical device initialized!");
                println!("      Device: {}", renderer.info.device_name);
                println!(
                    "      Vulkan API Version: {}.{}.{}",
                    renderer.info.api_version.0,
                    renderer.info.api_version.1,
                    renderer.info.api_version.2
                );
                println!("      Dynamic Rendering (1.3): {}", renderer.info.supports_dynamic_rendering);
                println!("      DRM Format Modifiers: {}", renderer.info.supports_drm_modifiers);
                println!("      Zero-Copy DMA-BUF Import: {}", renderer.info.supports_dmabuf_import);
                println!("      Timeline Semaphores (Explicit Sync): {}", renderer.info.supports_timeline_semaphores);

                // Create Smithay NativeRenderer adapter
                match NativeRenderer::new(renderer) {
                    Ok(_native) => {
                        println!("\nSuccessfully constructed Smithay NativeRenderer adapter!");
                        println!("Ready to render Wayland frames with native Vulkan 1.3 & HDR!");
                    }
                    Err(err) => {
                        eprintln!("Failed to construct NativeRenderer: {:?}", err);
                    }
                }
            }
            Err(err) => {
                eprintln!("      Note: Could not open DRM render node ({:?}). Running in headless mode.", err);
            }
        }
    } else {
        println!("[2/3] No direct DRM node accessible. Headless or nested mode available.");
    }

    Ok(())
}
