use std::{
    ffi::{CStr, CString, c_void},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use anyhow::{Context, Result, ensure};
use ash::{Entry, ext, vk};

use crate::config::ValidationMode;

#[derive(Default)]
pub struct ValidationState {
    pub errors: AtomicUsize,
}

pub struct Instance {
    pub raw: ash::Instance,
    pub validation: Arc<ValidationState>,
    debug: Option<(ext::debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    // Dynamic Vulkan entry points must remain loaded until after vkDestroyInstance.
    pub entry: Entry,
}

impl Instance {
    pub fn new(required_extensions: &[String], mode: ValidationMode) -> Result<Rc<Self>> {
        // SAFETY: ash owns the loader library and it is kept alive by this object.
        let entry = unsafe { Entry::load() }
            .context("Cannot load Vulkan; install a Vulkan-capable graphics driver")?;
        let api_version =
            unsafe { entry.try_enumerate_instance_version() }?.unwrap_or(vk::API_VERSION_1_0);
        ensure!(
            api_version >= vk::API_VERSION_1_3,
            "Vulkan 1.3 is required; the loader exposes {}.{}",
            vk::api_version_major(api_version),
            vk::api_version_minor(api_version)
        );
        let layers = unsafe { entry.enumerate_instance_layer_properties() }?;
        let validation_available = layers.iter().any(|layer| unsafe { CStr::from_ptr(layer.layer_name.as_ptr()) } == c"VK_LAYER_KHRONOS_validation");
        let enable_validation = match mode {
            ValidationMode::Enabled => {
                ensure!(
                    validation_available,
                    "--validation requires VK_LAYER_KHRONOS_validation; install the Vulkan SDK/validation layers"
                );
                true
            }
            ValidationMode::Disabled => false,
            ValidationMode::Auto => cfg!(debug_assertions) && validation_available,
        };
        if cfg!(debug_assertions)
            && !validation_available
            && !matches!(mode, ValidationMode::Disabled)
        {
            log::warn!("Validation layer unavailable; use --validation to require it");
        }
        let extension_names: Vec<CString> = required_extensions
            .iter()
            .map(|name| CString::new(name.as_str()))
            .collect::<std::result::Result<_, _>>()?;
        let mut extensions: Vec<_> = extension_names.iter().map(|name| name.as_ptr()).collect();
        let available_extensions = unsafe { entry.enumerate_instance_extension_properties(None) }?;
        let has_debug = available_extensions.iter().any(|extension| unsafe { CStr::from_ptr(extension.extension_name.as_ptr()) } == ext::debug_utils::NAME);
        ensure!(
            !enable_validation || has_debug,
            "Validation requires VK_EXT_debug_utils"
        );
        if enable_validation {
            extensions.push(ext::debug_utils::NAME.as_ptr());
        }
        let enabled_layers = if enable_validation {
            vec![c"VK_LAYER_KHRONOS_validation".as_ptr()]
        } else {
            Vec::new()
        };
        let validation = Arc::new(ValidationState::default());
        let mut debug_info = vk::DebugUtilsMessengerCreateInfoEXT::default()
            .message_severity(
                vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                    | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
            )
            .message_type(
                vk::DebugUtilsMessageTypeFlagsEXT::GENERAL
                    | vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                    | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
            )
            .pfn_user_callback(Some(debug_callback))
            .user_data(Arc::as_ptr(&validation) as *mut c_void);
        let application = vk::ApplicationInfo::default()
            .application_name(c"VulkanUS")
            .application_version(vk::make_api_version(0, 0, 1, 0))
            .engine_name(c"VulkanUS")
            .api_version(vk::API_VERSION_1_3);
        let mut create_info = vk::InstanceCreateInfo::default()
            .application_info(&application)
            .enabled_extension_names(&extensions)
            .enabled_layer_names(&enabled_layers);
        if enable_validation {
            create_info = create_info.push_next(&mut debug_info);
        }
        let raw = unsafe { entry.create_instance(&create_info, None) }
            .context("Failed to create Vulkan instance")?;
        let mut instance = Self {
            raw,
            validation,
            debug: None,
            entry,
        };
        if enable_validation {
            let loader = ext::debug_utils::Instance::new(&instance.entry, &instance.raw);
            let messenger = unsafe { loader.create_debug_utils_messenger(&debug_info, None) }?;
            instance.debug = Some((loader, messenger));
        }
        log::info!("Vulkan 1.3 instance; validation: {enable_validation}");
        Ok(Rc::new(instance))
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: all child objects retain an Rc<Instance> and have already been destroyed.
        unsafe {
            if let Some((loader, messenger)) = &self.debug {
                loader.destroy_debug_utils_messenger(*messenger, None);
            }
            self.raw.destroy_instance(None);
        }
    }
}

unsafe extern "system" fn debug_callback(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _message_type: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    user_data: *mut c_void,
) -> vk::Bool32 {
    // SAFETY: Vulkan provides the callback data; the Arc pointee outlives the instance.
    let message = unsafe {
        if data.is_null() || (*data).p_message.is_null() {
            return vk::FALSE;
        }
        CStr::from_ptr((*data).p_message)
    };
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        if !user_data.is_null() {
            unsafe { &*(user_data as *const ValidationState) }
                .errors
                .fetch_add(1, Ordering::Relaxed);
        }
        log::error!("Vulkan: {}", message.to_string_lossy());
    } else {
        log::warn!("Vulkan: {}", message.to_string_lossy());
    }
    vk::FALSE
}
