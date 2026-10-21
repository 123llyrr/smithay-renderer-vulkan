//! Hardware test fixtures and helpers for Vulkan testing on a DRM render node.

use std::sync::Arc;
use ash::vk;
use smithay::backend::allocator::gbm::{GbmAllocator, GbmBufferFlags, GbmDevice};
use smithay::backend::allocator::{Fourcc, Modifier};
use smithay::backend::drm::DrmNode;
use smithay::backend::egl::{EGLContext, EGLDisplay};
use smithay::backend::renderer::gles::GlesRenderer;

use super::device::VulkanContext;
use super::VulkanRenderer;

pub(crate) fn setup() -> (
    VulkanRenderer,
    GlesRenderer,
    GbmAllocator<Arc<std::fs::File>>,
) {
    let path = std::env::var("VULKAN_TEST_NODE")
        .or_else(|_| std::env::var("CLARION_VULKAN_TEST_NODE"))
        .unwrap_or_else(|_| {
            if std::path::Path::new("/dev/dri/renderD128").exists() {
                "/dev/dri/renderD128".to_string()
            } else {
                panic!("Set VULKAN_TEST_NODE to a valid DRM render node (e.g. /dev/dri/renderD128)");
            }
        });

    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("Failed to open DRM render node {path}: {e}"));

    let node = DrmNode::from_file(&file).expect("Failed to create DrmNode from file");
    let mut vk = VulkanRenderer::new();
    vk.init_from_drm_node(Some(&node))
        .expect("Failed to initialize VulkanRenderer from DRM node");

    let advertised_device = vk
        .ctx
        .as_ref()
        .unwrap()
        .drm_render_device_id()
        .expect("Failed to query render device id");
    assert_eq!(advertised_device, Some(node.dev_id()));

    let gbm = GbmDevice::new(Arc::new(file)).expect("Failed to create GbmDevice");
    let display = unsafe { EGLDisplay::new(gbm.clone()).expect("Failed to create EGLDisplay") };
    let egl = EGLContext::new(&display).expect("Failed to create EGLContext");
    let gles = unsafe { GlesRenderer::new(egl).expect("Failed to create GlesRenderer") };

    (vk, gles, GbmAllocator::new(gbm, GbmBufferFlags::RENDERING))
}

pub(crate) fn modifiers(vk: &VulkanRenderer, gles: &GlesRenderer, code: Fourcc) -> Vec<Modifier> {
    let formats = gles
        .egl_context()
        .dmabuf_render_formats()
        .iter()
        .copied()
        .filter(|f| f.code == code && vk.supports_target_format(*f))
        .map(|f| f.modifier);
    formats.collect()
}

pub(crate) struct SubmittedGpuWork {
    ctx: Arc<VulkanContext>,
    buffer: vk::Buffer,
    memory: vk::DeviceMemory,
    signal: vk::Semaphore,
    command: vk::CommandBuffer,
    fence: vk::Fence,
    submitted: bool,
}

impl Drop for SubmittedGpuWork {
    fn drop(&mut self) {
        unsafe {
            if self.submitted {
                let _ = self
                    .ctx
                    .device
                    .wait_for_fences(&[self.fence], true, u64::MAX);
            }
            let _lock = self.ctx.command_lock.lock().unwrap();
            if self.command != vk::CommandBuffer::null() {
                self.ctx
                    .device
                    .free_command_buffers(self.ctx.command_pool, &[self.command]);
            }
            self.ctx.device.destroy_fence(self.fence, None);
            self.ctx.device.destroy_semaphore(self.signal, None);
            self.ctx.device.destroy_buffer(self.buffer, None);
            self.ctx.device.free_memory(self.memory, None);
        }
    }
}

impl SubmittedGpuWork {
    pub(crate) fn new(ctx: Arc<VulkanContext>) -> Self {
        let mut work = SubmittedGpuWork {
            ctx: ctx.clone(),
            buffer: vk::Buffer::null(),
            memory: vk::DeviceMemory::null(),
            signal: vk::Semaphore::null(),
            command: vk::CommandBuffer::null(),
            fence: vk::Fence::null(),
            submitted: false,
        };
        unsafe {
            let size = 16 * 1024 * 1024;
            work.buffer = ctx
                .device
                .create_buffer(
                    &vk::BufferCreateInfo::default()
                        .size(size)
                        .usage(vk::BufferUsageFlags::TRANSFER_DST),
                    None,
                )
                .unwrap();
            let req = ctx.device.get_buffer_memory_requirements(work.buffer);
            let props = ctx
                .instance
                .get_physical_device_memory_properties(ctx.physical_device);
            let index = (0..props.memory_type_count)
                .find(|i| {
                    req.memory_type_bits & (1 << i) != 0
                        && props.memory_types[*i as usize]
                            .property_flags
                            .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
                })
                .unwrap();
            work.memory = ctx
                .device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(req.size)
                        .memory_type_index(index),
                    None,
                )
                .unwrap();
            ctx.device
                .bind_buffer_memory(work.buffer, work.memory, 0)
                .unwrap();
            let mut export = vk::ExportSemaphoreCreateInfo::default()
                .handle_types(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD);
            work.signal = ctx
                .device
                .create_semaphore(
                    &vk::SemaphoreCreateInfo::default().push_next(&mut export),
                    None,
                )
                .unwrap();
            work.fence = ctx
                .device
                .create_fence(&vk::FenceCreateInfo::default(), None)
                .unwrap();
            let _lock = ctx.command_lock.lock().unwrap();
            work.command = ctx
                .device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(ctx.command_pool)
                        .level(vk::CommandBufferLevel::PRIMARY)
                        .command_buffer_count(1),
                )
                .unwrap()[0];
            ctx.device
                .begin_command_buffer(work.command, &vk::CommandBufferBeginInfo::default())
                .unwrap();
            for i in 0..2048 {
                ctx.device
                    .cmd_fill_buffer(work.command, work.buffer, 0, size, i);
                let barrier = vk::BufferMemoryBarrier::default()
                    .buffer(work.buffer)
                    .size(size)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                    .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED);
                ctx.device.cmd_pipeline_barrier(
                    work.command,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[barrier],
                    &[],
                );
            }
            ctx.device.end_command_buffer(work.command).unwrap();
        }
        work
    }

    pub(crate) fn submit(&mut self) -> std::os::fd::OwnedFd {
        use std::os::fd::FromRawFd;
        unsafe {
            let _lock = self.ctx.command_lock.lock().unwrap();
            self.ctx
                .device
                .queue_submit(
                    self.ctx.graphics_queue,
                    &[vk::SubmitInfo::default()
                        .command_buffers(&[self.command])
                        .signal_semaphores(&[self.signal])],
                    self.fence,
                )
                .unwrap();
            self.submitted = true;
            let api =
                ash::khr::external_semaphore_fd::Device::new(&self.ctx.instance, &self.ctx.device);
            let raw = api
                .get_semaphore_fd(
                    &vk::SemaphoreGetFdInfoKHR::default()
                        .semaphore(self.signal)
                        .handle_type(vk::ExternalSemaphoreHandleTypeFlags::SYNC_FD),
                )
                .unwrap();
            assert!(raw >= 0);
            std::os::fd::OwnedFd::from_raw_fd(raw)
        }
    }

    pub(crate) fn wait(&self) {
        unsafe {
            self.ctx
                .device
                .wait_for_fences(&[self.fence], true, 30_000_000_000)
                .unwrap();
        }
    }

    pub(crate) fn is_reached(&self) -> bool {
        unsafe { self.ctx.device.get_fence_status(self.fence).unwrap() }
    }
}

pub(crate) mod lifecycle {
    pub(crate) fn run_quarantine_fixture_in_child() -> bool {
        false
    }
}
