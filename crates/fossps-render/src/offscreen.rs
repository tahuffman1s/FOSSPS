//! Draws the triangle without a window and checks the pixels.
//!
//! This runs wherever Vulkan 1.3 is installed, including on a software
//! renderer, and is skipped elsewhere. Set `FOSSPS_REQUIRE_VULKAN=1` to make
//! a missing Vulkan a failure instead, as CI does on Linux.

use std::sync::Arc;

use ash::vk;

use crate::error::RenderError;
use crate::gpu::{Gpu, validation_errors};
use crate::swapchain::COLOUR;
use crate::triangle::{Then, TrianglePipeline};

const SIZE: u32 = 64;
const FORMAT: vk::Format = vk::Format::R8G8B8A8_UNORM;

/// Connects to a graphics card, or returns `None` if this computer has none that will do.
fn gpu() -> Option<Arc<Gpu>> {
    // SAFETY: there is no window, so nothing has to outlive the `Gpu`.
    match unsafe { Gpu::new(None) } {
        Ok(gpu) => Some(Arc::new(gpu)),
        Err(
            error @ (RenderError::NoVulkan(_)
            | RenderError::OldVulkan { .. }
            | RenderError::NoSuitableGpu { .. }),
        ) => {
            let required = std::env::var("FOSSPS_REQUIRE_VULKAN").is_ok_and(|value| value != "0");
            assert!(!required, "FOSSPS_REQUIRE_VULKAN is set, but: {error}");
            eprintln!("Skipping the drawing test: {error}");
            None
        }
        Err(error) => panic!("{error}"),
    }
}

/// Finds a kind of memory that `allowed` permits and that has the `wanted` properties.
fn memory_type(gpu: &Gpu, allowed: u32, wanted: vk::MemoryPropertyFlags) -> u32 {
    // SAFETY: the physical device belongs to the live instance.
    let memory = unsafe {
        gpu.instance()
            .get_physical_device_memory_properties(gpu.physical_device())
    };
    (0..memory.memory_type_count)
        .find(|&index| {
            allowed & (1 << index) != 0
                && memory.memory_types[index as usize]
                    .property_flags
                    .contains(wanted)
        })
        .expect("the graphics card has no memory of the kind the test needs")
}

/// Draws the triangle into a `SIZE` by `SIZE` image and returns its RGBA pixels.
fn draw(gpu: &Arc<Gpu>) -> Vec<u8> {
    let device = gpu.device();
    let extent = vk::Extent2D {
        width: SIZE,
        height: SIZE,
    };
    let bytes = u64::from(SIZE * SIZE * 4);
    let pipeline = TrianglePipeline::new(gpu, FORMAT).unwrap();

    // SAFETY: every object is created on the live device from valid settings,
    // used only by the single submission below, which is waited for, and
    // destroyed once that has finished.
    unsafe {
        let image = device
            .create_image(
                &vk::ImageCreateInfo::default()
                    .image_type(vk::ImageType::TYPE_2D)
                    .format(FORMAT)
                    .extent(extent.into())
                    .mip_levels(1)
                    .array_layers(1)
                    .samples(vk::SampleCountFlags::TYPE_1)
                    .usage(
                        vk::ImageUsageFlags::COLOR_ATTACHMENT | vk::ImageUsageFlags::TRANSFER_SRC,
                    ),
                None,
            )
            .unwrap();
        let needs = device.get_image_memory_requirements(image);
        let image_memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(needs.size)
                    .memory_type_index(memory_type(
                        gpu,
                        needs.memory_type_bits,
                        vk::MemoryPropertyFlags::empty(),
                    )),
                None,
            )
            .unwrap();
        device.bind_image_memory(image, image_memory, 0).unwrap();
        let view = device
            .create_image_view(
                &vk::ImageViewCreateInfo::default()
                    .image(image)
                    .view_type(vk::ImageViewType::TYPE_2D)
                    .format(FORMAT)
                    .subresource_range(COLOUR),
                None,
            )
            .unwrap();

        let buffer = device
            .create_buffer(
                &vk::BufferCreateInfo::default()
                    .size(bytes)
                    .usage(vk::BufferUsageFlags::TRANSFER_DST),
                None,
            )
            .unwrap();
        let needs = device.get_buffer_memory_requirements(buffer);
        let buffer_memory = device
            .allocate_memory(
                &vk::MemoryAllocateInfo::default()
                    .allocation_size(needs.size)
                    .memory_type_index(memory_type(
                        gpu,
                        needs.memory_type_bits,
                        vk::MemoryPropertyFlags::HOST_VISIBLE
                            | vk::MemoryPropertyFlags::HOST_COHERENT,
                    )),
                None,
            )
            .unwrap();
        device.bind_buffer_memory(buffer, buffer_memory, 0).unwrap();

        let pool = device
            .create_command_pool(
                &vk::CommandPoolCreateInfo::default().queue_family_index(gpu.queue_family()),
                None,
            )
            .unwrap();
        let commands = device
            .allocate_command_buffers(
                &vk::CommandBufferAllocateInfo::default()
                    .command_pool(pool)
                    .command_buffer_count(1),
            )
            .unwrap()[0];
        device
            .begin_command_buffer(commands, &vk::CommandBufferBeginInfo::default())
            .unwrap();
        pipeline.record(commands, image, view, extent, Then::Copy);
        device.cmd_copy_image_to_buffer(
            commands,
            image,
            vk::ImageLayout::TRANSFER_SRC_OPTIMAL,
            buffer,
            &[vk::BufferImageCopy::default()
                .image_subresource(vk::ImageSubresourceLayers {
                    aspect_mask: vk::ImageAspectFlags::COLOR,
                    mip_level: 0,
                    base_array_layer: 0,
                    layer_count: 1,
                })
                .image_extent(extent.into())],
        );
        device.end_command_buffer(commands).unwrap();

        let fence = device
            .create_fence(&vk::FenceCreateInfo::default(), None)
            .unwrap();
        let buffers = [vk::CommandBufferSubmitInfo::default().command_buffer(commands)];
        device
            .queue_submit2(
                gpu.queue(),
                &[vk::SubmitInfo2::default().command_buffer_infos(&buffers)],
                fence,
            )
            .unwrap();
        device.wait_for_fences(&[fence], true, u64::MAX).unwrap();

        let mapped = device
            .map_memory(buffer_memory, 0, bytes, vk::MemoryMapFlags::empty())
            .unwrap();
        let pixels =
            std::slice::from_raw_parts(mapped.cast::<u8>(), usize::try_from(bytes).unwrap())
                .to_vec();
        device.unmap_memory(buffer_memory);

        device.destroy_fence(fence, None);
        device.destroy_command_pool(pool, None);
        device.destroy_buffer(buffer, None);
        device.free_memory(buffer_memory, None);
        device.destroy_image_view(view, None);
        device.destroy_image(image, None);
        device.free_memory(image_memory, None);
        pixels
    }
}

/// The red, green and blue of the pixel at `x`, `y`, counted from the top left.
fn pixel(pixels: &[u8], x: u32, y: u32) -> [u8; 3] {
    let start = ((y * SIZE + x) * 4) as usize;
    [pixels[start], pixels[start + 1], pixels[start + 2]]
}

#[test]
fn the_triangle_is_drawn_over_the_background() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!("Drawing with {}.", gpu.name());
    let pixels = draw(&gpu);

    // The background is nearly black: linear 0.0091, 0.0116, 0.0176 is 2, 3, 4 out of 255.
    for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
        let [red, green, blue] = pixel(&pixels, x, y);
        assert!(
            red <= 4 && green <= 5 && blue <= 6,
            "corner {x},{y} should be background, but is {red},{green},{blue}"
        );
    }

    // The middle is inside the triangle, where all three corner colours mix.
    let [red, green, blue] = pixel(&pixels, SIZE / 2, SIZE / 2);
    assert!(
        red > 80 && green > 80 && blue > 80,
        "the middle should be inside the triangle, but is {red},{green},{blue}"
    );

    // Each corner of the triangle is mostly its own colour: red on top,
    // green at the bottom right and blue at the bottom left.
    let [red, green, blue] = pixel(&pixels, SIZE / 2, SIZE / 4);
    assert!(red > green && red > blue, "the top should be mostly red");
    let [red, green, blue] = pixel(&pixels, SIZE * 5 / 8 + 4, SIZE * 3 / 4);
    assert!(
        green > red && green > blue,
        "the bottom right should be mostly green"
    );
    let [red, green, blue] = pixel(&pixels, SIZE * 3 / 8 - 4, SIZE * 3 / 4);
    assert!(
        blue > red && blue > green,
        "the bottom left should be mostly blue"
    );

    // Just above the tip and just below the base is background again.
    assert!(pixel(&pixels, SIZE / 2, 4)[0] <= 4);
    assert!(pixel(&pixels, SIZE / 2, SIZE - 5)[0] <= 4);

    drop(gpu);
    assert_eq!(
        validation_errors(),
        0,
        "the Vulkan validation layer reported mistakes; see the log above"
    );
}
