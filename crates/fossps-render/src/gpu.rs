//! The connection to the graphics card: Vulkan instance, chosen device and queue.

use std::ffi::{CStr, c_char, c_void};
use std::sync::atomic::{AtomicU32, Ordering};

use ash::ext::debug_utils;
use ash::khr::{portability_enumeration, portability_subset, surface, swapchain};
use ash::vk;
use raw_window_handle::{RawDisplayHandle, RawWindowHandle};

use crate::error::{RenderError, Version};

/// The oldest Vulkan the renderer runs on.
const NEEDED: Version = Version {
    major: 1,
    minor: 3,
    patch: 0,
};

/// The layer that checks every Vulkan call for mistakes.
const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// Set this environment variable to 1 to check every Vulkan call in a release build.
const VALIDATION_VARIABLE: &str = "FOSSPS_VALIDATION";

/// How many mistakes the validation layer has reported since the program started.
static VALIDATION_ERRORS: AtomicU32 = AtomicU32::new(0);

/// How many mistakes the validation layer has reported since the program started.
///
/// Always 0 when the validation layer is not installed or not switched on.
#[must_use]
pub fn validation_errors() -> u32 {
    VALIDATION_ERRORS.load(Ordering::Relaxed)
}

/// The Vulkan instance, and the window surface when there is a window.
struct Instance {
    /// Keeps the Vulkan library loaded.
    _entry: ash::Entry,
    raw: ash::Instance,
    debug: Option<(debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    surfaces: surface::Instance,
    /// Null when drawing without a window.
    surface: vk::SurfaceKHR,
}

impl Instance {
    /// Loads Vulkan and, given a window, prepares to draw into it.
    ///
    /// # Safety
    ///
    /// The window behind the handles must stay open until this is dropped.
    unsafe fn new(
        window: Option<(RawDisplayHandle, RawWindowHandle)>,
    ) -> Result<Self, RenderError> {
        // SAFETY: loading the system's Vulkan library runs its initialisers,
        // which is the documented way to start using Vulkan.
        let entry = unsafe { ash::Entry::load() }.map_err(RenderError::NoVulkan)?;

        // SAFETY: `entry` holds a loaded Vulkan library.
        let found = unsafe { entry.try_enumerate_instance_version() }
            .map_err(RenderError::driver("checking the Vulkan version"))?
            .map_or(
                Version {
                    major: 1,
                    minor: 0,
                    patch: 0,
                },
                Version::from_vk,
            );
        if found < NEEDED {
            return Err(RenderError::OldVulkan { found });
        }

        // SAFETY: `entry` holds a loaded Vulkan library.
        let offered = unsafe { entry.enumerate_instance_extension_properties(None) }
            .map_err(RenderError::driver("listing Vulkan extensions"))?;
        let offers = |name: &CStr| {
            offered
                .iter()
                .any(|extension| extension.extension_name_as_c_str() == Ok(name))
        };

        let mut extensions: Vec<*const c_char> = Vec::new();
        if let Some((display, _)) = window {
            extensions.extend_from_slice(
                ash_window::enumerate_required_extensions(display)
                    .map_err(RenderError::driver("preparing to draw into the window"))?,
            );
        }
        // MoltenVK on macOS only shows up when this is asked for.
        let mut flags = vk::InstanceCreateFlags::empty();
        if offers(portability_enumeration::NAME) {
            extensions.push(portability_enumeration::NAME.as_ptr());
            flags |= vk::InstanceCreateFlags::ENUMERATE_PORTABILITY_KHR;
        }

        let validate = wants_validation(&entry) && offers(debug_utils::NAME);
        let mut layers: Vec<*const c_char> = Vec::new();
        if validate {
            extensions.push(debug_utils::NAME.as_ptr());
            layers.push(VALIDATION_LAYER.as_ptr());
        }

        let application = vk::ApplicationInfo::default()
            .engine_name(c"FOSSPS")
            .api_version(vk::API_VERSION_1_3);
        let info = vk::InstanceCreateInfo::default()
            .application_info(&application)
            .enabled_extension_names(&extensions)
            .enabled_layer_names(&layers)
            .flags(flags);
        // SAFETY: `info` and everything it points to are alive for the call,
        // and every extension and layer named was reported as available.
        let raw = unsafe { entry.create_instance(&info, None) }.map_err(|code| match code {
            // The Vulkan library is installed, but no graphics driver is behind it.
            vk::Result::ERROR_INCOMPATIBLE_DRIVER => RenderError::NoSuitableGpu {
                rejected: Vec::new(),
            },
            code => RenderError::driver("starting Vulkan")(code),
        })?;

        let mut instance = Self {
            surfaces: surface::Instance::new(&entry, &raw),
            _entry: entry,
            raw,
            debug: None,
            surface: vk::SurfaceKHR::null(),
        };
        // From here on, dropping `instance` cleans up whatever was created.

        if validate {
            let loader = debug_utils::Instance::new(&instance._entry, &instance.raw);
            let info = vk::DebugUtilsMessengerCreateInfoEXT::default()
                .message_severity(
                    vk::DebugUtilsMessageSeverityFlagsEXT::WARNING
                        | vk::DebugUtilsMessageSeverityFlagsEXT::ERROR,
                )
                .message_type(
                    vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION
                        | vk::DebugUtilsMessageTypeFlagsEXT::PERFORMANCE,
                )
                .pfn_user_callback(Some(report_validation));
            // SAFETY: the debug extension was enabled on this instance and
            // `report_validation` matches the callback signature.
            match unsafe { loader.create_debug_utils_messenger(&info, None) } {
                Ok(messenger) => instance.debug = Some((loader, messenger)),
                Err(code) => log::warn!(
                    "Could not switch on Vulkan validation ({code:?}). Drawing continues without it."
                ),
            }
        }

        if let Some((display, handle)) = window {
            // SAFETY: the caller keeps the window open until `instance` is
            // dropped, and the extensions this needs were enabled above.
            instance.surface = unsafe {
                ash_window::create_surface(&instance._entry, &instance.raw, display, handle, None)
            }
            .map_err(RenderError::driver("preparing to draw into the window"))?;
        }
        Ok(instance)
    }
}

impl Drop for Instance {
    fn drop(&mut self) {
        // SAFETY: the device and everything made from it are destroyed before
        // the instance (see `Gpu`), so nothing still uses these objects.
        unsafe {
            if self.surface != vk::SurfaceKHR::null() {
                self.surfaces.destroy_surface(self.surface, None);
            }
            if let Some((loader, messenger)) = self.debug.take() {
                loader.destroy_debug_utils_messenger(messenger, None);
            }
            self.raw.destroy_instance(None);
        }
    }
}

/// Whether to check every Vulkan call: in debug builds, or when asked through
/// the environment, and only if the validation layer is installed.
fn wants_validation(entry: &ash::Entry) -> bool {
    let asked = std::env::var(VALIDATION_VARIABLE).is_ok_and(|value| value != "0");
    if !asked && !cfg!(debug_assertions) {
        return false;
    }
    // SAFETY: `entry` holds a loaded Vulkan library.
    let installed = unsafe { entry.enumerate_instance_layer_properties() }.is_ok_and(|layers| {
        layers
            .iter()
            .any(|layer| layer.layer_name_as_c_str() == Ok(VALIDATION_LAYER))
    });
    if asked && !installed {
        log::warn!(
            "{VALIDATION_VARIABLE} is set, but the Vulkan validation layer is not installed, \
             so Vulkan calls are not being checked. Install the Vulkan SDK (Windows), or the \
             `vulkan-validation-layers` package (Fedora, Arch) or `vulkan-validationlayers` \
             (Debian, Ubuntu). To stop this message, unset {VALIDATION_VARIABLE}."
        );
    }
    installed
}

/// Passes what the validation layer says on to the log.
unsafe extern "system" fn report_validation(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _: *mut c_void,
) -> vk::Bool32 {
    // SAFETY: Vulkan passes either null or a pointer valid for this call.
    let message = unsafe { data.as_ref() }
        // SAFETY: the message, when present, is a NUL-terminated string valid for this call.
        .and_then(|data| unsafe { data.message_as_c_str() })
        .map(CStr::to_string_lossy)
        .unwrap_or_default();
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
        log::error!("Vulkan validation: {message}");
    } else {
        log::warn!("Vulkan validation: {message}");
    }
    vk::FALSE
}

/// What the renderer needs to know about one graphics card to decide whether to use it.
#[derive(Debug, Clone)]
struct Candidate {
    name: String,
    kind: vk::PhysicalDeviceType,
    version: Version,
    /// Lets a draw target be named when drawing starts, without render pass objects.
    dynamic_rendering: bool,
    /// The simpler way of ordering work on the graphics card.
    synchronization2: bool,
    /// Whether it can show pictures in a window. Not needed without one.
    presents: bool,
    /// The queue family that can draw (and show the result, given a window).
    queue_family: Option<u32>,
}

impl Candidate {
    /// Why this card cannot be used, or `None` if it can.
    fn rejection(&self, windowed: bool) -> Option<String> {
        let name = &self.name;
        if self.version < NEEDED {
            return Some(format!(
                "{name}: its driver supports Vulkan {}, older than 1.3",
                self.version
            ));
        }
        if !self.dynamic_rendering || !self.synchronization2 {
            return Some(format!(
                "{name}: its driver does not support everything in Vulkan 1.3 \
                 (usually fixed by updating the driver)"
            ));
        }
        if windowed && !self.presents {
            return Some(format!(
                "{name}: its driver cannot draw into a window on this system"
            ));
        }
        if self.queue_family.is_none() {
            return Some(if windowed {
                format!(
                    "{name}: it cannot draw into this window; if you have several graphics \
                     cards, check the monitor is plugged into one that works"
                )
            } else {
                format!("{name}: it cannot draw")
            });
        }
        None
    }

    /// Higher is better: a dedicated card beats a built-in one, which beats software.
    fn rank(&self) -> u8 {
        match self.kind {
            vk::PhysicalDeviceType::DISCRETE_GPU => 4,
            vk::PhysicalDeviceType::INTEGRATED_GPU => 3,
            vk::PhysicalDeviceType::VIRTUAL_GPU => 2,
            vk::PhysicalDeviceType::CPU => 1,
            _ => 0,
        }
    }
}

/// Picks the best usable card, or explains why each one was turned down.
///
/// Among equally good cards the first one listed wins, which is the order the
/// operating system prefers.
fn choose(candidates: &[Candidate], windowed: bool) -> Result<usize, RenderError> {
    let mut best: Option<usize> = None;
    let mut rejected = Vec::new();
    for (index, candidate) in candidates.iter().enumerate() {
        if let Some(reason) = candidate.rejection(windowed) {
            rejected.push(reason);
        } else if best.is_none_or(|best| candidate.rank() > candidates[best].rank()) {
            best = Some(index);
        }
    }
    best.ok_or(RenderError::NoSuitableGpu { rejected })
}

/// The graphics card the renderer draws with.
///
/// The raw Vulkan handles are public through the accessor methods so that
/// libraries which need them, such as FSR, can share this device.
pub struct Gpu {
    device: ash::Device,
    swapchains: swapchain::Device,
    physical: vk::PhysicalDevice,
    queue_family: u32,
    queue: vk::Queue,
    name: String,
    /// Last, so the device above is destroyed first.
    instance: Instance,
}

impl Gpu {
    /// Connects to the best graphics card, ready to draw into a window if one is given.
    ///
    /// # Safety
    ///
    /// The window behind the handles must stay open until the `Gpu` is dropped.
    pub(crate) unsafe fn new(
        window: Option<(RawDisplayHandle, RawWindowHandle)>,
    ) -> Result<Self, RenderError> {
        let windowed = window.is_some();
        // SAFETY: the caller promises the window outlives the returned `Gpu`,
        // which owns this instance.
        let instance = unsafe { Instance::new(window) }?;

        // SAFETY: `instance.raw` is a live Vulkan instance.
        let devices = unsafe { instance.raw.enumerate_physical_devices() }
            .map_err(RenderError::driver("looking for graphics cards"))?;
        let mut candidates = Vec::with_capacity(devices.len());
        let mut subsets = Vec::with_capacity(devices.len());
        for &device in &devices {
            let (candidate, subset) = inspect(&instance, device);
            candidates.push(candidate);
            subsets.push(subset);
        }
        let chosen = choose(&candidates, windowed)?;
        let physical = devices[chosen];
        let candidate = &candidates[chosen];
        let Some(queue_family) = candidate.queue_family else {
            // `choose` only returns cards that have one.
            return Err(RenderError::NoSuitableGpu {
                rejected: Vec::new(),
            });
        };

        let mut extensions: Vec<*const c_char> = Vec::new();
        if windowed {
            extensions.push(swapchain::NAME.as_ptr());
        }
        // Vulkan requires this to be switched on wherever it is offered (MoltenVK).
        if subsets[chosen] {
            extensions.push(portability_subset::NAME.as_ptr());
        }
        let priorities = [1.0];
        let queues = [vk::DeviceQueueCreateInfo::default()
            .queue_family_index(queue_family)
            .queue_priorities(&priorities)];
        let mut features = vk::PhysicalDeviceVulkan13Features::default()
            .dynamic_rendering(true)
            .synchronization2(true);
        let info = vk::DeviceCreateInfo::default()
            .queue_create_infos(&queues)
            .enabled_extension_names(&extensions)
            .push_next(&mut features);
        // SAFETY: `physical` came from this instance, and the extensions and
        // features asked for were all reported as supported by `inspect`.
        let device = unsafe { instance.raw.create_device(physical, &info, None) }
            .map_err(RenderError::driver("connecting to the graphics card"))?;
        // SAFETY: the device was created with one queue in this family.
        let queue = unsafe { device.get_device_queue(queue_family, 0) };

        log::info!(
            "Drawing with {} (Vulkan {}).",
            candidate.name,
            candidate.version
        );
        Ok(Self {
            swapchains: swapchain::Device::new(&instance.raw, &device),
            device,
            physical,
            queue_family,
            queue,
            name: candidate.name.clone(),
            instance,
        })
    }

    /// The name of the graphics card, as its driver reports it.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The Vulkan instance.
    #[must_use]
    pub fn instance(&self) -> &ash::Instance {
        &self.instance.raw
    }

    /// The Vulkan physical device.
    #[must_use]
    pub fn physical_device(&self) -> vk::PhysicalDevice {
        self.physical
    }

    /// The Vulkan device.
    #[must_use]
    pub fn device(&self) -> &ash::Device {
        &self.device
    }

    /// The queue all drawing is submitted to.
    #[must_use]
    pub fn queue(&self) -> vk::Queue {
        self.queue
    }

    /// The family [`queue`](Self::queue) belongs to.
    #[must_use]
    pub fn queue_family(&self) -> u32 {
        self.queue_family
    }

    /// The swapchain functions of the device.
    pub(crate) fn swapchains(&self) -> &swapchain::Device {
        &self.swapchains
    }

    /// The window surface functions and the surface itself.
    pub(crate) fn surface(&self) -> (&surface::Instance, vk::SurfaceKHR) {
        (&self.instance.surfaces, self.instance.surface)
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: everything created from the device holds an `Arc<Gpu>`, so
        // by the time this runs all of it has been destroyed.
        unsafe {
            // Nothing useful can be done if waiting fails; destroy regardless.
            let _ = self.device.device_wait_idle();
            self.device.destroy_device(None);
        }
    }
}

/// Reads what [`Candidate`] needs from one graphics card, and whether it
/// offers the portability subset extension.
fn inspect(instance: &Instance, device: vk::PhysicalDevice) -> (Candidate, bool) {
    let windowed = instance.surface != vk::SurfaceKHR::null();
    // SAFETY: `device` was just listed by this instance, and the feature
    // structs live until the call that fills them returns.
    unsafe {
        let properties = instance.raw.get_physical_device_properties(device);
        let version = Version::from_vk(properties.api_version);

        // Asking an older driver about 1.3 features is not allowed.
        let mut features13 = vk::PhysicalDeviceVulkan13Features::default();
        if version >= NEEDED {
            let mut features = vk::PhysicalDeviceFeatures2::default().push_next(&mut features13);
            instance
                .raw
                .get_physical_device_features2(device, &mut features);
        }

        let extensions = instance
            .raw
            .enumerate_device_extension_properties(device)
            .unwrap_or_default();
        let offers = |name: &CStr| {
            extensions
                .iter()
                .any(|extension| extension.extension_name_as_c_str() == Ok(name))
        };

        let families = instance
            .raw
            .get_physical_device_queue_family_properties(device);
        let queue_family = (0u32..).zip(&families).find_map(|(index, family)| {
            let draws = family.queue_flags.contains(vk::QueueFlags::GRAPHICS);
            let shows = !windowed
                || instance
                    .surfaces
                    .get_physical_device_surface_support(device, index, instance.surface)
                    .unwrap_or(false);
            (draws && shows).then_some(index)
        });

        let candidate = Candidate {
            name: properties
                .device_name_as_c_str()
                .map_or_else(|_| "Unnamed graphics card".into(), CStr::to_string_lossy)
                .into_owned(),
            kind: properties.device_type,
            version,
            dynamic_rendering: features13.dynamic_rendering == vk::TRUE,
            synchronization2: features13.synchronization2 == vk::TRUE,
            presents: offers(swapchain::NAME),
            queue_family,
        };
        (candidate, offers(portability_subset::NAME))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(name: &str, kind: vk::PhysicalDeviceType) -> Candidate {
        Candidate {
            name: name.to_owned(),
            kind,
            version: Version {
                major: 1,
                minor: 3,
                patch: 250,
            },
            dynamic_rendering: true,
            synchronization2: true,
            presents: true,
            queue_family: Some(0),
        }
    }

    #[test]
    fn a_dedicated_card_beats_a_built_in_one_and_software() {
        let cards = [
            card("software", vk::PhysicalDeviceType::CPU),
            card("built-in", vk::PhysicalDeviceType::INTEGRATED_GPU),
            card("dedicated", vk::PhysicalDeviceType::DISCRETE_GPU),
        ];
        assert_eq!(choose(&cards, true).unwrap(), 2);
    }

    #[test]
    fn the_first_of_two_equal_cards_wins() {
        let cards = [
            card("first", vk::PhysicalDeviceType::DISCRETE_GPU),
            card("second", vk::PhysicalDeviceType::DISCRETE_GPU),
        ];
        assert_eq!(choose(&cards, true).unwrap(), 0);
    }

    #[test]
    fn an_old_driver_is_skipped_for_a_slower_card_that_works() {
        let mut old = card("old", vk::PhysicalDeviceType::DISCRETE_GPU);
        old.version = Version {
            major: 1,
            minor: 2,
            patch: 198,
        };
        let cards = [old, card("software", vk::PhysicalDeviceType::CPU)];
        assert_eq!(choose(&cards, true).unwrap(), 1);
    }

    #[test]
    fn every_unusable_card_is_explained() {
        let mut old = card("Old Card", vk::PhysicalDeviceType::DISCRETE_GPU);
        old.version = Version {
            major: 1,
            minor: 1,
            patch: 0,
        };
        let mut partial = card("Partial Card", vk::PhysicalDeviceType::INTEGRATED_GPU);
        partial.synchronization2 = false;
        let mut blind = card("Blind Card", vk::PhysicalDeviceType::INTEGRATED_GPU);
        blind.queue_family = None;

        let Err(RenderError::NoSuitableGpu { rejected }) = choose(&[old, partial, blind], true)
        else {
            panic!("no card should have been chosen");
        };
        assert_eq!(
            rejected,
            [
                "Old Card: its driver supports Vulkan 1.1.0, older than 1.3",
                "Partial Card: its driver does not support everything in Vulkan 1.3 \
                 (usually fixed by updating the driver)",
                "Blind Card: it cannot draw into this window; if you have several graphics \
                 cards, check the monitor is plugged into one that works",
            ]
        );
    }

    #[test]
    fn a_card_that_cannot_show_pictures_is_fine_without_a_window() {
        let mut headless = card("compute", vk::PhysicalDeviceType::DISCRETE_GPU);
        headless.presents = false;
        assert!(choose(std::slice::from_ref(&headless), true).is_err());
        assert_eq!(choose(&[headless], false).unwrap(), 0);
    }

    #[test]
    fn no_cards_at_all_is_an_error() {
        assert!(matches!(
            choose(&[], true),
            Err(RenderError::NoSuitableGpu { rejected }) if rejected.is_empty()
        ));
    }
}
