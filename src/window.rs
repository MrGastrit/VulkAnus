use anyhow::{Context, Result, ensure};
use ash::vk;
use glfw::{Action, ClientApiHint, Glfw, GlfwReceiver, Key, PWindow, WindowEvent, WindowHint};

pub struct Window {
    // GLFW's window must be destroyed before its final Glfw handle.
    pub raw: PWindow,
    events: GlfwReceiver<(f64, WindowEvent)>,
    pub glfw: Glfw,
}

impl Window {
    pub fn new() -> Result<Self> {
        let mut glfw =
            glfw::init(|error, description| log::error!("GLFW {error:?}: {description}"))
                .context("Failed to initialize GLFW (a desktop display is required)")?;
        ensure!(
            glfw.vulkan_supported(),
            "GLFW could not find a Vulkan loader/driver"
        );
        glfw.window_hint(WindowHint::ClientApi(ClientApiHint::NoApi));
        glfw.window_hint(WindowHint::Resizable(true));
        let (mut raw, events) = glfw
            .create_window(
                1280,
                720,
                "VulkanUS — Vulkan / GLFW / VMA",
                glfw::WindowMode::Windowed,
            )
            .context("Failed to create the GLFW window")?;
        raw.set_key_polling(true);
        raw.set_framebuffer_size_polling(true);
        Ok(Self { raw, events, glfw })
    }

    pub fn required_extensions(&self) -> Result<Vec<String>> {
        self.glfw
            .get_required_instance_extensions()
            .context("GLFW returned no Vulkan surface extensions")
    }

    pub fn extent(&self) -> vk::Extent2D {
        let (width, height) = self.raw.get_framebuffer_size();
        vk::Extent2D {
            width: width.max(0) as u32,
            height: height.max(0) as u32,
        }
    }

    pub fn poll_events(&mut self) -> bool {
        self.glfw.poll_events();
        let mut resized = false;
        for (_, event) in glfw::flush_messages(&self.events) {
            match event {
                WindowEvent::FramebufferSize(_, _) => resized = true,
                WindowEvent::Key(Key::Escape, _, Action::Press, _) => {
                    self.raw.set_should_close(true)
                }
                _ => {}
            }
        }
        resized
    }
}
