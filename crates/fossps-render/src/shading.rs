//! Draws materials without a window and checks the pixels: textures, how
//! metallic and rough a surface is, normal maps, baked shadowing, glow,
//! cut-out and see-through surfaces, and the sun.
//!
//! Skipped where there is no Vulkan 1.3, as the tests in `offscreen` are.

use std::f32::consts::{FRAC_PI_2, PI};

use fossps_assets::{
    AlphaMode, Filter, Image, Instance, Light, LightKind, Material, Mesh, MeshPart, Sampler, Scene,
    Texture, TextureRef, TextureTransform, Wrap,
};

use crate::camera::Camera;
use crate::offscreen::{MIDDLE, SIZE, draw, finish, gpu, is_background, pixel, placed, squares};

/// A matte white floor two metres across at height 0, facing up. Seen by
/// [`above`], its texture lies the way it would in an image viewer: u runs
/// to the right and v runs down. Its second UV map runs right to left.
fn floor(material: Material) -> Scene {
    let corners = [[-1.0, 1.0], [1.0, 1.0], [1.0, -1.0], [-1.0, -1.0]];
    let uv = |[x, z]: [f32; 2]| [(x + 1.0) / 2.0, (z + 1.0) / 2.0];
    let mut scene = Scene::default();
    scene.meshes.push(Mesh {
        name: None,
        parts: vec![MeshPart {
            positions: corners.map(|[x, z]| [x, 0.0, z]).to_vec(),
            normals: vec![[0.0, 1.0, 0.0]; 4],
            tangents: Some(vec![[1.0, 0.0, 0.0, 1.0]; 4]),
            uv0: Some(corners.map(uv).to_vec()),
            uv1: Some(corners.map(|[x, z]| uv([-x, z])).to_vec()),
            indices: vec![0, 1, 2, 0, 2, 3],
            ..MeshPart::default()
        }],
    });
    scene.materials.push(material);
    scene.instances.push(Instance {
        mesh: 0,
        transform: placed(0.0, 0.0, 0.0, 1.0),
        mirrored: false,
        name: None,
        extras: None,
    });
    scene
}

/// A camera one metre over the middle of the [`floor`], looking straight
/// down: the floor fills the picture, +X to the right and +Z down.
fn above() -> Camera {
    Camera {
        position: [0.0, 1.0, 0.0],
        pitch: -FRAC_PI_2,
        ..Camera::default()
    }
}

/// Sharp pixels, no smaller copies, and the edge colour past the edge.
const SHARP: Sampler = Sampler {
    wrap_u: Wrap::ClampToEdge,
    wrap_v: Wrap::ClampToEdge,
    magnify: Filter::Nearest,
    minify: Filter::Nearest,
    mipmaps: None,
};

/// Adds a picture `width` pixels wide to `scene` and says how to use it.
fn texture(scene: &mut Scene, width: u32, pixels: &[[u8; 4]], sampler: Sampler) -> TextureRef {
    scene.images.push(Image {
        name: None,
        width,
        height: pixels.len() as u32 / width,
        pixels: pixels.concat(),
    });
    scene.textures.push(Texture {
        name: None,
        image: scene.images.len() as u32 - 1,
        sampler,
    });
    TextureRef {
        texture: scene.textures.len() as u32 - 1,
        uv_set: 0,
        transform: None,
    }
}

/// A sun for a scene: `direction` is the way it shines.
fn sun(direction: [f32; 3], color: [f32; 3]) -> Light {
    Light {
        name: None,
        kind: LightKind::Sun,
        color,
        intensity: PI,
        range: None,
        position: [0.0; 3],
        direction,
    }
}

/// How bright a pixel is: its three colours added up.
fn light([red, green, blue]: [u8; 3]) -> u32 {
    u32::from(red) + u32::from(green) + u32::from(blue)
}

fn near(one: [u8; 3], other: [u8; 3], by: u8) -> bool {
    one.iter().zip(other).all(|(a, b)| a.abs_diff(b) <= by)
}

const LEFT: (u32, u32) = (SIZE / 4, SIZE / 2);
const RIGHT: (u32, u32) = (3 * SIZE / 4, SIZE / 2);

#[test]
fn a_texture_is_laid_over_the_surface_in_its_own_colours() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut scene = floor(Material::default());
    // Red and green above blue and a grey that is half the light of white.
    let quarters = [
        [255, 0, 0, 255],
        [0, 255, 0, 255],
        [0, 0, 255, 255],
        [188, 188, 188, 255],
    ];
    let mut placing = texture(&mut scene, 2, &quarters, SHARP);
    scene.materials[0].base_color_texture = Some(placing);
    let pixels = draw(&gpu, &scene, &above(), 0.0);
    let at = |pixels: &[u8], x: u32, y: u32| pixel(pixels, x * SIZE / 4, y * SIZE / 4);
    let [red, green, blue] = at(&pixels, 1, 1);
    assert!(
        red > 150 && green < 50 && blue < 50,
        "top left: {red},{green},{blue}"
    );
    let [red, green, blue] = at(&pixels, 3, 1);
    assert!(
        green > 150 && red < 50 && blue < 50,
        "top right: {red},{green},{blue}"
    );
    let [red, green, blue] = at(&pixels, 1, 3);
    assert!(
        blue > 150 && red < 50 && green < 50,
        "bottom left: {red},{green},{blue}"
    );

    // The texture's colours are sRGB: its grey is drawn as a material
    // whose colour is half of white is.
    let plain = floor(Material {
        base_color: [0.5, 0.5, 0.5, 1.0],
        ..Material::default()
    });
    let plain = draw(&gpu, &plain, &above(), 0.0);
    assert!(
        near(at(&pixels, 3, 3), at(&plain, 3, 3), 2),
        "{:?} is not {:?}",
        at(&pixels, 3, 3),
        at(&plain, 3, 3)
    );

    // The material's own colour multiplies the texture's.
    scene.materials[0].base_color = [1.0, 0.0, 0.0, 1.0];
    let tinted = draw(&gpu, &scene, &above(), 0.0);
    assert!(light(at(&tinted, 3, 1)) < 40, "green times red is black");
    assert!(at(&tinted, 1, 1)[0] > 150);
    scene.materials[0].base_color = [1.0; 4];

    // The second UV map runs the other way, so red is now on the right.
    placing.uv_set = 1;
    scene.materials[0].base_color_texture = Some(placing);
    let mirrored = draw(&gpu, &scene, &above(), 0.0);
    assert!(at(&mirrored, 3, 1)[0] > 150 && at(&mirrored, 1, 1)[1] > 150);

    // Moved half its width along, the texture shows green where red was,
    // and past its edge the edge colour goes on.
    placing.uv_set = 0;
    placing.transform = Some(TextureTransform {
        offset: [0.5, 0.0],
        rotation: 0.0,
        scale: [1.0, 1.0],
    });
    scene.materials[0].base_color_texture = Some(placing);
    let moved = draw(&gpu, &scene, &above(), 0.0);
    assert!(at(&moved, 1, 1)[1] > 150 && at(&moved, 3, 1)[1] > 150);

    finish(gpu);
}

#[test]
fn a_texture_seen_small_is_read_from_a_smaller_copy_of_itself() {
    let Some(gpu) = gpu() else {
        return;
    };
    // Black and white squares one pixel across, repeated so that several
    // fall in every pixel of the picture.
    let checks: Vec<[u8; 4]> = (0..64 * 64)
        .map(|at| {
            if (at % 64 + at / 64) % 2 == 0 {
                [255; 4]
            } else {
                [0, 0, 0, 255]
            }
        })
        .collect();
    let spread = |sampler: Sampler| {
        let mut scene = floor(Material::default());
        let mut placing = texture(&mut scene, 64, &checks, sampler);
        placing.transform = Some(TextureTransform {
            offset: [0.0, 0.0],
            rotation: 0.0,
            scale: [7.3, 7.3],
        });
        scene.materials[0].base_color_texture = Some(placing);
        let pixels = draw(&gpu, &scene, &above(), 0.0);
        let greens = pixels.as_chunks::<4>().0.iter().map(|pixel| pixel[1]);
        (
            greens.clone().min().unwrap_or(0),
            greens.max().unwrap_or(0),
            pixel(&pixels, MIDDLE.0, MIDDLE.1),
        )
    };
    let (darkest, brightest, _) = spread(Sampler {
        wrap_u: Wrap::Repeat,
        wrap_v: Wrap::Repeat,
        ..SHARP
    });
    assert!(
        darkest < 30 && brightest > 150,
        "with no smaller copies the squares flicker: {darkest} to {brightest}"
    );

    let (darkest, brightest, middle) = spread(Sampler::default());
    assert!(
        brightest - darkest <= 12,
        "smaller copies should blend the squares: {darkest} to {brightest}"
    );
    // Half the squares are white, so half the light: the copies are made
    // from light, not from sRGB numbers, which would come out darker.
    let plain = floor(Material {
        base_color: [0.5, 0.5, 0.5, 1.0],
        ..Material::default()
    });
    let plain = pixel(&draw(&gpu, &plain, &above(), 0.0), MIDDLE.0, MIDDLE.1);
    assert!(near(middle, plain, 6), "{middle:?} is not {plain:?}");

    finish(gpu);
}

#[test]
fn a_normal_map_bends_the_surface_towards_or_away_from_the_sun() {
    let Some(gpu) = gpu() else {
        return;
    };
    // The usual sun is up, to the right (+X, the floor's u) and towards +Z.
    // The floor is grey, so that more light has somewhere to show.
    let grey = || Material {
        base_color: [0.4, 0.4, 0.4, 1.0],
        ..Material::default()
    };
    let bent = |normal: [u8; 4], scale: f32, mirrored: bool| {
        let mut scene = floor(grey());
        scene.materials[0].normal_texture = Some(texture(&mut scene, 1, &[normal], SHARP));
        scene.materials[0].normal_scale = scale;
        if mirrored {
            scene.instances[0].transform = placed(0.0, 0.0, 0.0, -1.0);
            scene.instances[0].mirrored = true;
        }
        light(pixel(
            &draw(&gpu, &scene, &above(), 0.0),
            MIDDLE.0,
            MIDDLE.1,
        ))
    };
    let flat = bent([128, 128, 255, 255], 1.0, false);
    let unmapped = light(pixel(
        &draw(&gpu, &floor(grey()), &above(), 0.0),
        MIDDLE.0,
        MIDDLE.1,
    ));
    assert!(flat.abs_diff(unmapped) <= 6, "a flat map changes nothing");

    let towards = bent([204, 128, 230, 255], 1.0, false);
    let away = bent([51, 128, 230, 255], 1.0, false);
    assert!(
        towards > flat + 6 && flat > away + 60,
        "leaning to the sun {towards}, flat {flat}, leaning away {away}"
    );
    // No strength, no bend.
    assert!(bent([51, 128, 230, 255], 0.0, false).abs_diff(flat) <= 6);

    // A mirrored floor has its texture mirrored with it, so what leaned
    // right leans left; what leans along the texture's v, which the
    // mirroring left alone, still leans the same way.
    let mirrored = bent([204, 128, 230, 255], 1.0, true);
    assert!(mirrored.abs_diff(away) <= 6, "{mirrored} is not {away}");
    let along = bent([128, 204, 230, 255], 1.0, false);
    let along_mirrored = bent([128, 204, 230, 255], 1.0, true);
    assert!(
        along + 60 < flat,
        "leaning away along v: {along}, flat {flat}"
    );
    assert!(
        along.abs_diff(along_mirrored) <= 6,
        "{along} is not {along_mirrored}"
    );

    finish(gpu);
}

#[test]
fn smooth_metal_mirrors_the_sun_and_rough_metal_does_not() {
    let Some(gpu) = gpu() else {
        return;
    };
    // A wide grey metal floor under a sun that shines down a slope.
    let metal = |roughness: f32| {
        let mut scene = floor(Material {
            base_color: [0.5, 0.5, 0.5, 1.0],
            metallic: 1.0,
            roughness,
            ..Material::default()
        });
        scene.instances[0].transform[0][0] = 20.0;
        scene.instances[0].transform[2][2] = 20.0;
        scene.lights.push(sun([0.0, -0.8, -0.6], [1.0; 3]));
        scene
    };
    // The eye looks back up the mirrored slope, so the middle of the
    // picture is where the sun would be seen in a mirror.
    let shot = |scene: &Scene| {
        let eye = Camera {
            position: [0.0, 1.6, -1.2],
            pitch: -(0.8_f32.asin()),
            yaw: PI,
            ..Camera::default()
        };
        let pixels = draw(&gpu, scene, &eye, 0.0);
        (
            light(pixel(&pixels, MIDDLE.0, MIDDLE.1)),
            light(pixel(&pixels, 4, SIZE - 4)),
        )
    };
    let (mirror, beside) = shot(&metal(0.25));
    let (dull, _) = shot(&metal(1.0));
    assert!(
        mirror > dull + 150 && mirror > beside + 150,
        "smooth {mirror}, beside the highlight {beside}, rough {dull}"
    );

    // The texture's green multiplies the roughness and its blue the
    // metallic: a quarter of fully rough is the smooth metal again.
    let mut scene = metal(1.0);
    let smooth = texture(&mut scene, 1, &[[0, 64, 255, 255]], SHARP);
    scene.materials[0].metallic_roughness_texture = Some(smooth);
    let (from_texture, _) = shot(&scene);
    assert!(
        from_texture.abs_diff(mirror) <= 12,
        "a quarter rough from the texture {from_texture}, from the material {mirror}"
    );
    // Not metal at all, the floor is matte: no highlight to speak of.
    let mut scene = metal(1.0);
    let matte = texture(&mut scene, 1, &[[0, 255, 0, 255]], SHARP);
    scene.materials[0].metallic_roughness_texture = Some(matte);
    let (from_matte, _) = shot(&scene);
    assert!(
        from_texture > from_matte + 100,
        "{from_texture}, {from_matte}"
    );

    finish(gpu);
}

#[test]
fn a_surface_glows_by_itself_and_is_dimmed_in_its_crevices() {
    let Some(gpu) = gpu() else {
        return;
    };
    // A black floor gives back no light, so all that shows is its glow.
    let mut scene = floor(Material {
        base_color: [0.0, 0.0, 0.0, 1.0],
        roughness: 1.0,
        emissive: [0.0, 0.5, 0.0],
        ..Material::default()
    });
    let pixels = draw(&gpu, &scene, &above(), 0.0);
    let [red, green, blue] = pixel(&pixels, MIDDLE.0, MIDDLE.1);
    assert!(green > 150 && red < 50 && blue < 50, "{red},{green},{blue}");
    scene.materials[0].emissive_strength = 0.25;
    let dimmer = pixel(&draw(&gpu, &scene, &above(), 0.0), MIDDLE.0, MIDDLE.1);
    assert!(dimmer[1] + 40 < green, "a quarter of the glow: {dimmer:?}");
    // The glow's texture: none on the left, all of it on the right.
    scene.materials[0].emissive_strength = 1.0;
    scene.materials[0].emissive_texture =
        Some(texture(&mut scene, 2, &[[0, 0, 0, 255], [255; 4]], SHARP));
    let pixels = draw(&gpu, &scene, &above(), 0.0);
    assert!(pixel(&pixels, LEFT.0, LEFT.1)[1] < 40);
    assert!(pixel(&pixels, RIGHT.0, RIGHT.1)[1].abs_diff(green) <= 3);

    // Baked shadowing dims the light from the sky, and not the sun's. With
    // the sun under the floor the sky is all there is.
    let mut scene = floor(Material::default());
    scene.lights.push(sun([0.0, 1.0, 0.0], [1.0; 3]));
    scene.materials[0].occlusion_texture = Some(texture(
        &mut scene,
        2,
        &[[0, 255, 255, 255], [255, 0, 0, 255]],
        SHARP,
    ));
    let pixels = draw(&gpu, &scene, &above(), 0.0);
    let (shadowed, open) = (
        light(pixel(&pixels, LEFT.0, LEFT.1)),
        light(pixel(&pixels, RIGHT.0, RIGHT.1)),
    );
    assert!(
        shadowed < 12 && open > 250,
        "shadowed {shadowed}, open {open}"
    );
    scene.materials[0].occlusion_strength = 0.5;
    let half = light(pixel(&draw(&gpu, &scene, &above(), 0.0), LEFT.0, LEFT.1));
    assert!(half > shadowed + 60 && half + 60 < open, "{half}");
    // In the sun the shadowed half is lit all the same.
    scene.lights.clear();
    scene.materials[0].occlusion_strength = 1.0;
    let sunlit = light(pixel(&draw(&gpu, &scene, &above(), 0.0), LEFT.0, LEFT.1));
    assert!(sunlit > 300, "{sunlit}");

    finish(gpu);
}

#[test]
fn alpha_cuts_holes_or_lets_what_is_behind_show_through() {
    let Some(gpu) = gpu() else {
        return;
    };
    // See-through on the left, solid on the right.
    let mut scene = floor(Material::default());
    let halves = texture(&mut scene, 2, &[[255, 255, 255, 0], [255; 4]], SHARP);
    scene.materials[0].base_color_texture = Some(halves);
    let sides = |scene: &Scene| {
        let pixels = draw(&gpu, scene, &above(), 0.0);
        (
            is_background(pixel(&pixels, LEFT.0, LEFT.1)),
            is_background(pixel(&pixels, RIGHT.0, RIGHT.1)),
        )
    };
    assert_eq!(
        sides(&scene),
        (false, false),
        "a solid material ignores alpha"
    );
    scene.materials[0].alpha_mode = AlphaMode::Mask { cutoff: 0.5 };
    assert_eq!(
        sides(&scene),
        (true, false),
        "a cut-out has a hole on the left"
    );
    scene.materials[0].alpha_mode = AlphaMode::Blend;
    assert_eq!(
        sides(&scene),
        (true, false),
        "nothing shows of what has no alpha"
    );

    // A half see-through blue pane in front of a red wall, listed first:
    // the wall shows through it, and is not hidden by it.
    let pane = [0.1, 0.1, 1.0, 0.5];
    let wall = [1.0, 0.1, 0.1, 1.0];
    let see_through = |list: &[([f32; 4], f32)]| {
        let mut scene = squares(
            &list
                .iter()
                .map(|&(colour, z)| (colour, placed(0.0, 0.0, z, 1.0), false))
                .collect::<Vec<_>>(),
        );
        for material in &mut scene.materials {
            if material.base_color[3] < 1.0 {
                material.alpha_mode = AlphaMode::Blend;
            }
        }
        pixel(
            &draw(&gpu, &scene, &Camera::default(), 0.0),
            MIDDLE.0,
            MIDDLE.1,
        )
    };
    let [wall_red, _, wall_blue] = see_through(&[(wall, -3.0)]);
    let [red, _, blue] = see_through(&[(pane, -2.0), (wall, -3.0)]);
    assert!(
        red > 60 && red + 30 < wall_red && blue > wall_blue + 60,
        "the wall alone {wall_red},{wall_blue}; through the pane {red},{blue}"
    );
    // A wall in front of the pane hides it.
    assert_eq!(
        see_through(&[(pane, -3.0), (wall, -2.0)]),
        see_through(&[(wall, -2.0)])
    );

    // Two panes are mixed farthest first, whichever is listed first.
    let green = [0.1, 1.0, 0.1, 0.5];
    let one = see_through(&[(pane, -2.0), (green, -3.0)]);
    let other = see_through(&[(green, -3.0), (pane, -2.0)]);
    assert_eq!(one, other);
    let swapped = see_through(&[(pane, -3.0), (green, -2.0)]);
    assert!(
        one[2] > one[1] && swapped[1] > swapped[2],
        "the nearer pane shows most: {one:?}, {swapped:?}"
    );

    finish(gpu);
}

#[test]
fn a_level_is_lit_by_its_own_sun_and_a_back_face_by_what_it_faces() {
    let Some(gpu) = gpu() else {
        return;
    };
    let middle = |scene: &Scene, camera: &Camera| {
        pixel(&draw(&gpu, scene, camera, -1.0), MIDDLE.0, MIDDLE.1)
    };
    // The usual light shows a white floor white: no tint.
    let mut scene = floor(Material::default());
    let [red, green, blue] = middle(&scene, &above());
    assert!(red.abs_diff(green) <= 2 && green.abs_diff(blue) <= 2);

    // An orange sun overhead tints it, under the same blue sky.
    scene.lights.push(sun([0.0, -1.0, 0.0], [1.0, 0.5, 0.1]));
    let [red, green, blue] = middle(&scene, &above());
    assert!(red > green + 20 && green > blue + 5, "{red},{green},{blue}");
    // Shining along the floor it adds nothing, and the sky's blue is left.
    scene.lights[0] = sun([1.0, 0.0, 0.0], [1.0, 0.5, 0.1]);
    let [low_red, _, low_blue] = middle(&scene, &above());
    assert!(
        low_blue > low_red + 5 && low_red + 40 < red,
        "{low_red},{low_blue}"
    );

    // From underneath a floor with one side is not there; with two, its
    // underside faces the ground and is lit by that, not by the sun above.
    scene.lights.clear();
    let below = Camera {
        position: [0.0, -1.0, 0.0],
        pitch: FRAC_PI_2,
        ..Camera::default()
    };
    assert!(is_background(middle(&scene, &below)));
    scene.materials[0].double_sided = true;
    let (top, underside) = (middle(&scene, &above()), middle(&scene, &below));
    assert!(!is_background(underside));
    assert!(
        light(underside) + 150 < light(top),
        "underside {underside:?}, top {top:?}"
    );

    finish(gpu);
}

#[test]
fn a_picture_that_cannot_be_used_is_left_out_and_the_rest_is_drawn() {
    let Some(gpu) = gpu() else {
        return;
    };
    let mut scene = floor(Material {
        base_color: [1.0, 0.0, 0.0, 1.0],
        ..Material::default()
    });
    // Too few bytes for its size, and a texture of a picture that is not there.
    let mut broken = texture(&mut scene, 4, &[[0, 255, 0, 255]], SHARP);
    scene.images[0].height = 4;
    scene.materials[0].base_color_texture = Some(broken);
    let [red, green, _] = pixel(&draw(&gpu, &scene, &above(), 0.0), MIDDLE.0, MIDDLE.1);
    assert!(
        red > 150 && green < 50,
        "the material's own red: {red},{green}"
    );
    broken.texture = 7;
    scene.materials[0].normal_texture = Some(broken);
    scene.textures[0].image = 9;
    let [red, green, _] = pixel(&draw(&gpu, &scene, &above(), 0.0), MIDDLE.0, MIDDLE.1);
    assert!(red > 150 && green < 50);
    // A part that names a material the scene has not got is matte white.
    scene.meshes[0].parts[0].material = 5;
    let [red, green, blue] = pixel(&draw(&gpu, &scene, &above(), 0.0), MIDDLE.0, MIDDLE.1);
    assert!(red > 200 && green > 200 && blue > 200);

    finish(gpu);
}
