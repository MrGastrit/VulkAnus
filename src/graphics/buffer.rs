use std::rc::Rc;

use anyhow::{Context, Result, ensure};
use ash::vk;
use bytemuck::{Pod, Zeroable};
use vk_mem::Alloc;

use super::{device::Device, frame::Frame};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Vertex {
    pub position: [f32; 2],
    pub color: [f32; 3],
}

pub const TRIANGLE: [Vertex; 3] = [
    Vertex {
        position: [-0.65, -0.55],
        color: [1.0, 0.15, 0.15],
    },
    Vertex {
        position: [0.65, -0.55],
        color: [0.15, 1.0, 0.25],
    },
    Vertex {
        position: [0.0, 0.65],
        color: [0.15, 0.35, 1.0],
    },
];

pub struct Allocator {
    // Field order matters: VMA is destroyed before the final device reference.
    pub raw: vk_mem::Allocator,
    pub device: Rc<Device>,
}

impl Allocator {
    pub fn new(device: Rc<Device>) -> Result<Rc<Self>> {
        let mut info = vk_mem::AllocatorCreateInfo::new(
            &device.surface.instance.raw,
            &device.raw,
            device.physical,
        );
        info.vulkan_api_version = vk::API_VERSION_1_3;
        let raw = unsafe { vk_mem::Allocator::new(info) }.context("Failed to initialize VMA")?;
        Ok(Rc::new(Self { raw, device }))
    }
}

pub struct Buffer {
    pub handle: vk::Buffer,
    allocation: vk_mem::Allocation,
    allocator: Rc<Allocator>,
}

impl Buffer {
    fn new(
        allocator: Rc<Allocator>,
        size: vk::DeviceSize,
        usage: vk::BufferUsageFlags,
        host_visible: bool,
    ) -> Result<Self> {
        ensure!(size > 0, "Cannot create an empty buffer");
        let buffer = vk::BufferCreateInfo::default()
            .size(size)
            .usage(usage)
            .sharing_mode(vk::SharingMode::EXCLUSIVE);
        let allocation_info = vk_mem::AllocationCreateInfo {
            usage: if host_visible {
                vk_mem::MemoryUsage::AutoPreferHost
            } else {
                vk_mem::MemoryUsage::AutoPreferDevice
            },
            flags: if host_visible {
                vk_mem::AllocationCreateFlags::HOST_ACCESS_SEQUENTIAL_WRITE
            } else {
                vk_mem::AllocationCreateFlags::empty()
            },
            ..Default::default()
        };
        let (handle, allocation) =
            unsafe { allocator.raw.create_buffer(&buffer, &allocation_info) }
                .context("VMA buffer allocation failed")?;
        Ok(Self {
            handle,
            allocation,
            allocator,
        })
    }

    pub fn upload_vertices(allocator: Rc<Allocator>, vertices: &[Vertex]) -> Result<Self> {
        let bytes = bytemuck::cast_slice(vertices);
        let size = bytes.len() as vk::DeviceSize;
        let mut staging = Self::new(
            allocator.clone(),
            size,
            vk::BufferUsageFlags::TRANSFER_SRC,
            true,
        )?;
        unsafe {
            let mapped = allocator
                .raw
                .map_memory(&mut staging.allocation)
                .context("Cannot map staging buffer")?;
            std::ptr::copy_nonoverlapping(bytes.as_ptr(), mapped, bytes.len());
            let flushed = allocator.raw.flush_allocation(&staging.allocation, 0, size);
            allocator.raw.unmap_memory(&mut staging.allocation);
            flushed.context("Cannot flush staging memory")?;
        }
        let gpu = Self::new(
            allocator.clone(),
            size,
            vk::BufferUsageFlags::TRANSFER_DST | vk::BufferUsageFlags::VERTEX_BUFFER,
            false,
        )?;
        let upload = Frame::new(allocator.device.clone())?;
        upload.begin()?;
        let device = &allocator.device.raw;
        unsafe {
            device.cmd_copy_buffer(
                upload.command,
                staging.handle,
                gpu.handle,
                &[vk::BufferCopy::default().size(size)],
            );
            let barrier = [vk::BufferMemoryBarrier2::default()
                .src_stage_mask(vk::PipelineStageFlags2::TRANSFER)
                .src_access_mask(vk::AccessFlags2::TRANSFER_WRITE)
                .dst_stage_mask(vk::PipelineStageFlags2::VERTEX_ATTRIBUTE_INPUT)
                .dst_access_mask(vk::AccessFlags2::VERTEX_ATTRIBUTE_READ)
                .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
                .buffer(gpu.handle)
                .offset(0)
                .size(size)];
            device.cmd_pipeline_barrier2(
                upload.command,
                &vk::DependencyInfo::default().buffer_memory_barriers(&barrier),
            );
            device.end_command_buffer(upload.command)?;
            device.reset_fences(&[upload.fence])?;
            let commands = [vk::CommandBufferSubmitInfo::default().command_buffer(upload.command)];
            let submits = [vk::SubmitInfo2::default().command_buffer_infos(&commands)];
            device
                .queue_submit2(allocator.device.graphics_queue, &submits, upload.fence)
                .context("Vertex upload submission failed")?;
        }
        // Staging memory and upload commands remain alive until the transfer finishes.
        if let Err(error) = upload.wait() {
            let _ = allocator.device.wait_idle();
            return Err(error);
        }
        Ok(gpu)
    }
}

impl Drop for Buffer {
    fn drop(&mut self) {
        unsafe {
            self.allocator
                .raw
                .destroy_buffer(self.handle, &mut self.allocation)
        };
    }
}
