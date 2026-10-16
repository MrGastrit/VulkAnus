use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Result, bail};

use crate::{
    config::Config,
    graphics::{Renderer, ValidationState},
    window::Window,
};

pub struct Application {
    // Rust drops fields in declaration order: Vulkan surface before native window.
    renderer: Renderer,
    window: Window,
    config: Config,
}

impl Application {
    pub fn new(config: Config) -> Result<Self> {
        let window = Window::new()?;
        let renderer = Renderer::new(&window, config.validation)?;
        Ok(Self {
            renderer,
            window,
            config,
        })
    }

    pub fn validation_state(&self) -> Arc<ValidationState> {
        self.renderer.validation_state()
    }

    pub fn run(&mut self) -> Result<()> {
        let mut presented = 0;
        let mut smoke_stage = 0;
        let mut last_progress = Instant::now();
        while !self.window.raw.should_close() {
            if self.window.poll_events() {
                self.renderer.request_resize();
            }
            if self.window.raw.should_close() {
                break;
            }
            if self.config.smoke_test {
                self.smoke_step(presented, &mut smoke_stage);
                if last_progress.elapsed() > Duration::from_secs(30) {
                    bail!("Smoke test made no presentation progress for 30 seconds");
                }
            }
            let extent = self.window.extent();
            if extent.width == 0 || extent.height == 0 || self.window.raw.is_iconified() {
                // Keep processing close/restore events without spinning or acquiring images.
                self.window.glfw.wait_events_timeout(0.05);
                continue;
            }
            if self.renderer.draw(extent)? {
                presented += 1;
                last_progress = Instant::now();
                if self
                    .config
                    .frame_limit
                    .is_some_and(|limit| presented >= limit)
                {
                    break;
                }
            } else {
                self.window.glfw.wait_events_timeout(0.01);
            }
        }
        self.renderer.wait_idle()?;
        log::info!("Presented {presented} frames");
        Ok(())
    }

    fn smoke_step(&mut self, presented: u64, stage: &mut u32) {
        match (*stage, presented) {
            (0, 20..) => {
                self.window.raw.set_size(800, 600);
                self.renderer.request_resize();
                *stage = 1;
            }
            (1, 40..) => {
                self.window.raw.set_size(1400, 500);
                self.renderer.request_resize();
                *stage = 2;
            }
            (2, 60..) => {
                self.window.raw.iconify();
                // A finite pause also works under Xvfb without a window manager.
                self.window.glfw.wait_events_timeout(0.1);
                self.window.raw.restore();
                self.renderer.request_resize();
                *stage = 3;
            }
            (3, 80..) => {
                self.window.raw.set_size(960, 720);
                self.renderer.request_resize();
                *stage = 4;
            }
            _ => {}
        }
    }
}
