//! What can go wrong when upscaling.

use std::fmt;

use ash::vk;

use crate::ffi;

/// Why the upscaler could not be made or could not upscale a frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsrError {
    /// A size of zero pixels was asked for.
    EmptySize,
    /// The drawn picture would be larger than the picture it is upscaled to.
    RenderLargerThanOutput {
        /// The largest picture that would be drawn.
        max_render: vk::Extent2D,
        /// The picture it would be upscaled to.
        output: vk::Extent2D,
    },
    /// An image is not the size the upscaler was made for.
    WrongSize {
        /// Which image.
        what: &'static str,
        /// The size it is.
        size: vk::Extent2D,
        /// The size it has to be, or for the drawn picture, the largest it may be.
        expected: vk::Extent2D,
    },
    /// An image the upscaler needs was not given.
    MissingImage {
        /// Which image.
        what: &'static str,
    },
    /// An image was created without a usage flag the upscaler needs.
    WrongUsage {
        /// Which image.
        what: &'static str,
        /// The `VK_IMAGE_USAGE_` flag it has to be created with.
        needs: &'static str,
    },
    /// Another Vulkan device already has an upscaler.
    SecondDevice,
    /// The graphics card ran out of memory.
    OutOfMemory {
        /// What the upscaler was doing.
        doing: &'static str,
    },
    /// The graphics driver refused something the upscaler asked for.
    Driver {
        /// What the upscaler was doing.
        doing: &'static str,
    },
    /// The FidelityFX SDK reported an error that should not happen.
    Sdk {
        /// What the upscaler was doing.
        doing: &'static str,
        /// The SDK's error code, an `FfxErrorCodes` value.
        code: u32,
    },
}

impl FsrError {
    /// Turns an error code from the C side into an error.
    pub(crate) fn from_code(doing: &'static str, code: i32) -> Self {
        // `FFX_ERROR_OUT_OF_MEMORY`, `FFX_ERROR_INSUFFICIENT_MEMORY` and
        // `FFX_ERROR_BACKEND_API_ERROR` in the SDK's `ffx_error.h`.
        match code.cast_unsigned() {
            _ if code == ffi::ERROR_SECOND_DEVICE => Self::SecondDevice,
            0x8000_0007 | 0x8000_000e => Self::OutOfMemory { doing },
            0x8000_000d => Self::Driver { doing },
            code => Self::Sdk { doing, code },
        }
    }
}

impl fmt::Display for FsrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let size = |extent: &vk::Extent2D| format!("{}x{}", extent.width, extent.height);
        match self {
            Self::EmptySize => write!(
                f,
                "The upscaler was asked for a picture with no pixels. Make it once the window has \
                 a size."
            ),
            Self::RenderLargerThanOutput { max_render, output } => write!(
                f,
                "The upscaler was asked to turn a {} picture into a smaller {} one. It only makes \
                 pictures bigger; draw at the window's size or smaller.",
                size(max_render),
                size(output)
            ),
            Self::WrongSize {
                what,
                size: actual,
                expected,
            } => write!(
                f,
                "The {what} is {} but the upscaler was made for {}. Make a new upscaler when the \
                 window or the quality setting changes.",
                size(actual),
                size(expected)
            ),
            Self::MissingImage { what } => write!(
                f,
                "The upscaler was given no {what}. It needs the drawn picture, its depth buffer \
                 and its motion vectors, and a picture to write to."
            ),
            Self::WrongUsage { what, needs } => write!(
                f,
                "The {what} was created without VK_IMAGE_USAGE_{needs}_BIT, so the upscaler \
                 cannot use it. Add that flag where the image is created."
            ),
            Self::SecondDevice => write!(
                f,
                "Another graphics device is already upscaling. Drop its upscaler first; only one \
                 device can upscale at a time."
            ),
            Self::OutOfMemory { doing } => write!(
                f,
                "The graphics card ran out of memory while {doing}. Lower the resolution or the \
                 texture quality, or close other programs that use the graphics card."
            ),
            Self::Driver { doing } => write!(
                f,
                "The graphics driver reported a failure while {doing}. Update the graphics \
                 driver; if that does not help, turn upscaling off in the settings."
            ),
            Self::Sdk { doing, code } => write!(
                f,
                "FSR reported error {code:#010x} while {doing}. This is a bug in FOSSPS; please \
                 report it at https://github.com/tahuffman1s/FOSSPS/issues with this message. \
                 Turning upscaling off in the settings works around it."
            ),
        }
    }
}

impl std::error::Error for FsrError {}
