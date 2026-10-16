use std::rc::Rc;

use anyhow::{Context, Result};
use ash::vk;

use super::device::Device;

pub struct Frame {
    pub command: vk::CommandBuffer,
    pub image_available: vk::Semaphore,
    pub fence: vk::Fence,
    pool: vk::CommandPool,
    device: Rc<Device>,
}

impl Frame {
    pub fn new(device: Rc<Device>) -> Result<Self> {
        // Start with null handles so every partial initialization path is covered by Drop.
        let mut frame = Self {
            command: vk::CommandBuffer::null(),
            image_available: vk::Semaphore::null(),
            fence: vk::Fence::null(),
            pool: vk::CommandPool::null(),
            device,
        };
        unsafe {
            let info = vk::CommandPoolCreateInfo::default()
                .queue_family_index(frame.device.families.graphics)
                .flags(vk::CommandPoolCreateFlags::TRANSIENT);
            frame.pool = frame.device.raw.create_command_pool(&info, None)?;
            let allocation = vk::CommandBufferAllocateInfo::default()
                .command_pool(frame.pool)
                .level(vk::CommandBufferLevel::PRIMARY)
                .command_buffer_count(1);
            frame.command = frame.device.raw.allocate_command_buffers(&allocation)?[0];
            frame.image_available = frame
                .device
                .raw
                .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?;
            frame.fence = frame.device.raw.create_fence(
                &vk::FenceCreateInfo::default().flags(vk::FenceCreateFlags::SIGNALED),
                None,
            )?;
        }
        Ok(frame)
    }

    pub fn begin(&self) -> Result<()> {
        unsafe {
            self.device
                .raw
                .reset_command_pool(self.pool, vk::CommandPoolResetFlags::empty())?;
            self.device.raw.begin_command_buffer(
                self.command,
                &vk::CommandBufferBeginInfo::default()
                    .flags(vk::CommandBufferUsageFlags::ONE_TIME_SUBMIT),
            )?;
        }
        Ok(())
    }

    pub fn wait(&self) -> Result<()> {
        unsafe {
            self.device
                .raw
                .wait_for_fences(&[self.fence], true, u64::MAX)
        }
        .context("Frame fence wait failed")
    }
}

impl Drop for Frame {
    fn drop(&mut self) {
        // The caller waits for submissions before releasing a frame or its command pool.
        unsafe {
            self.device.raw.destroy_fence(self.fence, None);
            self.device
                .raw
                .destroy_semaphore(self.image_available, None);
            self.device.raw.destroy_command_pool(self.pool, None);
        }
    }
}
