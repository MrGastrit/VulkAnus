use std::{ffi::CStr, rc::Rc};

use anyhow::{Context, Result};
use ash::{khr, vk};

use super::surface::Surface;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QueueFamilies {
    pub graphics: u32,
    pub present: u32,
}

pub struct Device {
    pub raw: ash::Device,
    pub physical: vk::PhysicalDevice,
    pub graphics_queue: vk::Queue,
    pub present_queue: vk::Queue,
    pub families: QueueFamilies,
    pub surface: Rc<Surface>,
}

impl Device {
    pub fn new(surface: Rc<Surface>) -> Result<Rc<Self>> {
        let instance = &surface.instance.raw;
        let mut candidates = Vec::new();
        for physical in unsafe { instance.enumerate_physical_devices() }? {
            let properties = unsafe { instance.get_physical_device_properties(physical) };
            if properties.api_version < vk::API_VERSION_1_3 {
                continue;
            }
            let extensions = unsafe { instance.enumerate_device_extension_properties(physical) }?;
            if !extensions.iter().any(|extension| unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) } == khr::swapchain::NAME) { continue; }
            let mut features13 = vk::PhysicalDeviceVulkan13Features::default();
            let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut features13);
            unsafe { instance.get_physical_device_features2(physical, &mut features) };
            if features13.dynamic_rendering == vk::FALSE || features13.synchronization2 == vk::FALSE
            {
                continue;
            }
            let properties_queue =
                unsafe { instance.get_physical_device_queue_family_properties(physical) };
            let mut support = Vec::new();
            for (index, family) in properties_queue.iter().enumerate() {
                let present = unsafe {
                    surface.loader.get_physical_device_surface_support(
                        physical,
                        index as u32,
                        surface.handle,
                    )
                }?;
                support.push((
                    family.queue_count > 0 && family.queue_flags.contains(vk::QueueFlags::GRAPHICS),
                    family.queue_count > 0 && present,
                ));
            }
            let Some(families) = choose_queue_families(&support) else {
                continue;
            };
            let formats = unsafe {
                surface
                    .loader
                    .get_physical_device_surface_formats(physical, surface.handle)
            }?;
            let modes = unsafe {
                surface
                    .loader
                    .get_physical_device_surface_present_modes(physical, surface.handle)
            }?;
            let capabilities = unsafe {
                surface
                    .loader
                    .get_physical_device_surface_capabilities(physical, surface.handle)
            }?;
            if formats.is_empty()
                || modes.is_empty()
                || !capabilities
                    .supported_usage_flags
                    .contains(vk::ImageUsageFlags::COLOR_ATTACHMENT)
            {
                continue;
            }
            let score = match properties.device_type {
                vk::PhysicalDeviceType::DISCRETE_GPU => 1000,
                vk::PhysicalDeviceType::INTEGRATED_GPU => 500,
                vk::PhysicalDeviceType::VIRTUAL_GPU => 100,
                _ => 10,
            };
            candidates.push((score, physical, families, properties));
        }
        let (_, physical, families, properties) = candidates.into_iter().max_by_key(|candidate| candidate.0)
            .context("No suitable GPU: Vulkan 1.3, dynamicRendering, synchronization2, graphics/present queues and VK_KHR_swapchain are required")?;
        let priorities = [1.0];
        let mut queue_infos = vec![
            vk::DeviceQueueCreateInfo::default()
                .queue_family_index(families.graphics)
                .queue_priorities(&priorities),
        ];
        if families.present != families.graphics {
            queue_infos.push(
                vk::DeviceQueueCreateInfo::default()
                    .queue_family_index(families.present)
                    .queue_priorities(&priorities),
            );
        }
        let extensions = [khr::swapchain::NAME.as_ptr()];
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true);
        let create_info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queue_infos)
            .enabled_extension_names(&extensions)
            .push_next(&mut features13);
        let raw = unsafe { instance.create_device(physical, &create_info, None) }
            .context("Failed to create logical device")?;
        let graphics_queue = unsafe { raw.get_device_queue(families.graphics, 0) };
        let present_queue = unsafe { raw.get_device_queue(families.present, 0) };
        log::info!(
            "GPU: {}; queues: {families:?}",
            unsafe { CStr::from_ptr(properties.device_name.as_ptr()) }.to_string_lossy()
        );
        Ok(Rc::new(Self {
            raw,
            physical,
            graphics_queue,
            present_queue,
            families,
            surface,
        }))
    }

    pub fn wait_idle(&self) -> Result<()> {
        unsafe { self.raw.device_wait_idle() }.context("Failed to wait for the GPU")
    }
}

impl Drop for Device {
    fn drop(&mut self) {
        // All resource wrappers retain this device. Renderer waits before dropping them.
        unsafe { self.raw.destroy_device(None) };
    }
}

fn choose_queue_families(support: &[(bool, bool)]) -> Option<QueueFamilies> {
    if let Some(index) = support
        .iter()
        .position(|&(graphics, present)| graphics && present)
    {
        return Some(QueueFamilies {
            graphics: index as u32,
            present: index as u32,
        });
    }
    Some(QueueFamilies {
        graphics: support.iter().position(|&(graphics, _)| graphics)? as u32,
        present: support.iter().position(|&(_, present)| present)? as u32,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefers_a_shared_family_and_supports_separate_present() {
        assert_eq!(
            choose_queue_families(&[(true, false), (true, true)]),
            Some(QueueFamilies {
                graphics: 1,
                present: 1
            })
        );
        assert_eq!(
            choose_queue_families(&[(true, false), (false, true)]),
            Some(QueueFamilies {
                graphics: 0,
                present: 1
            })
        );
        assert_eq!(choose_queue_families(&[(true, false)]), None);
        assert_eq!(choose_queue_families(&[]), None);
    }
}
