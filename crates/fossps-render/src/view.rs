//! One view of the scene: drawn, upscaled if asked, and ready to tone map.

use std::sync::Arc;

use ash::vk;

use crate::camera::{Camera, Mat4, shifted};
use crate::error::RenderError;
use crate::gpu::Gpu;
use crate::mesh::{Contents, MeshPipeline, View};
use crate::scene::SceneTarget;
use crate::upscale::{Drawn, Upscale, Upscaling};

/// Everything between the camera and the picture that is tone mapped: the
/// images the scene is drawn into, the upscaler, and what has to be
/// remembered from one frame to the next.
pub(crate) struct SceneView {
    // The upscaler goes before the images it reads.
    upscale: Option<Upscale>,
    target: SceneTarget,
    /// Where each point in the world was drawn in the last frame; `None`
    /// before the first frame and after a cut.
    before: Option<Mat4>,
    /// How many frames have been drawn, for the camera shift.
    frame: u32,
    /// How many sizes larger than usual textures are read; see [`sharper`].
    sharper: f32,
}

/// How many sizes larger than it otherwise would the graphics card should
/// read a texture when a scene `render` pixels wide is upscaled to `output`.
///
/// A scene drawn small picks small, blurry copies of its textures, which
/// upscaling cannot make sharp again. FSR's manual gives the rule: the
/// base-2 logarithm of how much smaller the scene is drawn, and one more.
fn sharper(render: u32, output: u32) -> f32 {
    // Sizes are far below the point where f32 stops being exact.
    #[allow(clippy::cast_precision_loss)]
    let ratio = output.max(1) as f32 / render.max(1) as f32;
    ratio.log2().max(0.0) + 1.0
}

impl SceneView {
    /// Makes a view whose finished picture is `output` pixels.
    ///
    /// If `upscaling` cannot be had on this graphics card, the reason is
    /// logged and the scene is drawn at full size.
    pub(crate) fn new(
        gpu: &Arc<Gpu>,
        output: vk::Extent2D,
        upscaling: Upscaling,
    ) -> Result<Self, RenderError> {
        let upscale = Upscale::new(gpu, output, upscaling)?;
        let render = upscale.as_ref().map_or(output, |upscale| upscale.render);
        Ok(Self {
            sharper: if upscale.is_some() {
                sharper(render.width, output.width)
            } else {
                0.0
            },
            target: SceneTarget::new(gpu, render)?,
            upscale,
            before: None,
            frame: 0,
        })
    }

    /// The finished picture, in high dynamic range, for the tone mapping to read.
    pub(crate) fn picture(&self) -> vk::ImageView {
        self.upscale
            .as_ref()
            .map_or(self.target.colour.view, |upscale| upscale.picture.view)
    }

    /// The size the scene is drawn at, which is smaller than the finished
    /// picture when upscaling.
    pub(crate) fn render_size(&self) -> vk::Extent2D {
        self.target.extent
    }

    /// Whether the scene is being upscaled.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn upscales(&self) -> bool {
        self.upscale.is_some()
    }

    /// The images the scene was drawn into.
    #[cfg(test)]
    pub(crate) fn target(&self) -> &SceneTarget {
        &self.target
    }

    /// Forgets the frames before: the next one is not blended with them.
    pub(crate) fn cut(&mut self) {
        self.before = None;
    }

    /// Records drawing `contents` as `camera` sees them, and upscaling the result.
    ///
    /// `seconds` is how long the last frame took.
    ///
    /// # Errors
    ///
    /// Fails if the graphics driver or the upscaler reports a failure.
    ///
    /// # Safety
    ///
    /// `commands` must be a command buffer of this view's device that is
    /// being recorded, outside a render pass. `mesh` and the meshes in `contents` must
    /// belong to the same device, and they and this view must stay alive
    /// until the commands have run. The view must only be in use by earlier
    /// commands on the same queue. `frame` is as for [`MeshPipeline::record`].
    pub(crate) unsafe fn record(
        &mut self,
        commands: vk::CommandBuffer,
        frame: usize,
        mesh: &MeshPipeline,
        contents: Contents<'_>,
        camera: &Camera,
        seconds: f32,
    ) -> Result<(), RenderError> {
        let size = self.target.extent;
        let steady = camera.clip_from_world(size.width, size.height);
        let jitter = self
            .upscale
            .as_ref()
            .map_or((0.0, 0.0), |upscale| upscale.jitter(self.frame));
        // Sizes are far below the point where f32 stops being exact.
        #[allow(clippy::cast_precision_loss)]
        let (width, height) = (size.width as f32, size.height as f32);
        let view = View {
            // The picture is 2 wide and 2 high with y running up, and the
            // shift is in pixels with y running down. The direction is the
            // FSR manual's; the offscreen test fails with either sign turned.
            clip: shifted(&steady, 2.0 * jitter.0 / width, -2.0 * jitter.1 / height),
            steady,
            before: self.before.unwrap_or(steady),
            eye: camera.position,
            sharper: self.sharper,
        };
        let cut = self.before.is_none();
        self.before = Some(steady);
        self.frame = self.frame.wrapping_add(1);

        // SAFETY: the caller's guarantees are the ones `MeshPipeline::record`
        // asks for, and the target is this view's own.
        unsafe {
            mesh.record(
                commands,
                frame,
                &self.target,
                &view,
                contents.meshes,
                contents.grid.then_some(camera.position),
            )
        }?;
        if let Some(upscale) = &mut self.upscale {
            let drawn = Drawn {
                jitter,
                seconds,
                near: camera.near_metres(),
                fov: camera.fov_radians(),
                cut,
            };
            // SAFETY: the mesh pass above drew the target, which was made
            // the size the upscaler draws at, and left it ready to be read.
            unsafe { upscale.record(commands, &self.target, &drawn) }
                .map_err(RenderError::Upscaler)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::sharper;

    #[test]
    fn a_scene_drawn_smaller_reads_larger_textures() {
        // Drawn full size and only smoothed: one size larger, as FSR asks.
        assert_eq!(sharper(1920, 1920), 1.0);
        // Half as wide: one more.
        assert_eq!(sharper(960, 1920), 2.0);
        assert!((sharper(1280, 1920) - 1.585).abs() < 1e-3);
        // Nothing to divide by is not a reason to fail.
        assert!(sharper(0, 0).is_finite());
    }
}
