//! Draws scenes without a window and checks the pixels.
//!
//! This runs wherever Vulkan 1.3 is installed, including on a software
//! renderer, and is skipped elsewhere. Set `FOSSPS_REQUIRE_VULKAN=1` to make
//! a missing Vulkan a failure instead, as CI does on Linux.

use std::sync::Arc;

use ash::vk;
use fossps_assets::{Instance, Material, Mesh, MeshPart, Scene, Transform};

use crate::camera::Camera;
use crate::error::RenderError;
use crate::gpu::{Gpu, validation_errors};
use crate::mesh::{MeshPipeline, Meshes};
use crate::scene::SceneTarget;
use crate::swapchain::COLOUR;
use crate::tonemap::{Then, TonemapPipeline, exposure_scale};

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

/// Draws `scene` as `camera` sees it, `stops` brighter than it is, into a
/// `SIZE` by `SIZE` image and returns its RGBA pixels.
fn draw(gpu: &Arc<Gpu>, scene: &Scene, camera: &Camera, stops: f32) -> Vec<u8> {
    let device = gpu.device();
    let extent = vk::Extent2D {
        width: SIZE,
        height: SIZE,
    };
    let bytes = u64::from(SIZE * SIZE * 4);
    let target = SceneTarget::new(gpu, extent).unwrap();
    let meshes = Meshes::new(gpu, scene).unwrap();
    let mesh = MeshPipeline::new(gpu).unwrap();
    let tonemap = TonemapPipeline::new(gpu, FORMAT).unwrap();

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
                    .memory_type_index(
                        gpu.memory_type(needs.memory_type_bits, vk::MemoryPropertyFlags::empty())
                            .unwrap(),
                    ),
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
                    .memory_type_index(
                        gpu.memory_type(
                            needs.memory_type_bits,
                            vk::MemoryPropertyFlags::HOST_VISIBLE
                                | vk::MemoryPropertyFlags::HOST_COHERENT,
                        )
                        .unwrap(),
                    ),
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
        tonemap.read_from(target.colour.view);
        mesh.record(
            commands,
            &target,
            &camera.clip_from_world(SIZE, SIZE),
            Some(&meshes),
        );
        tonemap.record(
            commands,
            image,
            view,
            extent,
            exposure_scale(stops),
            Then::Copy,
        );
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

/// Whether a pixel is the dark background: sRGB #181C24 with the tone
/// curve's toe taken off is about 2, 8, 23 out of 255.
fn is_background([red, green, blue]: [u8; 3]) -> bool {
    red <= 4 && green <= 14 && blue <= 36
}

const MIDDLE: (u32, u32) = (SIZE / 2, SIZE / 2);
const RED: [f32; 4] = [1.0, 0.25, 0.2, 1.0];
const GREEN: [f32; 4] = [0.25, 1.0, 0.3, 1.0];

/// The transform that moves a shape by `x`, `y`, `z` metres and makes it
/// `wide` times as wide.
fn placed(x: f32, y: f32, z: f32, wide: f32) -> Transform {
    [
        [wide, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
        [x, y, z, 1.0],
    ]
}

/// A square two metres across that stands upright at the origin and faces +Z.
fn square() -> Mesh {
    Mesh {
        name: Some("square".to_owned()),
        parts: vec![MeshPart {
            positions: vec![
                [-1.0, -1.0, 0.0],
                [1.0, -1.0, 0.0],
                [1.0, 1.0, 0.0],
                [-1.0, 1.0, 0.0],
            ],
            normals: vec![[0.0, 0.0, 1.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..MeshPart::default()
        }],
    }
}

/// A scene of squares: each has a colour, a transform and whether both of
/// its sides are drawn.
fn squares(list: &[([f32; 4], Transform, bool)]) -> Scene {
    let mut scene = Scene::default();
    for (number, &(base_color, transform, double_sided)) in list.iter().enumerate() {
        let mut mesh = square();
        mesh.parts[0].material = number as u32;
        scene.meshes.push(mesh);
        scene.materials.push(Material {
            base_color,
            double_sided,
            ..Material::default()
        });
        scene.instances.push(Instance {
            mesh: number as u32,
            transform,
            mirrored: transform[0][0] < 0.0,
            name: None,
            extras: None,
        });
    }
    scene
}

/// Ends a test: the validation layer must have had nothing to say.
fn finish(gpu: Arc<Gpu>) {
    drop(gpu);
    assert_eq!(
        validation_errors(),
        0,
        "the Vulkan validation layer reported mistakes; see the log above"
    );
}

#[test]
fn a_mesh_is_drawn_where_it_stands_over_the_background() {
    let Some(gpu) = gpu() else {
        return;
    };
    eprintln!("Drawing with {}.", gpu.name());
    let camera = Camera::default();

    // Five metres ahead, a two-metre square covers about a quarter of the picture.
    let ahead = draw(
        &gpu,
        &squares(&[(RED, placed(0.0, 0.0, -5.0, 1.0), false)]),
        &camera,
        0.0,
    );
    for (x, y) in [(0, 0), (SIZE - 1, 0), (0, SIZE - 1), (SIZE - 1, SIZE - 1)] {
        let colour = pixel(&ahead, x, y);
        assert!(is_background(colour), "corner {x},{y} is {colour:?}");
    }
    let [red, green, blue] = pixel(&ahead, MIDDLE.0, MIDDLE.1);
    assert!(
        red > 120 && red > green * 2 && red > blue * 2,
        "the middle should be the red square, but is {red},{green},{blue}"
    );
    assert!(is_background(pixel(&ahead, MIDDLE.0 + 13, MIDDLE.1)));
    assert!(is_background(pixel(&ahead, MIDDLE.0, MIDDLE.1 - 13)));

    // Moved a metre and a half to the right, it is right of the middle.
    let right = draw(
        &gpu,
        &squares(&[(RED, placed(1.5, 0.0, -5.0, 1.0), false)]),
        &camera,
        0.0,
    );
    assert!(is_background(pixel(&right, MIDDLE.0, MIDDLE.1)));
    assert!(!is_background(pixel(&right, MIDDLE.0 + 13, MIDDLE.1)));
    assert!(is_background(pixel(&right, MIDDLE.0 - 13, MIDDLE.1)));

    // Moved up, it is above the middle; turning the camera to look up at
    // it brings it back.
    let scene = squares(&[(RED, placed(0.0, 1.5, -5.0, 1.0), false)]);
    let above = draw(&gpu, &scene, &camera, 0.0);
    assert!(is_background(pixel(&above, MIDDLE.0, MIDDLE.1)));
    assert!(!is_background(pixel(&above, MIDDLE.0, MIDDLE.1 - 13)));
    assert!(is_background(pixel(&above, MIDDLE.0, MIDDLE.1 + 13)));
    let looking_up = Camera {
        pitch: (1.5_f32 / 5.0).atan(),
        ..camera
    };
    let found = draw(&gpu, &scene, &looking_up, 0.0);
    assert!(!is_background(pixel(&found, MIDDLE.0, MIDDLE.1)));

    // A scene with nothing in it is all background.
    let empty = draw(&gpu, &Scene::default(), &camera, 0.0);
    assert!(
        empty
            .chunks(4)
            .all(|rgba| is_background([rgba[0], rgba[1], rgba[2]]))
    );

    finish(gpu);
}

#[test]
fn the_nearer_surface_is_in_front_whichever_is_drawn_first() {
    let Some(gpu) = gpu() else {
        return;
    };
    // A red square three metres away and a green wall, eight times as wide,
    // behind it.
    let near = (RED, placed(0.0, 0.0, -3.0, 0.5), false);
    let far = (GREEN, placed(0.0, 0.0, -10.0, 8.0), false);
    for scene in [squares(&[near, far]), squares(&[far, near])] {
        let pixels = draw(&gpu, &scene, &Camera::default(), 0.0);
        let [red, green, _] = pixel(&pixels, MIDDLE.0, MIDDLE.1);
        assert!(red > green, "the near red square should hide the wall");
        let [red, green, _] = pixel(&pixels, MIDDLE.0 + 12, MIDDLE.1);
        assert!(green > red, "the far green wall should show beside it");
    }
    finish(gpu);
}

#[test]
fn the_back_of_a_surface_is_not_drawn_unless_it_has_two_sides() {
    let Some(gpu) = gpu() else {
        return;
    };
    // The square faces +Z. From behind it, looking back along +Z:
    let behind = Camera {
        position: [0.0, 0.0, -10.0],
        yaw: std::f32::consts::PI,
        ..Camera::default()
    };
    let at = placed(0.0, 0.0, -5.0, 1.0);
    let one_sided = draw(&gpu, &squares(&[(RED, at, false)]), &behind, 0.0);
    assert!(is_background(pixel(&one_sided, MIDDLE.0, MIDDLE.1)));
    let two_sided = draw(&gpu, &squares(&[(RED, at, true)]), &behind, 0.0);
    assert!(!is_background(pixel(&two_sided, MIDDLE.0, MIDDLE.1)));

    // A mirrored square still shows its front to a camera in front of it,
    // and still hides its back.
    let mirrored = squares(&[(RED, placed(0.0, 0.0, -5.0, -1.0), false)]);
    let front = draw(&gpu, &mirrored, &Camera::default(), 0.0);
    assert!(!is_background(pixel(&front, MIDDLE.0, MIDDLE.1)));
    let back = draw(&gpu, &mirrored, &behind, 0.0);
    assert!(is_background(pixel(&back, MIDDLE.0, MIDDLE.1)));

    finish(gpu);
}

#[test]
fn a_surface_facing_up_shows_its_material_colour_and_point_colours_tint_it() {
    let Some(gpu) = gpu() else {
        return;
    };
    // A grey floor two metres under a camera that looks straight down.
    let mut scene = Scene::default();
    scene.meshes.push(Mesh {
        name: None,
        parts: vec![MeshPart {
            positions: vec![
                [-9.0, 0.0, 9.0],
                [9.0, 0.0, 9.0],
                [9.0, 0.0, -9.0],
                [-9.0, 0.0, -9.0],
            ],
            normals: vec![[0.0, 1.0, 0.0]; 4],
            indices: vec![0, 1, 2, 0, 2, 3],
            ..MeshPart::default()
        }],
    });
    scene.materials.push(Material {
        base_color: [0.5, 0.5, 0.5, 1.0],
        ..Material::default()
    });
    scene.instances.push(Instance {
        mesh: 0,
        transform: placed(0.0, 0.0, 0.0, 1.0),
        mirrored: false,
        name: None,
        extras: None,
    });
    let down = Camera {
        position: [0.0, 2.0, 0.0],
        pitch: -std::f32::consts::FRAC_PI_2,
        ..Camera::default()
    };
    let pixels = draw(&gpu, &scene, &down, 0.0);
    // Half grey, less the tone curve's 0.04, is 181 out of 255 in sRGB.
    for (x, y) in [(0, 0), MIDDLE, (SIZE - 1, SIZE - 1)] {
        let grey = pixel(&pixels, x, y);
        assert!(
            grey.iter().all(|&channel| channel.abs_diff(181) <= 3),
            "the floor at {x},{y} should be mid grey, but is {grey:?}"
        );
    }

    // Colours painted on the points multiply the material's.
    scene.meshes[0].parts[0].colors = Some(vec![[1.0, 0.0, 0.0, 1.0]; 4]);
    let pixels = draw(&gpu, &scene, &down, 0.0);
    let [red, green, blue] = pixel(&pixels, MIDDLE.0, MIDDLE.1);
    // With no green or blue the tone curve takes nothing off: 188.
    assert!(red.abs_diff(188) <= 3 && green < 10 && blue < 10);

    finish(gpu);
}

#[test]
fn light_brighter_than_the_display_rolls_off_and_keeps_its_colour() {
    let Some(gpu) = gpu() else {
        return;
    };
    let scene = squares(&[(RED, placed(0.0, 0.0, -2.0, 1.0), false)]);
    let camera = Camera::default();
    let normal = draw(&gpu, &scene, &camera, 0.0);
    // Eight times the light: the red is now about 5, far over the display's
    // 1.0, which would be a flat 255 red with nothing done to it.
    let bright = draw(&gpu, &scene, &camera, 3.0);
    let dark = draw(&gpu, &scene, &camera, -3.0);

    let [red, green, blue] = pixel(&bright, MIDDLE.0, MIDDLE.1);
    assert!(
        red > green && red > blue,
        "the square should still be mostly red, but is {red},{green},{blue}"
    );
    assert!(
        red < 255,
        "the brightest red should stop short of the display's limit"
    );
    let [_, normal_green, _] = pixel(&normal, MIDDLE.0, MIDDLE.1);
    assert!(
        green > normal_green,
        "over-bright red should fade towards white: green {normal_green} became {green}"
    );

    // More light is never darker, and less is never brighter, anywhere.
    for index in 0..normal.len() {
        if index % 4 == 3 {
            continue;
        }
        assert!(bright[index] >= normal[index] && normal[index] >= dark[index]);
    }
    assert!(pixel(&dark, MIDDLE.0, MIDDLE.1)[1] < pixel(&normal, MIDDLE.0, MIDDLE.1)[1]);

    finish(gpu);
}
