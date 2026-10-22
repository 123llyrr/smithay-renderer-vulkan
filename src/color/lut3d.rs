//! 3D Lookup Table (LUT) management and hardware sampling interface.
//!
//! Provides support for parsing Adobe/DaVinci `.cube` 3D LUT files and
//! uploading them as true Vulkan `VkImageType::TYPE_3D` textures with
//! hardware trilinear interpolation for color grading, display calibration,
//! and cinematic color transformations.

use std::io::{self, BufRead};
use std::sync::Arc;

use ash::vk;

use crate::device::VulkanContext;

/// Errors that can occur during 3D LUT parsing or validation.
#[derive(Debug)]
pub enum LutError {
    InvalidSize(u32),
    InvalidDomain,
    DataMismatch { expected: usize, actual: usize },
    ParseError(String),
    Io(io::Error),
}

impl std::fmt::Display for LutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidSize(s) => write!(f, "invalid 3D LUT size: {s} (expected 2..=256)"),
            Self::InvalidDomain => write!(f, "invalid 3D LUT domain: max must be greater than min"),
            Self::DataMismatch { expected, actual } => {
                write!(f, "3D LUT data count mismatch: expected {expected}, got {actual}")
            }
            Self::ParseError(msg) => write!(f, "3D LUT parse error: {msg}"),
            Self::Io(err) => write!(f, "3D LUT I/O error: {err}"),
        }
    }
}

impl std::error::Error for LutError {}

impl From<io::Error> for LutError {
    fn from(err: io::Error) -> Self {
        Self::Io(err)
    }
}

/// Errors that can occur when creating Vulkan 3D LUT GPU resources.
#[derive(Debug)]
pub enum VulkanLutError {
    Vk(vk::Result),
    Lut(LutError),
    Unsupported(String),
}

impl std::fmt::Display for VulkanLutError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Vk(res) => write!(f, "Vulkan error during 3D LUT allocation: {res:?}"),
            Self::Lut(err) => write!(f, "3D LUT error: {err}"),
            Self::Unsupported(msg) => write!(f, "unsupported 3D LUT configuration: {msg}"),
        }
    }
}

impl std::error::Error for VulkanLutError {}

impl From<vk::Result> for VulkanLutError {
    fn from(res: vk::Result) -> Self {
        Self::Vk(res)
    }
}

impl From<LutError> for VulkanLutError {
    fn from(err: LutError) -> Self {
        Self::Lut(err)
    }
}

/// A CPU-side representation of a 3D color lookup table.
#[derive(Debug, Clone, PartialEq)]
pub struct Lut3D {
    /// Dimension size $N$ along each edge (e.g. 17, 33, 65).
    pub size: u32,
    /// Optional title or description from the `.cube` header.
    pub title: Option<String>,
    /// Minimum input domain bound (typically `[0.0, 0.0, 0.0]`).
    pub domain_min: [f32; 3],
    /// Maximum input domain bound (typically `[1.0, 1.0, 1.0]`).
    pub domain_max: [f32; 3],
    /// Contiguous array of RGBA color points of length $N \times N \times N$.
    /// Point ordering: Red varies fastest, then Green, then Blue.
    pub data: Vec<[f32; 4]>,
}

impl Lut3D {
    /// Create a new 3D LUT from raw data points.
    pub fn new(size: u32, data: Vec<[f32; 4]>) -> Result<Self, LutError> {
        if !(2..=256).contains(&size) {
            return Err(LutError::InvalidSize(size));
        }
        let expected = (size * size * size) as usize;
        if data.len() != expected {
            return Err(LutError::DataMismatch {
                expected,
                actual: data.len(),
            });
        }
        Ok(Self {
            size,
            title: None,
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            data,
        })
    }

    /// Generate an identity 3D LUT of dimension `size`.
    ///
    /// For an identity LUT, $f(r, g, b) = (r, g, b)$ for all input values.
    pub fn identity(size: u32) -> Self {
        assert!((2..=256).contains(&size), "size must be in 2..=256");
        let mut data = Vec::with_capacity((size * size * size) as usize);
        let denom = (size - 1) as f32;
        for b in 0..size {
            for g in 0..size {
                for r in 0..size {
                    data.push([r as f32 / denom, g as f32 / denom, b as f32 / denom, 1.0]);
                }
            }
        }
        Self {
            size,
            title: Some("Identity".into()),
            domain_min: [0.0, 0.0, 0.0],
            domain_max: [1.0, 1.0, 1.0],
            data,
        }
    }

    /// Parse an Adobe `.cube` 3D LUT file from a string.
    pub fn from_cube_str(content: &str) -> Result<Self, LutError> {
        Self::from_cube(io::Cursor::new(content.as_bytes()))
    }

    /// Parse an Adobe `.cube` 3D LUT file from a buffered reader.
    pub fn from_cube<R: BufRead>(reader: R) -> Result<Self, LutError> {
        let mut size = None;
        let mut title = None;
        let mut domain_min = [0.0f32, 0.0, 0.0];
        let mut domain_max = [1.0f32, 1.0, 1.0];
        let mut data = Vec::new();

        for line_result in reader.lines() {
            let line = line_result?;
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            let mut parts = trimmed.split_whitespace();
            let first = match parts.next() {
                Some(p) => p,
                None => continue,
            };

            if first.eq_ignore_ascii_case("TITLE") {
                let remainder = trimmed["TITLE".len()..].trim();
                let clean = remainder.trim_matches('"');
                title = Some(clean.to_string());
            } else if first.eq_ignore_ascii_case("LUT_3D_SIZE") {
                let s = parts.next().ok_or_else(|| {
                    LutError::ParseError("LUT_3D_SIZE missing size value".into())
                })?;
                let sz: u32 = s.parse().map_err(|e| {
                    LutError::ParseError(format!("invalid LUT_3D_SIZE '{s}': {e}"))
                })?;
                if !(2..=256).contains(&sz) {
                    return Err(LutError::InvalidSize(sz));
                }
                size = Some(sz);
                data.reserve((sz * sz * sz) as usize);
            } else if first.eq_ignore_ascii_case("DOMAIN_MIN") {
                domain_min[0] = parts.next().and_then(|v| v.parse().ok()).ok_or_else(|| {
                    LutError::ParseError("invalid DOMAIN_MIN R value".into())
                })?;
                domain_min[1] = parts.next().and_then(|v| v.parse().ok()).ok_or_else(|| {
                    LutError::ParseError("invalid DOMAIN_MIN G value".into())
                })?;
                domain_min[2] = parts.next().and_then(|v| v.parse().ok()).ok_or_else(|| {
                    LutError::ParseError("invalid DOMAIN_MIN B value".into())
                })?;
            } else if first.eq_ignore_ascii_case("DOMAIN_MAX") {
                domain_max[0] = parts.next().and_then(|v| v.parse().ok()).ok_or_else(|| {
                    LutError::ParseError("invalid DOMAIN_MAX R value".into())
                })?;
                domain_max[1] = parts.next().and_then(|v| v.parse().ok()).ok_or_else(|| {
                    LutError::ParseError("invalid DOMAIN_MAX G value".into())
                })?;
                domain_max[2] = parts.next().and_then(|v| v.parse().ok()).ok_or_else(|| {
                    LutError::ParseError("invalid DOMAIN_MAX B value".into())
                })?;
            } else if first.eq_ignore_ascii_case("LUT_1D_SIZE") {
                return Err(LutError::ParseError(
                    "1D LUTs are not supported by the 3D LUT module".into(),
                ));
            } else {
                // Must be a data row: R G B
                let r: f32 = first.parse().map_err(|e| {
                    LutError::ParseError(format!("invalid float '{first}': {e}"))
                })?;
                let g_str = parts.next().ok_or_else(|| {
                    LutError::ParseError("expected 3 values per row, got 1".into())
                })?;
                let g: f32 = g_str.parse().map_err(|e| {
                    LutError::ParseError(format!("invalid float '{g_str}': {e}"))
                })?;
                let b_str = parts.next().ok_or_else(|| {
                    LutError::ParseError("expected 3 values per row, got 2".into())
                })?;
                let b: f32 = b_str.parse().map_err(|e| {
                    LutError::ParseError(format!("invalid float '{b_str}': {e}"))
                })?;
                data.push([r, g, b, 1.0]);
            }
        }

        let sz = size.ok_or_else(|| LutError::ParseError("missing LUT_3D_SIZE header".into()))?;
        let expected = (sz * sz * sz) as usize;
        if data.len() != expected {
            return Err(LutError::DataMismatch {
                expected,
                actual: data.len(),
            });
        }
        if domain_max[0] <= domain_min[0]
            || domain_max[1] <= domain_min[1]
            || domain_max[2] <= domain_min[2]
        {
            return Err(LutError::InvalidDomain);
        }

        Ok(Self {
            size: sz,
            title,
            domain_min,
            domain_max,
            data,
        })
    }

    /// Sample the 3D LUT CPU-side using trilinear interpolation.
    ///
    /// This matches Vulkan's hardware trilinear texture filtering behavior
    /// with `VK_SAMPLER_ADDRESS_MODE_CLAMP_TO_EDGE`.
    pub fn sample_trilinear(&self, r: f32, g: f32, b: f32) -> [f32; 3] {
        let n = self.size as f32;
        // Normalize input within domain bounds
        let nr = ((r - self.domain_min[0]) / (self.domain_max[0] - self.domain_min[0]))
            .clamp(0.0, 1.0);
        let ng = ((g - self.domain_min[1]) / (self.domain_max[1] - self.domain_min[1]))
            .clamp(0.0, 1.0);
        let nb = ((b - self.domain_min[2]) / (self.domain_max[2] - self.domain_min[2]))
            .clamp(0.0, 1.0);

        let fx = nr * (n - 1.0);
        let fy = ng * (n - 1.0);
        let fz = nb * (n - 1.0);

        let x0 = fx.floor() as u32;
        let y0 = fy.floor() as u32;
        let z0 = fz.floor() as u32;

        let x1 = (x0 + 1).min(self.size - 1);
        let y1 = (y0 + 1).min(self.size - 1);
        let z1 = (z0 + 1).min(self.size - 1);

        let dx = fx - x0 as f32;
        let dy = fy - y0 as f32;
        let dz = fz - z0 as f32;

        let idx = |x: u32, y: u32, z: u32| -> usize {
            (z * self.size * self.size + y * self.size + x) as usize
        };

        let c000 = self.data[idx(x0, y0, z0)];
        let c100 = self.data[idx(x1, y0, z0)];
        let c010 = self.data[idx(x0, y1, z0)];
        let c110 = self.data[idx(x1, y1, z0)];
        let c001 = self.data[idx(x0, y0, z1)];
        let c101 = self.data[idx(x1, y0, z1)];
        let c011 = self.data[idx(x0, y1, z1)];
        let c111 = self.data[idx(x1, y1, z1)];

        let mut result = [0.0f32; 3];
        for i in 0..3 {
            let c00 = c000[i] * (1.0 - dx) + c100[i] * dx;
            let c10 = c010[i] * (1.0 - dx) + c110[i] * dx;
            let c01 = c001[i] * (1.0 - dx) + c101[i] * dx;
            let c11 = c011[i] * (1.0 - dx) + c111[i] * dx;

            let c0 = c00 * (1.0 - dy) + c10 * dy;
            let c1 = c01 * (1.0 - dy) + c11 * dy;

            result[i] = c0 * (1.0 - dz) + c1 * dz;
        }
        result
    }

    /// Raw byte representation of the RGBA float32 points.
    pub fn to_fp32_bytes(&self) -> &[u8] {
        bytemuck::cast_slice(&self.data)
    }
}

/// GPU-side hardware 3D texture handle backed by `VkImageType::TYPE_3D`.
///
/// Enables zero-ALU trilinear color grading and ICC 3D LUT sampling directly
/// within Vulkan fragment pipelines.
pub struct VulkanLut3D {
    ctx: Arc<VulkanContext>,
    image: vk::Image,
    image_view: vk::ImageView,
    sampler: vk::Sampler,
    memory: vk::DeviceMemory,
    allocation_size: vk::DeviceSize,
    size: u32,
    title: Option<String>,
}

impl VulkanLut3D {
    /// Create and upload a 3D LUT texture to the GPU.
    pub fn new(ctx: Arc<VulkanContext>, lut: &Lut3D) -> Result<Self, VulkanLutError> {
        let device = &ctx.device;
        let size = lut.size;
        let extent = vk::Extent3D {
            width: size,
            height: size,
            depth: size,
        };
        let format = vk::Format::R32G32B32A32_SFLOAT;

        // 1. Create true 3D Vulkan Image
        let image_info = vk::ImageCreateInfo::default()
            .image_type(vk::ImageType::TYPE_3D)
            .format(format)
            .extent(extent)
            .mip_levels(1)
            .array_layers(1)
            .samples(vk::SampleCountFlags::TYPE_1)
            .tiling(vk::ImageTiling::OPTIMAL)
            .usage(vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST)
            .sharing_mode(vk::SharingMode::EXCLUSIVE)
            .initial_layout(vk::ImageLayout::UNDEFINED);

        let image = unsafe { device.create_image(&image_info, None) }
            .map_err(VulkanLutError::Vk)?;

        // 2. Allocate and bind Device Memory
        let req = unsafe { device.get_image_memory_requirements(image) };
        let props = unsafe {
            ctx.instance.get_physical_device_memory_properties(ctx.physical_device)
        };
        let mem_type_index = (0..props.memory_type_count)
            .find(|&i| {
                (req.memory_type_bits & (1 << i)) != 0
                    && props.memory_types[i as usize]
                        .property_flags
                        .contains(vk::MemoryPropertyFlags::DEVICE_LOCAL)
            })
            .ok_or(VulkanLutError::Vk(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY))?;

        let alloc_info = vk::MemoryAllocateInfo::default()
            .allocation_size(req.size)
            .memory_type_index(mem_type_index);

        let memory = match unsafe { device.allocate_memory(&alloc_info, None) } {
            Ok(mem) => mem,
            Err(err) => {
                unsafe { device.destroy_image(image, None) };
                return Err(VulkanLutError::Vk(err));
            }
        };

        if let Err(err) = unsafe { device.bind_image_memory(image, memory, 0) } {
            unsafe {
                device.destroy_image(image, None);
                device.free_memory(memory, None);
            }
            return Err(VulkanLutError::Vk(err));
        }

        // 3. Create 3D Image View
        let view_info = vk::ImageViewCreateInfo::default()
            .image(image)
            .view_type(vk::ImageViewType::TYPE_3D)
            .format(format)
            .subresource_range(vk::ImageSubresourceRange {
                aspect_mask: vk::ImageAspectFlags::COLOR,
                base_mip_level: 0,
                level_count: 1,
                base_array_layer: 0,
                layer_count: 1,
            });

        let image_view = match unsafe { device.create_image_view(&view_info, None) } {
            Ok(v) => v,
            Err(err) => {
                unsafe {
                    device.destroy_image(image, None);
                    device.free_memory(memory, None);
                }
                return Err(VulkanLutError::Vk(err));
            }
        };

        // 4. Create 3D Linear Sampler with ClampToEdge
        let sampler_info = vk::SamplerCreateInfo::default()
            .mag_filter(vk::Filter::LINEAR)
            .min_filter(vk::Filter::LINEAR)
            .mipmap_mode(vk::SamplerMipmapMode::NEAREST)
            .address_mode_u(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_v(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .address_mode_w(vk::SamplerAddressMode::CLAMP_TO_EDGE)
            .max_lod(0.0);

        let sampler = match unsafe { device.create_sampler(&sampler_info, None) } {
            Ok(s) => s,
            Err(err) => {
                unsafe {
                    device.destroy_image_view(image_view, None);
                    device.destroy_image(image, None);
                    device.free_memory(memory, None);
                }
                return Err(VulkanLutError::Vk(err));
            }
        };

        // 5. Upload data using a staging buffer
        let raw_bytes = lut.to_fp32_bytes();
        let staging = match ctx.acquire_staging_buffer(raw_bytes.len()) {
            Ok(s) => s,
            Err(err) => {
                unsafe {
                    device.destroy_sampler(sampler, None);
                    device.destroy_image_view(image_view, None);
                    device.destroy_image(image, None);
                    device.free_memory(memory, None);
                }
                return Err(VulkanLutError::Vk(err));
            }
        };

        // Map and copy data to staging
        unsafe {
            match device.map_memory(
                staging.memory,
                0,
                raw_bytes.len() as vk::DeviceSize,
                vk::MemoryMapFlags::empty(),
            ) {
                Ok(ptr) => {
                    std::ptr::copy_nonoverlapping(
                        raw_bytes.as_ptr(),
                        ptr.cast::<u8>(),
                        raw_bytes.len(),
                    );
                    device.unmap_memory(staging.memory);
                }
                Err(err) => {
                    ctx.release_staging_buffer(staging);
                    device.destroy_sampler(sampler, None);
                    device.destroy_image_view(image_view, None);
                    device.destroy_image(image, None);
                    device.free_memory(memory, None);
                    return Err(VulkanLutError::Vk(err));
                }
            }
        }

        // Record commands to transition layout and copy staging to 3D image
        let upload_result = (|| -> Result<(), vk::Result> {
            let _lock = ctx.command_lock.lock().unwrap();
            let pool_info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(ctx.graphics_queue_family)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT);
            let pool = unsafe { device.create_command_pool(&pool_info, None) }?;

            let alloc = vk::CommandBufferAllocateInfo::default()
                .command_pool(pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            let cmd = unsafe { device.allocate_command_buffers(&alloc) }?[0];

            unsafe {
                device.begin_command_buffer(
                    cmd,
                    &vk::CommandBufferBeginInfo::default()
                        .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
                )?;

                // Barrier 1: UNDEFINED -> TRANSFER_DST_OPTIMAL
                let before = vk::ImageMemoryBarrier::default()
                    .image(image)
                    .subresource_range(vk::ImageSubresourceRange {
                        aspect_mask: vk::ImageAspectFlags::COLOR,
                        base_mip_level: 0,
                        level_count: 1,
                        base_array_layer: 0,
                        layer_count: 1,
                    })
                    .old_layout(vk::ImageLayout::UNDEFINED)
                    .new_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .src_access_mask(vk::AccessFlags::empty())
                    .dst_access_mask(vk::AccessFlags::TRANSFER_WRITE);

                device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TOP_OF_PIPE,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[before],
                );

                // Copy buffer to 3D image
                let copy = vk::BufferImageCopy::default()
                    .buffer_offset(0)
                    .image_subresource(
                        vk::ImageSubresourceLayers::default()
                            .aspect_mask(vk::ImageAspectFlags::COLOR)
                            .layer_count(1),
                    )
                    .image_offset(vk::Offset3D::default())
                    .image_extent(extent);

                device.cmd_copy_buffer_to_image(
                    cmd,
                    staging.buffer,
                    image,
                    vk::ImageLayout::TRANSFER_DST_OPTIMAL,
                    &[copy],
                );

                // Barrier 2: TRANSFER_DST_OPTIMAL -> SHADER_READ_ONLY_OPTIMAL
                let after = before
                    .old_layout(vk::ImageLayout::TRANSFER_DST_OPTIMAL)
                    .new_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
                    .src_access_mask(vk::AccessFlags::TRANSFER_WRITE)
                    .dst_access_mask(vk::AccessFlags::SHADER_READ);

                device.cmd_pipeline_barrier(
                    cmd,
                    vk::PipelineStageFlags::TRANSFER,
                    vk::PipelineStageFlags::FRAGMENT_SHADER,
                    vk::DependencyFlags::empty(),
                    &[],
                    &[],
                    &[after],
                );

                device.end_command_buffer(cmd)?;

                let fence = device.create_fence(&vk::FenceCreateInfo::default(), None)?;
                let cmds = [cmd];
                let submit = vk::SubmitInfo::default().command_buffers(&cmds);
                device.queue_submit(ctx.graphics_queue, &[submit], fence)?;
                crate::wait_for_fences(device, &[fence])?;

                device.destroy_fence(fence, None);
                device.destroy_command_pool(pool, None);
            }
            Ok(())
        })();

        ctx.release_staging_buffer(staging);

        if let Err(err) = upload_result {
            unsafe {
                device.destroy_sampler(sampler, None);
                device.destroy_image_view(image_view, None);
                device.destroy_image(image, None);
                device.free_memory(memory, None);
            }
            return Err(VulkanLutError::Vk(err));
        }

        Ok(Self {
            ctx,
            image,
            image_view,
            sampler,
            memory,
            allocation_size: req.size,
            size,
            title: lut.title.clone(),
        })
    }

    /// Convenience helper to parse a `.cube` string and upload it directly as a 3D texture.
    pub fn from_cube_str(ctx: Arc<VulkanContext>, cube_str: &str) -> Result<Self, VulkanLutError> {
        let lut = Lut3D::from_cube_str(cube_str)?;
        Self::new(ctx, &lut)
    }

    /// Get descriptor image info for binding this 3D LUT to a Vulkan descriptor set.
    pub fn descriptor_image_info(&self) -> vk::DescriptorImageInfo {
        vk::DescriptorImageInfo::default()
            .sampler(self.sampler)
            .image_view(self.image_view)
            .image_layout(vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL)
    }

    /// The underlying raw `VkImage`.
    pub fn image(&self) -> vk::Image {
        self.image
    }

    /// The underlying raw `VkImageView` (with `VkImageViewType::TYPE_3D`).
    pub fn image_view(&self) -> vk::ImageView {
        self.image_view
    }

    /// The underlying linear trilinear `VkSampler`.
    pub fn sampler(&self) -> vk::Sampler {
        self.sampler
    }

    /// The dimension $N$ along each edge of the 3D LUT.
    pub fn size(&self) -> u32 {
        self.size
    }

    /// Optional title or description associated with this 3D LUT.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    /// Memory footprint allocated on the GPU in bytes.
    pub fn allocation_size(&self) -> vk::DeviceSize {
        self.allocation_size
    }
}

impl Drop for VulkanLut3D {
    fn drop(&mut self) {
        unsafe {
            self.ctx.device.destroy_sampler(self.sampler, None);
            self.ctx.device.destroy_image_view(self.image_view, None);
            self.ctx.device.destroy_image(self.image, None);
            self.ctx.device.free_memory(self.memory, None);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identity_lut_generation_and_sampling() {
        let lut = Lut3D::identity(17);
        assert_eq!(lut.size, 17);
        assert_eq!(lut.data.len(), 17 * 17 * 17);

        // Black corner
        let black = lut.sample_trilinear(0.0, 0.0, 0.0);
        assert!((black[0] - 0.0).abs() < 1e-4);
        assert!((black[1] - 0.0).abs() < 1e-4);
        assert!((black[2] - 0.0).abs() < 1e-4);

        // White corner
        let white = lut.sample_trilinear(1.0, 1.0, 1.0);
        assert!((white[0] - 1.0).abs() < 1e-4);
        assert!((white[1] - 1.0).abs() < 1e-4);
        assert!((white[2] - 1.0).abs() < 1e-4);

        // Mid gray
        let mid = lut.sample_trilinear(0.5, 0.5, 0.5);
        assert!((mid[0] - 0.5).abs() < 1e-4);
        assert!((mid[1] - 0.5).abs() < 1e-4);
        assert!((mid[2] - 0.5).abs() < 1e-4);

        // Arbitrary point
        let sample = lut.sample_trilinear(0.25, 0.75, 0.33);
        assert!((sample[0] - 0.25).abs() < 1e-3);
        assert!((sample[1] - 0.75).abs() < 1e-3);
        assert!((sample[2] - 0.33).abs() < 1e-3);
    }

    #[test]
    fn test_cube_parser_with_sample_lut() {
        let sample_cube = r#"
# Sample test LUT
TITLE "Warm Cinematic Tone"
LUT_3D_SIZE 2
DOMAIN_MIN 0.0 0.0 0.0
DOMAIN_MAX 1.0 1.0 1.0

0.0 0.0 0.0
1.0 0.0 0.0
0.0 1.0 0.0
1.0 1.0 0.0
0.0 0.0 1.0
1.0 0.0 1.0
0.0 1.0 1.0
1.0 1.0 1.0
"#;
        let lut = Lut3D::from_cube_str(sample_cube).expect("failed to parse valid .cube");
        assert_eq!(lut.size, 2);
        assert_eq!(lut.title.as_deref(), Some("Warm Cinematic Tone"));
        assert_eq!(lut.data.len(), 8);

        // Test sample
        let red = lut.sample_trilinear(1.0, 0.0, 0.0);
        assert_eq!(red, [1.0, 0.0, 0.0]);
    }

    #[test]
    fn test_invalid_cube_error_handling() {
        // Missing size
        let missing_size = "0.0 0.0 0.0\n";
        assert!(matches!(
            Lut3D::from_cube_str(missing_size),
            Err(LutError::ParseError(_))
        ));

        // Data count mismatch
        let mismatch = "LUT_3D_SIZE 2\n0.0 0.0 0.0\n";
        assert!(matches!(
            Lut3D::from_cube_str(mismatch),
            Err(LutError::DataMismatch { .. })
        ));

        // Invalid size
        let invalid_size = "LUT_3D_SIZE 1\n";
        assert!(matches!(
            Lut3D::from_cube_str(invalid_size),
            Err(LutError::InvalidSize(1))
        ));
    }
}
