use std::rc::Rc;

use anyhow::{Context, Result, ensure};
use ash::{khr, vk};

use super::device::Device;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SwapchainConfig {
    pub format: vk::SurfaceFormatKHR,
    pub extent: vk::Extent2D,
    pub image_count: u32,
    pub transform: vk::SurfaceTransformFlagsKHR,
    pub composite_alpha: vk::CompositeAlphaFlagsKHR,
}

impl SwapchainConfig {
    pub fn query(device: &Device, requested: vk::Extent2D) -> Result<Self> {
        let surface = &device.surface;
        let capabilities = unsafe {
            surface
                .loader
                .get_physical_device_surface_capabilities(device.physical, surface.handle)
        }?;
        let formats = unsafe {
            surface
                .loader
                .get_physical_device_surface_formats(device.physical, surface.handle)
        }?;
        ensure!(
            capabilities
                .supported_usage_flags
                .contains(vk::ImageUsageFlags::COLOR_ATTACHMENT),
            "Surface does not support color attachments"
        );
        let composite_alpha = [
            vk::CompositeAlphaFlagsKHR::OPAQUE,
            vk::CompositeAlphaFlagsKHR::PRE_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::POST_MULTIPLIED,
            vk::CompositeAlphaFlagsKHR::INHERIT,
        ]
        .into_iter()
        .find(|flag| capabilities.supported_composite_alpha.contains(*flag))
        .context("Surface has no supported composite alpha mode")?;
        Ok(Self {
            format: choose_format(&formats).context("Surface has no supported formats")?,
            extent: choose_extent(&capabilities, requested),
            image_count: choose_image_count(&capabilities),
            transform: capabilities.current_transform,
            composite_alpha,
        })
    }
}

pub struct Swapchain {
    pub loader: khr::swapchain::Device,
    pub handle: vk::SwapchainKHR,
    pub config: SwapchainConfig,
    pub images: Vec<vk::Image>,
    pub views: Vec<vk::ImageView>,
    // Presentation completion is associated with an acquired image, not a frame fence.
    pub render_finished: Vec<vk::Semaphore>,
    device: Rc<Device>,
}

impl Swapchain {
    pub fn new(device: Rc<Device>, config: SwapchainConfig, old: vk::SwapchainKHR) -> Result<Self> {
        ensure!(
            config.extent.width > 0 && config.extent.height > 0,
            "Cannot create a zero-sized swapchain"
        );
        let loader = khr::swapchain::Device::new(&device.surface.instance.raw, &device.raw);
        let mut swapchain = Self {
            loader,
            handle: vk::SwapchainKHR::null(),
            config,
            images: Vec::new(),
            views: Vec::new(),
            render_finished: Vec::new(),
            device,
        };
        let families = [
            swapchain.device.families.graphics,
            swapchain.device.families.present,
        ];
        let mut create_info = vk::SwapchainCreateInfoKHR::default()
            .surface(swapchain.device.surface.handle)
            .min_image_count(config.image_count)
            .image_format(config.format.format)
            .image_color_space(config.format.color_space)
            .image_extent(config.extent)
            .image_array_layers(1)
            .image_usage(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            .pre_transform(config.transform)
            .composite_alpha(config.composite_alpha)
            .present_mode(vk::PresentModeKHR::FIFO)
            .clipped(true)
            .old_swapchain(old);
        if families[0] == families[1] {
            create_info = create_info.image_sharing_mode(vk::SharingMode::EXCLUSIVE);
        } else {
            // Concurrent sharing removes the need for graphics/present ownership transfers.
            create_info = create_info
                .image_sharing_mode(vk::SharingMode::CONCURRENT)
                .queue_family_indices(&families);
        }
        unsafe {
            swapchain.handle = swapchain
                .loader
                .create_swapchain(&create_info, None)
                .context("Failed to create swapchain")?;
            swapchain.images = swapchain.loader.get_swapchain_images(swapchain.handle)?;
            for &image in &swapchain.images {
                let view_info = vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(config.format.format)
                    .subresource_range(color_range());
                swapchain
                    .views
                    .push(swapchain.device.raw.create_image_view(&view_info, None)?);
                swapchain.render_finished.push(
                    swapchain
                        .device
                        .raw
                        .create_semaphore(&vk::SemaphoreCreateInfo::default(), None)?,
                );
            }
        }
        log::info!(
            "Swapchain: {}x{}, {:?}, {} images, FIFO",
            config.extent.width,
            config.extent.height,
            config.format.format,
            swapchain.images.len()
        );
        Ok(swapchain)
    }
}

impl Drop for Swapchain {
    fn drop(&mut self) {
        // Renderer drains the device before swapchain replacement or shutdown.
        unsafe {
            for &semaphore in &self.render_finished {
                self.device.raw.destroy_semaphore(semaphore, None);
            }
            for &view in &self.views {
                self.device.raw.destroy_image_view(view, None);
            }
            self.loader.destroy_swapchain(self.handle, None);
        }
    }
}

pub fn color_range() -> vk::ImageSubresourceRange {
    vk::ImageSubresourceRange::default()
        .aspect_mask(vk::ImageAspectFlags::COLOR)
        .base_mip_level(0)
        .level_count(1)
        .base_array_layer(0)
        .layer_count(1)
}

fn choose_format(formats: &[vk::SurfaceFormatKHR]) -> Option<vk::SurfaceFormatKHR> {
    if formats.len() == 1 && formats[0].format == vk::Format::UNDEFINED {
        return Some(vk::SurfaceFormatKHR {
            format: vk::Format::B8G8R8A8_SRGB,
            color_space: formats[0].color_space,
        });
    }
    for preferred in [vk::Format::B8G8R8A8_SRGB, vk::Format::R8G8B8A8_SRGB] {
        if let Some(format) = formats.iter().find(|format| {
            format.format == preferred && format.color_space == vk::ColorSpaceKHR::SRGB_NONLINEAR
        }) {
            return Some(*format);
        }
    }
    formats.first().copied()
}

fn choose_extent(
    capabilities: &vk::SurfaceCapabilitiesKHR,
    requested: vk::Extent2D,
) -> vk::Extent2D {
    if capabilities.current_extent.width != u32::MAX {
        return capabilities.current_extent;
    }
    vk::Extent2D {
        width: requested.width.clamp(
            capabilities.min_image_extent.width,
            capabilities.max_image_extent.width,
        ),
        height: requested.height.clamp(
            capabilities.min_image_extent.height,
            capabilities.max_image_extent.height,
        ),
    }
}

fn choose_image_count(capabilities: &vk::SurfaceCapabilitiesKHR) -> u32 {
    let count = capabilities.min_image_count.saturating_add(1);
    if capabilities.max_image_count == 0 {
        count
    } else {
        count.min(capabilities.max_image_count)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extent_respects_fixed_surface_and_variable_limits() {
        let mut caps = vk::SurfaceCapabilitiesKHR::default()
            .current_extent(vk::Extent2D {
                width: u32::MAX,
                height: u32::MAX,
            })
            .min_image_extent(vk::Extent2D {
                width: 64,
                height: 64,
            })
            .max_image_extent(vk::Extent2D {
                width: 4096,
                height: 4096,
            });
        assert_eq!(
            choose_extent(
                &caps,
                vk::Extent2D {
                    width: 1,
                    height: 8000
                }
            ),
            vk::Extent2D {
                width: 64,
                height: 4096
            }
        );
        caps.current_extent = vk::Extent2D {
            width: 800,
            height: 600,
        };
        assert_eq!(
            choose_extent(
                &caps,
                vk::Extent2D {
                    width: 1280,
                    height: 720
                }
            ),
            caps.current_extent
        );
    }

    #[test]
    fn image_count_handles_unlimited_and_bounded_surfaces() {
        let mut caps = vk::SurfaceCapabilitiesKHR::default()
            .min_image_count(2)
            .max_image_count(0);
        assert_eq!(choose_image_count(&caps), 3);
        caps.max_image_count = 2;
        assert_eq!(choose_image_count(&caps), 2);
    }

    #[test]
    fn format_selection_prefers_srgb_and_handles_undefined() {
        let fallback = vk::SurfaceFormatKHR {
            format: vk::Format::B8G8R8A8_UNORM,
            color_space: vk::ColorSpaceKHR::SRGB_NONLINEAR,
        };
        let preferred = vk::SurfaceFormatKHR {
            format: vk::Format::B8G8R8A8_SRGB,
            ..fallback
        };
        assert_eq!(choose_format(&[fallback, preferred]), Some(preferred));
        assert_eq!(
            choose_format(&[vk::SurfaceFormatKHR {
                format: vk::Format::UNDEFINED,
                ..fallback
            }]),
            Some(preferred)
        );
        assert_eq!(choose_format(&[]), None);
    }
}
