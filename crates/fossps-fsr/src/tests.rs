//! Upscales a small picture on a real Vulkan device and checks the result.
//!
//! This runs wherever Vulkan 1.3 is installed, including on a software
//! renderer, and is skipped elsewhere. Set `FOSSPS_REQUIRE_VULKAN=1` to make
//! a missing Vulkan a failure instead, as CI does on Linux.

use std::ffi::{CStr, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU32, Ordering};

use ash::ext::debug_utils;
use ash::vk;

use crate::{Frame, FsrError, Image, Quality, REQUIRED_FEATURES, Upscaler, jitter};

const VALIDATION_LAYER: &CStr = c"VK_LAYER_KHRONOS_validation";

/// The SDK lets one device upscale at a time, and every test here makes its
/// own device, so the tests that make an upscaler take turns.
static ONE_DEVICE: Mutex<()> = Mutex::new(());

/// How many mistakes the validation layer has reported.
static VALIDATION_ERRORS: AtomicU32 = AtomicU32::new(0);

unsafe extern "system" fn on_message(
    severity: vk::DebugUtilsMessageSeverityFlagsEXT,
    _kind: vk::DebugUtilsMessageTypeFlagsEXT,
    data: *const vk::DebugUtilsMessengerCallbackDataEXT<'_>,
    _user: *mut c_void,
) -> vk::Bool32 {
    if severity.contains(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR) {
        VALIDATION_ERRORS.fetch_add(1, Ordering::Relaxed);
        // SAFETY: Vulkan passes a valid struct whose message, when not null,
        // is a string that lives for the length of this call.
        let text = unsafe { data.as_ref().and_then(|data| data.message_as_c_str()) };
        eprintln!(
            "Vulkan validation: {}",
            text.unwrap_or(c"?").to_string_lossy()
        );
    }
    vk::FALSE
}

/// A Vulkan device with nothing but what the upscaler needs.
struct Gpu {
    _entry: ash::Entry,
    instance: ash::Instance,
    debug: Option<(debug_utils::Instance, vk::DebugUtilsMessengerEXT)>,
    physical: vk::PhysicalDevice,
    device: ash::Device,
    queue: vk::Queue,
    family: u32,
}

impl Gpu {
    /// Connects to a graphics card, or returns `None` if this computer has none that will do.
    fn new() -> Option<Self> {
        let skip = |why: &str| {
            let required = std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
            assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but {why}");
            eprintln!("Skipping the upscaling test: {why}");
            None
        };
        // SAFETY: loading the system's Vulkan library is how Vulkan is started.
        let Ok(entry) = (unsafe { ash::Entry::load() }) else {
            return skip("Vulkan is not installed");
        };
        // SAFETY: every call below passes valid, live arguments to the
        // instance or device it was made from, and each object is stored in
        // the returned `Gpu`, whose `Drop` destroys it.
        unsafe {
            let layers = entry.enumerate_instance_layer_properties().unwrap();
            let validate = layers
                .iter()
                .any(|layer| layer.layer_name_as_c_str() == Ok(VALIDATION_LAYER));
            let app = vk::ApplicationInfo::default().api_version(vk::API_VERSION_1_3);
            let layer_names = [VALIDATION_LAYER.as_ptr()];
            let extension_names = [debug_utils::NAME.as_ptr()];
            let mut info = vk::InstanceCreateInfo::default().application_info(&app);
            if validate {
                info = info
                    .enabled_layer_names(&layer_names)
                    .enabled_extension_names(&extension_names);
            }
            let Ok(instance) = entry.create_instance(&info, None) else {
                return skip("Vulkan 1.3 is not available");
            };
            let debug = validate.then(|| {
                let debug = debug_utils::Instance::new(&entry, &instance);
                let messenger = debug
                    .create_debug_utils_messenger(
                        &vk::DebugUtilsMessengerCreateInfoEXT::default()
                            .message_severity(vk::DebugUtilsMessageSeverityFlagsEXT::ERROR)
                            // Not GENERAL: that reports other programs' broken
                            // layers, which are not this crate's mistakes.
                            .message_type(vk::DebugUtilsMessageTypeFlagsEXT::VALIDATION)
                            .pfn_user_callback(Some(on_message)),
                        None,
                    )
                    .unwrap();
                (debug, messenger)
            });

            let found = instance
                .enumerate_physical_devices()
                .unwrap()
                .into_iter()
                .find_map(|physical| {
                    let properties = instance.get_physical_device_properties(physical);
                    let features = instance.get_physical_device_features(physical);
                    let family = instance
                        .get_physical_device_queue_family_properties(physical)
                        .iter()
                        .position(|family| {
                            family
                                .queue_flags
                                .contains(vk::QueueFlags::GRAPHICS | vk::QueueFlags::COMPUTE)
                        })?;
                    let enough = properties.api_version >= vk::API_VERSION_1_3
                        && features.shader_storage_image_extended_formats == vk::TRUE
                        && features.shader_storage_image_write_without_format == vk::TRUE;
                    enough.then_some((physical, u32::try_from(family).unwrap(), properties))
                });
            let Some((physical, family, properties)) = found else {
                if let Some((debug, messenger)) = &debug {
                    debug.destroy_debug_utils_messenger(*messenger, None);
                }
                instance.destroy_instance(None);
                return skip("no graphics card here has Vulkan 1.3 and what FSR needs");
            };
            eprintln!(
                "Upscaling with {}.",
                properties.device_name_as_c_str().unwrap().to_string_lossy()
            );

            let queues = [vk::DeviceQueueCreateInfo::default()
                .queue_family_index(family)
                .queue_priorities(&[1.0])];
            let mut features13 =
                vk::PhysicalDeviceVulkan13Features::default().synchronization2(true);
            let device = instance
                .create_device(
                    physical,
                    &vk::DeviceCreateInfo::default()
                        .queue_create_infos(&queues)
                        .enabled_features(&REQUIRED_FEATURES)
                        .push_next(&mut features13),
                    None,
                )
                .unwrap();
            let queue = device.get_device_queue(family, 0);
            Some(Self {
                _entry: entry,
                instance,
                debug,
                physical,
                device,
                queue,
                family,
            })
        }
    }

    /// Makes an image with its own memory.
    fn image(&self, format: vk::Format, extent: vk::Extent2D, usage: vk::ImageUsageFlags) -> Made {
        // SAFETY: the device is alive and the settings are valid; the caller
        // frees what is returned before the device goes.
        unsafe {
            let image = self
                .device
                .create_image(
                    &vk::ImageCreateInfo::default()
                        .image_type(vk::ImageType::TYPE_2D)
                        .format(format)
                        .extent(extent.into())
                        .mip_levels(1)
                        .array_layers(1)
                        .samples(vk::SampleCountFlags::TYPE_1)
                        .usage(usage),
                    None,
                )
                .unwrap();
            let needs = self.device.get_image_memory_requirements(image);
            let memory = self.allocate(needs, vk::MemoryPropertyFlags::empty());
            self.device.bind_image_memory(image, memory, 0).unwrap();
            Made {
                image: Image {
                    image,
                    format,
                    usage,
                    extent,
                },
                memory,
            }
        }
    }

    /// Makes a buffer the CPU can read and write.
    fn buffer(&self, bytes: u64, usage: vk::BufferUsageFlags) -> (vk::Buffer, vk::DeviceMemory) {
        // SAFETY: as for `image`.
        unsafe {
            let buffer = self
                .device
                .create_buffer(
                    &vk::BufferCreateInfo::default().size(bytes).usage(usage),
                    None,
                )
                .unwrap();
            let needs = self.device.get_buffer_memory_requirements(buffer);
            let memory = self.allocate(
                needs,
                vk::MemoryPropertyFlags::HOST_VISIBLE | vk::MemoryPropertyFlags::HOST_COHERENT,
            );
            self.device.bind_buffer_memory(buffer, memory, 0).unwrap();
            (buffer, memory)
        }
    }

    fn allocate(
        &self,
        needs: vk::MemoryRequirements,
        wanted: vk::MemoryPropertyFlags,
    ) -> vk::DeviceMemory {
        // SAFETY: the instance and device are alive.
        unsafe {
            let memory = self
                .instance
                .get_physical_device_memory_properties(self.physical);
            let index = (0..memory.memory_type_count)
                .find(|index| {
                    needs.memory_type_bits & (1 << index) != 0
                        && memory.memory_types[*index as usize]
                            .property_flags
                            .contains(wanted)
                })
                .unwrap();
            self.device
                .allocate_memory(
                    &vk::MemoryAllocateInfo::default()
                        .allocation_size(needs.size)
                        .memory_type_index(index),
                    None,
                )
                .unwrap()
        }
    }

    /// Records commands with `record`, runs them and waits for them to finish.
    fn run(&self, pool: vk::CommandPool, record: impl FnOnce(vk::CommandBuffer)) {
        // SAFETY: the pool belongs to this device and nothing else uses it;
        // the wait makes sure the commands are done before the pool is reset.
        unsafe {
            self.device
                .reset_command_pool(pool, vk::CommandPoolResetFlags::empty())
                .unwrap();
            let commands = self
                .device
                .allocate_command_buffers(
                    &vk::CommandBufferAllocateInfo::default()
                        .command_pool(pool)
                        .command_buffer_count(1),
                )
                .unwrap()[0];
            self.device
                .begin_command_buffer(commands, &vk::CommandBufferBeginInfo::default())
                .unwrap();
            record(commands);
            self.device.end_command_buffer(commands).unwrap();
            let buffers = [vk::CommandBufferSubmitInfo::default().command_buffer(commands)];
            self.device
                .queue_submit2(
                    self.queue,
                    &[vk::SubmitInfo2::default().command_buffer_infos(&buffers)],
                    vk::Fence::null(),
                )
                .unwrap();
            self.device.queue_wait_idle(self.queue).unwrap();
            self.device.free_command_buffers(pool, &[commands]);
        }
    }

    /// Moves an image from one layout to another.
    fn layout(
        &self,
        commands: vk::CommandBuffer,
        image: vk::Image,
        from: vk::ImageLayout,
        to: vk::ImageLayout,
    ) {
        let barriers = [vk::ImageMemoryBarrier2::default()
            .src_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .src_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
            .dst_stage_mask(vk::PipelineStageFlags2::ALL_COMMANDS)
            .dst_access_mask(vk::AccessFlags2::MEMORY_READ | vk::AccessFlags2::MEMORY_WRITE)
            .old_layout(from)
            .new_layout(to)
            .image(image)
            .subresource_range(COLOUR)];
        // SAFETY: `commands` is recording and the image is alive.
        unsafe {
            self.device.cmd_pipeline_barrier2(
                commands,
                &vk::DependencyInfo::default().image_memory_barriers(&barriers),
            );
        }
    }

    fn free(&self, made: &Made) {
        // SAFETY: the graphics card is idle and nothing uses the image any more.
        unsafe {
            self.device.destroy_image(made.image.image, None);
            self.device.free_memory(made.memory, None);
        }
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        // SAFETY: everything made from the device has been destroyed by now.
        unsafe {
            self.device.destroy_device(None);
            if let Some((debug, messenger)) = &self.debug {
                debug.destroy_debug_utils_messenger(*messenger, None);
            }
            self.instance.destroy_instance(None);
        }
    }
}

/// An image and the memory behind it.
struct Made {
    image: Image,
    memory: vk::DeviceMemory,
}

const COLOUR: vk::ImageSubresourceRange = vk::ImageSubresourceRange {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    base_mip_level: 0,
    level_count: 1,
    base_array_layer: 0,
    layer_count: 1,
};
const LAYER: vk::ImageSubresourceLayers = vk::ImageSubresourceLayers {
    aspect_mask: vk::ImageAspectFlags::COLOR,
    mip_level: 0,
    base_array_layer: 0,
    layer_count: 1,
};

const RENDER: vk::Extent2D = vk::Extent2D {
    width: 64,
    height: 48,
};
const OUTPUT: vk::Extent2D = vk::Extent2D {
    width: 128,
    height: 96,
};

/// 16-bit floats: 2.0 and about 0.1.
const BRIGHT: u16 = 0x4000;
const DIM: u16 = 0x2e66;

/// A 16-bit float as a number.
fn half(bits: u16) -> f32 {
    let sign = if bits & 0x8000 == 0 { 1.0 } else { -1.0 };
    let exponent = i32::from((bits >> 10) & 0x1f);
    let fraction = f32::from(bits & 0x3ff) / 1024.0;
    sign * match exponent {
        0 => fraction * 2f32.powi(-14),
        31 if fraction == 0.0 => f32::INFINITY,
        31 => f32::NAN,
        _ => (1.0 + fraction) * 2f32.powi(exponent - 15),
    }
}

/// Upscales a picture that is red on the left and blue on the right for
/// `frames` frames, and returns the red, green and blue of every output pixel.
fn upscale(gpu: &Gpu, frames: u32, sharpness: f32) -> Vec<[f32; 3]> {
    let device = &gpu.device;
    let sampled = vk::ImageUsageFlags::SAMPLED | vk::ImageUsageFlags::TRANSFER_DST;
    let colour = gpu.image(vk::Format::R16G16B16A16_SFLOAT, RENDER, sampled);
    let depth = gpu.image(vk::Format::R32_SFLOAT, RENDER, sampled);
    let motion = gpu.image(vk::Format::R16G16_SFLOAT, RENDER, sampled);
    let output = gpu.image(
        vk::Format::R16G16B16A16_SFLOAT,
        OUTPUT,
        vk::ImageUsageFlags::STORAGE
            | vk::ImageUsageFlags::SAMPLED
            | vk::ImageUsageFlags::TRANSFER_SRC,
    );

    let in_bytes = u64::from(RENDER.width * RENDER.height * 8);
    let out_bytes = u64::from(OUTPUT.width * OUTPUT.height * 8);
    let (upload, upload_memory) = gpu.buffer(in_bytes, vk::BufferUsageFlags::TRANSFER_SRC);
    let (download, download_memory) = gpu.buffer(out_bytes, vk::BufferUsageFlags::TRANSFER_DST);

    // SAFETY: every object is created on the live device from valid settings,
    // used only by submissions that are waited for, and destroyed at the end,
    // after the upscaler that used them.
    unsafe {
        let mapped = device
            .map_memory(upload_memory, 0, in_bytes, vk::MemoryMapFlags::empty())
            .unwrap()
            .cast::<u16>();
        for y in 0..RENDER.height {
            for x in 0..RENDER.width {
                let left = x < RENDER.width / 2;
                let pixel = if left {
                    [BRIGHT, DIM, DIM, 0x3c00]
                } else {
                    [DIM, DIM, BRIGHT, 0x3c00]
                };
                let at = ((y * RENDER.width + x) * 4) as usize;
                std::ptr::copy_nonoverlapping(pixel.as_ptr(), mapped.add(at), 4);
            }
        }
        device.unmap_memory(upload_memory);

        let pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(gpu.family),
                None,
            )
            .unwrap();
        let (undefined, to_write, to_read) = (
            vk::ImageLayout::UNDEFINED,
            vk::ImageLayout::TRANSFER_DST_OPTIMAL,
            vk::ImageLayout::SHADER_READ_ONLY_OPTIMAL,
        );
        gpu.run(pool, |commands| {
            for made in [&colour, &depth, &motion] {
                gpu.layout(commands, made.image.image, undefined, to_write);
            }
            device.cmd_copy_buffer_to_image(
                commands,
                upload,
                colour.image.image,
                to_write,
                &[vk::BufferImageCopy::default()
                    .image_subresource(LAYER)
                    .image_extent(RENDER.into())],
            );
            // Everything is a quarter of the way to infinitely far, and still.
            for (made, value) in [(&depth, 0.25), (&motion, 0.0)] {
                device.cmd_clear_color_image(
                    commands,
                    made.image.image,
                    to_write,
                    &vk::ClearColorValue {
                        float32: [value; 4],
                    },
                    &[COLOUR],
                );
            }
            for made in [&colour, &depth, &motion] {
                gpu.layout(commands, made.image.image, to_write, to_read);
            }
            gpu.layout(
                commands,
                output.image.image,
                undefined,
                vk::ImageLayout::GENERAL,
            );
        });

        let mut upscaler =
            Upscaler::new(&gpu.instance, gpu.physical, device, RENDER, OUTPUT).unwrap();
        for frame in 0..frames {
            gpu.run(pool, |commands| {
                upscaler
                    .upscale(
                        commands,
                        &Frame {
                            color: colour.image,
                            depth: depth.image,
                            motion: motion.image,
                            output: output.image,
                            jitter: jitter(frame, RENDER.width, OUTPUT.width),
                            motion_scale: (1.0, 1.0),
                            sharpness,
                            frame_seconds: 1.0 / 60.0,
                            camera_near: 0.1,
                            camera_fov_y: 1.0,
                            reset: frame == 0,
                        },
                    )
                    .unwrap();
            });
        }

        gpu.run(pool, |commands| {
            gpu.layout(
                commands,
                output.image.image,
                vk::ImageLayout::GENERAL,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            );
            device.cmd_copy_image_to_buffer(
                commands,
                output.image.image,
                vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
                download,
                &[vk::BufferImageCopy::default()
                    .image_subresource(LAYER)
                    .image_extent(OUTPUT.into())],
            );
        });
        let mapped = device
            .map_memory(download_memory, 0, out_bytes, vk::MemoryMapFlags::empty())
            .unwrap()
            .cast::<u16>();
        let count = (OUTPUT.width * OUTPUT.height) as usize;
        let pixels = std::slice::from_raw_parts(mapped, count * 4)
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| [half(pixel[0]), half(pixel[1]), half(pixel[2])])
            .collect();
        device.unmap_memory(download_memory);

        drop(upscaler);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(upload, None);
        device.free_memory(upload_memory, None);
        device.destroy_buffer(download, None);
        device.free_memory(download_memory, None);
        for made in [&colour, &depth, &motion, &output] {
            gpu.free(made);
        }
        pixels
    }
}

fn pixel(pixels: &[[f32; 3]], x: u32, y: u32) -> [f32; 3] {
    pixels[(y * OUTPUT.width + x) as usize]
}

#[test]
fn a_small_picture_becomes_a_big_one_with_the_same_colours() {
    let _turn = ONE_DEVICE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(gpu) = Gpu::new() else {
        return;
    };
    for sharpness in [0.0, 0.5] {
        let pixels = upscale(&gpu, 4, sharpness);
        assert!(
            pixels.iter().flatten().all(|value| value.is_finite()),
            "every output pixel should be a number"
        );

        // Well inside each half the colour is the one that went in.
        for y in [OUTPUT.height / 4, OUTPUT.height / 2, OUTPUT.height * 3 / 4] {
            let [red, green, blue] = pixel(&pixels, OUTPUT.width / 4, y);
            assert!(
                red > 1.0 && red < 3.0 && green < 0.5 && blue < 0.5,
                "the left should be bright red, but is {red}, {green}, {blue}"
            );
            let [red, green, blue] = pixel(&pixels, OUTPUT.width * 3 / 4, y);
            assert!(
                blue > 1.0 && blue < 3.0 && green < 0.5 && red < 0.5,
                "the right should be bright blue, but is {red}, {green}, {blue}"
            );
        }
    }

    // A second upscaler after the first was dropped works too.
    let again = upscale(&gpu, 1, 0.0);
    assert!(pixel(&again, OUTPUT.width / 4, OUTPUT.height / 2)[0] > 1.0);

    drop(gpu);
    assert_eq!(
        VALIDATION_ERRORS.load(Ordering::Relaxed),
        0,
        "the Vulkan validation layer reported mistakes; see the log above"
    );
}

#[test]
fn sizes_the_upscaler_cannot_work_with_are_refused_before_vulkan_is_touched() {
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let none = vk::Extent2D {
        width: 0,
        height: 48,
    };
    // SAFETY: the device is alive and made with the required features; both
    // calls return before using it.
    let (empty, backwards) = unsafe {
        (
            Upscaler::new(&gpu.instance, gpu.physical, &gpu.device, none, OUTPUT),
            Upscaler::new(&gpu.instance, gpu.physical, &gpu.device, OUTPUT, RENDER),
        )
    };
    assert_eq!(empty.unwrap_err(), FsrError::EmptySize);
    let error = backwards.unwrap_err();
    assert!(matches!(error, FsrError::RenderLargerThanOutput { .. }));
    assert!(error.to_string().contains("128x96"), "{error}");
}

#[test]
fn an_image_the_upscaler_cannot_use_is_refused_with_what_to_change() {
    let _turn = ONE_DEVICE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let Some(gpu) = Gpu::new() else {
        return;
    };
    let sampled = vk::ImageUsageFlags::SAMPLED;
    let input = |format| gpu.image(format, RENDER, sampled);
    let colour = input(vk::Format::R16G16B16A16_SFLOAT);
    let depth = input(vk::Format::R32_SFLOAT);
    let motion = input(vk::Format::R16G16_SFLOAT);
    // Not created for writing from a compute shader.
    let output = gpu.image(vk::Format::R16G16B16A16_SFLOAT, OUTPUT, sampled);
    let mut frame = Frame {
        color: colour.image,
        depth: depth.image,
        motion: motion.image,
        output: output.image,
        jitter: (0.0, 0.0),
        motion_scale: (1.0, 1.0),
        sharpness: 0.0,
        frame_seconds: 1.0 / 60.0,
        camera_near: 0.1,
        camera_fov_y: 1.0,
        reset: true,
    };
    // SAFETY: the device is alive and has the required features. Every call
    // to `upscale` below is refused before the command buffer, which is
    // null, or any image is touched.
    unsafe {
        let mut upscaler =
            Upscaler::new(&gpu.instance, gpu.physical, &gpu.device, RENDER, OUTPUT).unwrap();
        let none = vk::CommandBuffer::null();

        let error = upscaler.upscale(none, &frame).unwrap_err();
        assert_eq!(
            error,
            FsrError::WrongUsage {
                what: "output picture",
                needs: "STORAGE"
            }
        );
        assert!(error.to_string().contains("VK_IMAGE_USAGE_STORAGE_BIT"));

        frame.output.usage |= vk::ImageUsageFlags::STORAGE;
        frame.depth.image = vk::Image::null();
        assert_eq!(
            upscaler.upscale(none, &frame).unwrap_err(),
            FsrError::MissingImage {
                what: "depth buffer"
            }
        );

        frame.depth = depth.image;
        frame.motion.extent.width += 1;
        assert!(matches!(
            upscaler.upscale(none, &frame).unwrap_err(),
            FsrError::WrongSize {
                what: "motion vectors",
                ..
            }
        ));
        drop(upscaler);
    }
    for made in [&colour, &depth, &motion, &output] {
        gpu.free(made);
    }
}

#[test]
fn quality_names_give_amds_render_sizes() {
    let window = vk::Extent2D {
        width: 1920,
        height: 1080,
    };
    let size = |quality: Quality| {
        let size = quality.render_size(window);
        (size.width, size.height)
    };
    assert_eq!(size(Quality::NativeAa), (1920, 1080));
    assert_eq!(size(Quality::Quality), (1280, 720));
    assert_eq!(size(Quality::Balanced), (1129, 635));
    assert_eq!(size(Quality::Performance), (960, 540));
    assert_eq!(size(Quality::UltraPerformance), (640, 360));
    let tiny = vk::Extent2D {
        width: 1,
        height: 1,
    };
    assert_eq!(Quality::UltraPerformance.render_size(tiny), tiny);
}

#[test]
fn the_jitter_stays_within_a_pixel_and_repeats() {
    // Twice as wide each way gives AMD's 32 different shifts.
    let shifts: Vec<(f32, f32)> = (0..32).map(|frame| jitter(frame, 960, 1920)).collect();
    for (x, y) in &shifts {
        assert!((-0.5..=0.5).contains(x) && (-0.5..=0.5).contains(y));
    }
    for (index, shift) in shifts.iter().enumerate() {
        assert!(!shifts[..index].contains(shift), "shift {index} repeats");
    }
    assert_eq!(jitter(32, 960, 1920), shifts[0]);
    // AMD's counts: 8 at full size, 18 for Quality, 72 for Ultra Performance.
    assert_eq!(crate::jitter_phases(1920, 1920), 8);
    assert_eq!(crate::jitter_phases(1280, 1920), 18);
    assert_eq!(crate::jitter_phases(640, 1920), 72);
    assert_eq!(crate::jitter_phases(1, u32::MAX), 1024);
    // A width of zero is treated as one pixel rather than dividing by it.
    let (x, y) = jitter(3, 0, 0);
    assert!(x.is_finite() && y.is_finite());
}

#[test]
fn half_floats_read_back_as_the_numbers_written() {
    assert_eq!(half(BRIGHT), 2.0);
    assert_eq!(half(0x3c00), 1.0);
    assert_eq!(half(0), 0.0);
    assert!((half(DIM) - 0.1).abs() < 0.001);
    assert_eq!(half(0xc000), -2.0);
}
