use std::{rc::Rc, sync::Arc};

use anyhow::{Context, Result};
use ash::vk;

use super::{
    buffer::{Allocator, Buffer, TRIANGLE},
    device::Device,
    frame::Frame,
    instance::{Instance, ValidationState},
    pipeline::Pipeline,
    surface::Surface,
    swapchain::{Swapchain, SwapchainConfig, color_range},
};
use crate::{config::ValidationMode, window::Window};

const FRAMES_IN_FLIGHT: usize = 2;

pub struct Renderer {
    frames: Vec<Frame>,
    pipeline: Option<Pipeline>,
    vertices: Buffer,
    swapchain: Option<Swapchain>,
    device: Rc<Device>,
    current_frame: usize,
    surface_changed: bool,
    invalid_swapchain: bool,
}

impl Renderer {
    pub fn new(window: &Window, validation: ValidationMode) -> Result<Self> {
        let instance = Instance::new(&window.required_extensions()?, validation)?;
        let surface = Surface::new(instance, &window.raw)?;
        let device = Device::new(surface)?;
        let allocator = Allocator::new(device.clone())?;
        let vertices = Buffer::upload_vertices(allocator, &TRIANGLE)?;
        let frames = (0..FRAMES_IN_FLIGHT)
            .map(|_| Frame::new(device.clone()))
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            frames,
            pipeline: None,
            vertices,
            swapchain: None,
            device,
            current_frame: 0,
            surface_changed: true,
            invalid_swapchain: false,
        })
    }

    pub fn validation_state(&self) -> Arc<ValidationState> {
        self.device.surface.instance.validation.clone()
    }

    pub fn request_resize(&mut self) {
        self.surface_changed = true;
    }

    pub fn wait_idle(&self) -> Result<()> {
        self.device.wait_idle()
    }

    fn update_swapchain(&mut self, extent: vk::Extent2D) -> Result<bool> {
        let config = SwapchainConfig::query(&self.device, extent)?;
        if config.extent.width == 0 || config.extent.height == 0 {
            return Ok(false);
        }
        let changed = self
            .swapchain
            .as_ref()
            .is_none_or(|swapchain| swapchain.config != config);
        if changed || self.invalid_swapchain {
            self.device.wait_idle()?;
            let old_handle = self
                .swapchain
                .as_ref()
                .map_or(vk::SwapchainKHR::null(), |swapchain| swapchain.handle);
            let replacement = Swapchain::new(self.device.clone(), config, old_handle)?;
            if self
                .swapchain
                .as_ref()
                .is_none_or(|swapchain| swapchain.config.format.format != config.format.format)
            {
                self.pipeline = Some(Pipeline::new(self.device.clone(), config.format.format)?);
            }
            // The replacement owns its views/semaphores before the retired chain is dropped.
            self.swapchain = Some(replacement);
        }
        self.invalid_swapchain = false;
        self.surface_changed = false;
        Ok(true)
    }

    /// Returns false when no image could be rendered (minimized or out of date).
    pub fn draw(&mut self, extent: vk::Extent2D) -> Result<bool> {
        if extent.width == 0 || extent.height == 0 {
            return Ok(false);
        }
        if (self.surface_changed || self.invalid_swapchain || self.swapchain.is_none())
            && !self.update_swapchain(extent)?
        {
            return Ok(false);
        }
        let swapchain = self
            .swapchain
            .as_ref()
            .context("Swapchain was not initialized")?;
        let pipeline = self
            .pipeline
            .as_ref()
            .context("Pipeline was not initialized")?;
        let frame = &self.frames[self.current_frame];
        frame.wait()?;
        let (image_index, acquire_suboptimal) = match unsafe {
            swapchain.loader.acquire_next_image(
                swapchain.handle,
                u64::MAX,
                frame.image_available,
                vk::Fence::null(),
            )
        } {
            Ok(acquired) => acquired,
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                // No submission follows: leave the signaled fence untouched.
                self.invalid_swapchain = true;
                return Ok(false);
            }
            Err(error) => return Err(error).context("Failed to acquire a swapchain image"),
        };
        let image_index = image_index as usize;
        frame.begin()?;
        self.record(frame.command, swapchain, pipeline, image_index)?;
        let finished = swapchain.render_finished[image_index];
        unsafe {
            let waits = [vk::SemaphoreSubmitInfo::default()
                .semaphore(frame.image_available)
                .stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)];
            let signals = [vk::SemaphoreSubmitInfo::default()
                .semaphore(finished)
                .stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)];
            let commands = [vk::CommandBufferSubmitInfo::default().command_buffer(frame.command)];
            let submits = [vk::SubmitInfo2::default()
                .wait_semaphore_infos(&waits)
                .command_buffer_infos(&commands)
                .signal_semaphore_infos(&signals)];
            // Reset only once acquisition and recording succeeded and a submission is ready.
            self.device.raw.reset_fences(&[frame.fence])?;
            self.device
                .raw
                .queue_submit2(self.device.graphics_queue, &submits, frame.fence)
                .context("Graphics submission failed")?;
        }
        let wait_semaphores = [finished];
        let swapchains = [swapchain.handle];
        let indices = [image_index as u32];
        let present_info = vk::PresentInfoKHR::default()
            .wait_semaphores(&wait_semaphores)
            .swapchains(&swapchains)
            .image_indices(&indices);
        let presented = match unsafe {
            swapchain
                .loader
                .queue_present(self.device.present_queue, &present_info)
        } {
            Ok(present_suboptimal) => {
                // SUBOPTIMAL is usable. Re-query configuration; unchanged configurations do
                // not recreate forever on platforms that keep reporting SUBOPTIMAL.
                self.surface_changed |= acquire_suboptimal || present_suboptimal;
                true
            }
            Err(vk::Result::ERROR_OUT_OF_DATE_KHR) => {
                self.invalid_swapchain = true;
                false
            }
            Err(error) => return Err(error).context("Presentation failed"),
        };
        self.current_frame = (self.current_frame + 1) % self.frames.len();
        Ok(presented)
    }

    fn record(
        &self,
        command: vk::CommandBuffer,
        swapchain: &Swapchain,
        pipeline: &Pipeline,
        index: usize,
    ) -> Result<()> {
        let device = &self.device.raw;
        let extent = swapchain.config.extent;
        let to_color = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .dst_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            .old_layout(vk::ImageLayout::UNDEFINED)
            .new_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(swapchain.images[index])
            .subresource_range(color_range())];
        let attachments = [vk::RenderingAttachmentInfo::default()
            .image_view(swapchain.views[index])
            .image_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .load_op(vk::AttachmentLoadOp::CLEAR)
            .store_op(vk::AttachmentStoreOp::STORE)
            .clear_value(vk::ClearValue {
                color: vk::ClearColorValue {
                    float32: [0.015, 0.02, 0.035, 1.0],
                },
            })];
        let area = vk::Rect2D {
            offset: vk::Offset2D::default(),
            extent,
        };
        let rendering = vk::RenderingInfo::default()
            .render_area(area)
            .layer_count(1)
            .color_attachments(&attachments);
        let to_present = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::COLOR_ATTACHMENT_OUTPUT)
            .src_access_mask(vk::AccessFlags2::COLOR_ATTACHMENT_WRITE)
            // Keep the layout transition in the execution chain of the semaphore signal.
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .old_layout(vk::ImageLayout::COLOR_ATTACHMENT_OPTIMAL)
            .new_layout(vk::ImageLayout::PRESENT_SRC_KHR)
            .src_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .dst_queue_family_index(vk::QUEUE_FAMILY_IGNORED)
            .image(swapchain.images[index])
            .subresource_range(color_range())];
        unsafe {
            device.cmd_pipeline_barrier2(
                command,
                &vk::DependencyInfo::default().image_memory_barriers(&to_color),
            );
            device.cmd_begin_rendering(command, &rendering);
            device.cmd_bind_pipeline(command, vk::PipelineBindPoint::GRAPHICS, pipeline.handle);
            // A centered square viewport preserves the triangle's shape while resizing.
            let side = extent.width.min(extent.height) as f32;
            let viewport = vk::Viewport {
                x: (extent.width as f32 - side) * 0.5,
                y: (extent.height as f32 - side) * 0.5,
                width: side,
                height: side,
                min_depth: 0.0,
                max_depth: 1.0,
            };
            device.cmd_set_viewport(command, 0, &[viewport]);
            device.cmd_set_scissor(command, 0, &[area]);
            device.cmd_bind_vertex_buffers(command, 0, &[self.vertices.handle], &[0]);
            device.cmd_draw(command, TRIANGLE.len() as u32, 1, 0, 0);
            device.cmd_end_rendering(command);
            device.cmd_pipeline_barrier2(
                command,
                &vk::DependencyInfo::default().image_memory_barriers(&to_present),
            );
            device.end_command_buffer(command)?;
        }
        Ok(())
    }
}

impl Drop for Renderer {
    fn drop(&mut self) {
        // Drop runs on normal exit and every error path before resource fields are released.
        if let Err(error) = self.device.wait_idle() {
            log::error!("GPU shutdown: {error:#}");
        }
    }
}
