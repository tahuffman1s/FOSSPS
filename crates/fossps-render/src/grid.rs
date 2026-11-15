//! The editor's ground grid: lines on the plane at height 0, a metre apart,
//! drawn over the level by `shaders/grid.wgsl`.
//!
//! It is one more pipeline in the mesh pass, drawn after the meshes with
//! the same view and the same pipeline layout.

use std::io::Cursor;

use ash::vk;

use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::scene::SceneTarget;

/// The grid shaders, compiled from `shaders/grid.wgsl` by the build script.
const SHADER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/grid.spv"));

/// The size in bytes of the shader's `Grid`: where the eye is.
pub(crate) const GRID_BYTES: usize = 16;

/// The shader's `Grid` for an eye at `eye`.
pub(crate) fn grid_bytes(eye: [f32; 3]) -> [u8; GRID_BYTES] {
    let mut bytes = [0_u8; GRID_BYTES];
    for (slot, number) in bytes.as_chunks_mut::<4>().0.iter_mut().zip(eye) {
        // A camera that is nowhere would otherwise draw the grid nowhere.
        *slot = if number.is_finite() { number } else { 0.0 }.to_ne_bytes();
    }
    bytes
}

/// Builds the pipeline that draws the grid into a [`SceneTarget`]. `layout`
/// is the mesh pipeline's, whose view the grid shares.
///
/// The caller destroys the pipeline.
pub(crate) fn pipeline(gpu: &Gpu, layout: vk::PipelineLayout) -> Result<vk::Pipeline, RenderError> {
    let doing = "preparing the shaders";
    let device = gpu.device();
    let code = ash::util::read_spv(&mut Cursor::new(SHADER)).map_err(|_| {
        // Only possible if the build script wrote a broken file.
        RenderError::Driver {
            doing,
            code: vk::Result::ERROR_INVALID_SHADER_NV,
        }
    })?;
    // SAFETY: `code` is SPIR-V produced and validated by naga at build time.
    let module = unsafe {
        device.create_shader_module(&vk::ShaderModuleCreateInfo::default().code(&code), None)
    }
    .map_err(RenderError::driver(doing))?;

    let stages = [
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::VERTEX)
            .module(module)
            .name(c"vertex"),
        vk::PipelineShaderStageCreateInfo::default()
            .stage(vk::ShaderStageFlags::FRAGMENT)
            .module(module)
            .name(c"fragment"),
    ];
    // The shader makes the square's corners itself.
    let vertex_input = vk::PipelineVertexInputStateCreateInfo::default();
    let assembly = vk::PipelineInputAssemblyStateCreateInfo::default()
        .topology(vk::PrimitiveTopology::TRIANGLE_LIST);
    let viewport = vk::PipelineViewportStateCreateInfo::default()
        .viewport_count(1)
        .scissor_count(1);
    // Seen from above and from below.
    let raster = vk::PipelineRasterizationStateCreateInfo::default()
        .polygon_mode(vk::PolygonMode::FILL)
        .cull_mode(vk::CullModeFlags::NONE)
        .line_width(1.0);
    let multisample = vk::PipelineMultisampleStateCreateInfo::default()
        .rasterization_samples(vk::SampleCountFlags::TYPE_1);
    // Hidden by what stands in front of it, and hides nothing itself: its
    // lines are see-through, so the floor's depth stays.
    let depth = vk::PipelineDepthStencilStateCreateInfo::default()
        .depth_test_enable(true)
        .depth_write_enable(false)
        .depth_compare_op(vk::CompareOp::GREATER_OR_EQUAL);
    // Laid over the picture by its opacity. Both images blend the same way,
    // which every graphics card can do; the shader gives how far the pixel
    // moved an opacity of 1, so that it is written as it is.
    let attachments = [vk::PipelineColorBlendAttachmentState::default()
        .blend_enable(true)
        .src_color_blend_factor(vk::BlendFactor::SRC_ALPHA)
        .dst_color_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .color_blend_op(vk::BlendOp::ADD)
        .src_alpha_blend_factor(vk::BlendFactor::ONE)
        .dst_alpha_blend_factor(vk::BlendFactor::ONE_MINUS_SRC_ALPHA)
        .alpha_blend_op(vk::BlendOp::ADD)
        .color_write_mask(vk::ColorComponentFlags::RGBA); 2];
    let blend = vk::PipelineColorBlendStateCreateInfo::default().attachments(&attachments);
    let dynamic_states = [vk::DynamicState::VIEWPORT, vk::DynamicState::SCISSOR];
    let dynamic = vk::PipelineDynamicStateCreateInfo::default().dynamic_states(&dynamic_states);
    let formats = [SceneTarget::FORMAT, SceneTarget::MOTION_FORMAT];
    let mut rendering = vk::PipelineRenderingCreateInfo::default()
        .color_attachment_formats(&formats)
        .depth_attachment_format(SceneTarget::DEPTH_FORMAT);
    let info = vk::GraphicsPipelineCreateInfo::default()
        .stages(&stages)
        .vertex_input_state(&vertex_input)
        .input_assembly_state(&assembly)
        .viewport_state(&viewport)
        .rasterization_state(&raster)
        .multisample_state(&multisample)
        .depth_stencil_state(&depth)
        .color_blend_state(&blend)
        .dynamic_state(&dynamic)
        .layout(layout)
        .push_next(&mut rendering);

    // SAFETY: `info` and everything it points to are alive for the call;
    // the shader module holds both entry points named in `stages`, and the
    // layout is the caller's live one. The module is only needed while the
    // pipeline is being created.
    let created = unsafe {
        let created = device.create_graphics_pipelines(
            vk::PipelineCache::null(),
            std::slice::from_ref(&info),
            None,
        );
        device.destroy_shader_module(module, None);
        created
    };
    match created {
        Ok(pipelines) if !pipelines.is_empty() => Ok(pipelines[0]),
        failed => {
            let code = failed.map_or_else(|(_, code)| code, |_| vk::Result::ERROR_UNKNOWN);
            Err(RenderError::driver(doing)(code))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_shader_is_told_where_the_eye_is() {
        let bytes = grid_bytes([1.0, f32::NAN, -3.0]);
        let number = |at: usize| f32::from_ne_bytes([0, 1, 2, 3].map(|byte| bytes[at * 4 + byte]));
        assert_eq!(
            [number(0), number(1), number(2), number(3)],
            [1.0, 0.0, -3.0, 0.0]
        );
    }
}
