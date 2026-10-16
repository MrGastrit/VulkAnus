use std::rc::Rc;

use anyhow::{Context, Result};
use ash::{
    khr,
    vk::{self, Handle},
};

use super::instance::Instance;

pub struct Surface {
    pub loader: khr::surface::Instance,
    pub handle: vk::SurfaceKHR,
    pub instance: Rc<Instance>,
}

impl Surface {
    // The application owns the GLFW window and destroys the renderer first.
    pub fn new(instance: Rc<Instance>, window: &glfw::PWindow) -> Result<Rc<Self>> {
        let mut handle = std::ptr::null_mut();
        let result = unsafe {
            window.create_window_surface(
                instance.raw.handle().as_raw() as _,
                std::ptr::null(),
                &mut handle,
            )
        };
        if result != vk::Result::SUCCESS.as_raw() {
            return Err(vk::Result::from_raw(result))
                .context("GLFW could not create a Vulkan surface");
        }
        let loader = khr::surface::Instance::new(&instance.entry, &instance.raw);
        Ok(Rc::new(Self {
            loader,
            handle: vk::SurfaceKHR::from_raw(handle as u64),
            instance,
        }))
    }
}

impl Drop for Surface {
    fn drop(&mut self) {
        unsafe { self.loader.destroy_surface(self.handle, None) };
    }
}
