//! What can go wrong in the renderer, worded for the person who has to fix it.

use std::fmt;

use ash::vk;

/// Why the renderer could not start or keep drawing.
#[derive(Debug)]
pub enum RenderError {
    /// The Vulkan library is not installed.
    NoVulkan(ash::LoadingError),
    /// Vulkan is installed but is older than 1.3.
    OldVulkan {
        /// The newest version the installed Vulkan library offers.
        found: Version,
    },
    /// No graphics card in this computer can run the renderer.
    NoSuitableGpu {
        /// Each graphics card that was found and why it cannot be used.
        rejected: Vec<String>,
    },
    /// The window could not be used for drawing.
    Window(raw_window_handle::HandleError),
    /// The graphics driver stopped responding.
    DeviceLost,
    /// The graphics card ran out of memory.
    OutOfMemory {
        /// What the renderer was doing.
        doing: &'static str,
    },
    /// The graphics driver reported some other failure.
    Driver {
        /// What the renderer was doing.
        doing: &'static str,
        /// The code the driver returned.
        code: vk::Result,
    },
}

/// A Vulkan version, such as 1.3.280.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Version {
    /// The first number: 1 in 1.3.280.
    pub major: u32,
    /// The second number: 3 in 1.3.280.
    pub minor: u32,
    /// The third number: 280 in 1.3.280.
    pub patch: u32,
}

impl Version {
    /// Unpacks the single number Vulkan stores a version in.
    pub(crate) fn from_vk(packed: u32) -> Self {
        Self {
            major: vk::api_version_major(packed),
            minor: vk::api_version_minor(packed),
            patch: vk::api_version_patch(packed),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

impl RenderError {
    /// Wraps a driver failure, saying what the renderer was doing at the time.
    pub(crate) fn driver(doing: &'static str) -> impl FnOnce(vk::Result) -> Self {
        move |code| match code {
            vk::Result::ERROR_DEVICE_LOST => Self::DeviceLost,
            vk::Result::ERROR_OUT_OF_DEVICE_MEMORY | vk::Result::ERROR_OUT_OF_HOST_MEMORY => {
                Self::OutOfMemory { doing }
            }
            code => Self::Driver { doing, code },
        }
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoVulkan(error) => write!(
                f,
                "Could not find Vulkan on this computer. \
                 FOSSPS draws with Vulkan 1.3: install or update your graphics driver, \
                 then start the game again. On Linux, also install the Vulkan loader package \
                 (`vulkan-loader` on Fedora, `libvulkan1` on Debian and Ubuntu, \
                 `vulkan-icd-loader` on Arch). Details: {error}"
            ),
            Self::OldVulkan { found } => write!(
                f,
                "This computer has Vulkan {found}, but FOSSPS needs Vulkan 1.3 or newer. \
                 Update your graphics driver, then start the game again."
            ),
            Self::NoSuitableGpu { rejected } if rejected.is_empty() => write!(
                f,
                "Vulkan is installed but reports no graphics card. \
                 Install or update your graphics driver, then start the game again."
            ),
            Self::NoSuitableGpu { rejected } => {
                write!(f, "No graphics card in this computer can run FOSSPS:")?;
                for reason in rejected {
                    write!(f, "\n  - {reason}")?;
                }
                write!(
                    f,
                    "\nFOSSPS needs Vulkan 1.3. Updating your graphics driver usually fixes this. \
                     On Linux you can also run `vulkaninfo --summary` (from the `vulkan-tools` \
                     package) to see which Vulkan version each card supports."
                )
            }
            Self::Window(error) => write!(
                f,
                "Could not draw into the game window ({error}). \
                 Start the game again; if it keeps happening, update your graphics driver."
            ),
            Self::DeviceLost => write!(
                f,
                "The graphics driver stopped responding. Start the game again; \
                 if it keeps happening, update your graphics driver."
            ),
            Self::OutOfMemory { doing } => write!(
                f,
                "Ran out of graphics memory while {doing}. \
                 Close other programs that use the graphics card, or make the game window smaller."
            ),
            Self::Driver { doing, code } => write!(
                f,
                "The graphics driver failed while {doing} ({code:?}). \
                 Update your graphics driver; if that does not help, report this message at \
                 https://github.com/tahuffman1s/FOSSPS/issues."
            ),
        }
    }
}

impl std::error::Error for RenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::NoVulkan(error) => Some(error),
            Self::Window(error) => Some(error),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn driver_failures_are_sorted_by_what_the_player_can_do() {
        assert!(matches!(
            RenderError::driver("drawing")(vk::Result::ERROR_DEVICE_LOST),
            RenderError::DeviceLost
        ));
        assert!(matches!(
            RenderError::driver("drawing")(vk::Result::ERROR_OUT_OF_DEVICE_MEMORY),
            RenderError::OutOfMemory { doing: "drawing" }
        ));
        assert!(matches!(
            RenderError::driver("drawing")(vk::Result::ERROR_UNKNOWN),
            RenderError::Driver { .. }
        ));
    }

    #[test]
    fn every_rejected_gpu_is_listed() {
        let error = RenderError::NoSuitableGpu {
            rejected: vec!["Old Card: supports Vulkan 1.1.0".to_owned()],
        };
        let text = error.to_string();
        assert!(text.contains("Old Card: supports Vulkan 1.1.0"));
        assert!(text.contains("vulkaninfo"));
    }

    #[test]
    fn versions_read_like_versions() {
        let version = Version::from_vk(vk::make_api_version(0, 1, 3, 280));
        assert_eq!(version.to_string(), "1.3.280");
        assert!(
            version
                > Version {
                    major: 1,
                    minor: 2,
                    patch: 999
                }
        );
    }
}
