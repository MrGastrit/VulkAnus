use std::{
    io::Cursor,
    mem::{offset_of, size_of},
    rc::Rc,
};

use anyhow::{Context, Result};
use ash::vk;

use super::{buffer::Vertex, device::Device};

pub struct Pipeline {
    pub handle: vk::Pipeline,
    layout: vk::PipelineLayout,
    device: Rc<Device>,
}

struct Shader {
    handle: vk::ShaderModule,
    device: Rc<Device>,
}

impl Shader {
    fn new(device: Rc<Device>, bytes: &[u8]) -> Result<Self> {
        let words =
            ash::util::read_spv(&mut Cursor::new(bytes)).context("Invalid embedded SPIR-V")?;
        let handle = unsafe {
            device
                .raw
                .create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&words), None)
        }?;
        Ok(Self { handle, device })
    }
}

impl Drop for Shader {
    fn drop(&mut self) {
        unsafe { self.device.raw.destroy_shader_module(self.handle, None) };
    }
}

impl Pipeline {
    pub fn new(device: Rc<Device>, format: vk::Format) -> Result<Self> {
        let vertex = Shader::new(
            device.clone(),
            include_bytes!(concat!(env!("OUT_DIR"), "/triangle.vert.spv")),
        )?;
        let fragment = Shader::new(
            device.clone(),
            include_bytes!(concat!(env!("OUT_DIR"), "/triangle.frag.spv")),
        )?;
        let mut pipeline = Self {
            handle: vk::Pipeline::null(),
            layout: vk::PipelineLayout::null(),
            device,
        };
        unsafe {
            pipeline.layout = pipeline
                .device
                .raw
                .create_pipeline_layout(&vk::PipelineLayoutCreateInfo::default(), None)?;
        }
        let stages = [
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::VERTEX)
                .module(vertex.handle)
                .name(c"main"),
            vk::PipelineShaderStageCreateInfo::default()
                .stage(vk::ShaderStageFlags::FRAGMENT)
                .module(fragment.handle)
                .name(c"main"),
        ];
        let bindings = [vk::VertexInputBindingDescription {
            binding: 0,
            stride: size_of::<Vertex>() as u32,
            input_rate: vk::VertexInputRate::VERTEX,
        }];
        let attributes = [
            vk::VertexInputAttributeDescription {
                location: 0,
                binding: 0,
                format: vk::Format::R32G32_SFLOAT,
                offset: offset_of!(Vertex, position) as u32,
            },
            vk::VertexInputAttributeDescription {
                location: 1,
                binding: 0,
                format: vk::Format::R32G32B32_SFLOAT,
                offset: offset_of!(Vertex, color) as u32,
            },
        ];
        let vertex_input = vk::PipelineVertexInputStateCreateInfo::default()
            .vertex_binding_descriptions(&bindings)
            .vertex_attribute_descriptions(&attributes);
        let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
            .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
        let viewport = vk::PipelineViewportStateCreateInfo::default()
            .viewport_count(1)
            .scissor_count(1);
        let rasterization = vk::PipelineRasterizationStateCreateInfo::default()
            .polygon_mode(vk::PolygonMode::FILL)
            .cull_mode(vk::CullModeFlags::NONE)
            .front_face(vk::FrontFace::COUNTER_CLOCKWISE)
            .line_width(1.0);
        let multisampling = vk::PipelineMultisampleStateCreateInfo::default()
            .rasterization_samples(vk::SampleCountFlags::TYPE_1);
        let attachments = [vk::PipelineColorBlendAttachmentState::default()
            .blend_enable(false)
            .color_write_mask(vk::ColorComponentFlags::RGBA)];
        let blending = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
        let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
        let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
        let formats = [format];
        let mut rendering =
            vk::PipelineRenderingCreateInfo::default().color_attachment_formats(&formats);
        let info = [vk::GraphicsPipelineCreateInfo::default()
            .stages(&stages)
            .vertex_input_state(&vertex_input)
            .input_assembly_state(&assembly)
            .viewport_state(&viewport)
            .rasterization_state(&rasterization)
            .multisample_state(&multisampling)
            .color_blend_state(&blending)
            .dynamic_state(&dynamic)
            .layout(pipeline.layout)
            .push_next(&mut rendering)];
        match unsafe {
            pipeline
                .device
                .raw
                .create_graphics_pipelines(vk::PipelineCache::null(), &info, None)
        } {
            Ok(handles) => pipeline.handle = handles[0],
            Err((handles, error)) => {
                for handle in handles {
                    unsafe { pipeline.device.raw.destroy_pipeline(handle, None) };
                }
                return Err(error).context("Failed to create triangle pipeline");
            }
        }
        Ok(pipeline)
    }
}

impl Drop for Pipeline {
    fn drop(&mut self) {
        unsafe {
            self.device.raw.destroy_pipeline(self.handle, None);
            self.device.raw.destroy_pipeline_layout(self.layout, None);
        }
    }
}
