// The last pass: turns the scene's high-dynamic-range colours into ones a
// display can show.
//
// The curve is the Khronos PBR Neutral tone mapper, written from its
// published description (https://github.com/KhronosGroup/ToneMapping). Colours
// below 0.76 keep their hue and saturation; only brighter ones are squeezed
// towards white, so team colours stay the colours the map maker picked.

struct Settings {
    // What the scene's light is multiplied by before the curve.
    exposure: f32,
    // 1 when the image being drawn into stores numbers as given, so the sRGB
    // curve has to be applied here. 0 when the image applies it itself.
    encode: u32,
}

var<immediate> settings: Settings;

@group(0) @binding(0) var scene: texture_2d<f32>;

// One triangle that covers the whole picture.
@vertex
fn vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

// Brightness at which highlights start to be squeezed.
const START: f32 = 0.76;
// How quickly squeezed highlights lose their colour.
const DESATURATION: f32 = 0.15;

fn pbr_neutral(light: vec3<f32>) -> vec3<f32> {
    // Take off the 4% every surface reflects, so that black stays black.
    let lowest = min(light.r, min(light.g, light.b));
    var offset = 0.04;
    if lowest < 0.08 {
        offset = lowest - 6.25 * lowest * lowest;
    }
    var colour = light - offset;

    let peak = max(colour.r, max(colour.g, colour.b));
    if peak < START {
        return colour;
    }
    let room = 1.0 - START;
    let squeezed = 1.0 - room * room / (peak + room - START);
    colour *= squeezed / peak;
    let fade = 1.0 - 1.0 / (DESATURATION * (peak - squeezed) + 1.0);
    return mix(colour, vec3<f32>(squeezed), fade);
}

// The sRGB curve, for images that do not apply it themselves.
fn srgb(linear: vec3<f32>) -> vec3<f32> {
    let low = linear * 12.92;
    let high = 1.055 * pow(linear, vec3<f32>(1.0 / 2.4)) - 0.055;
    return select(high, low, linear <= vec3<f32>(0.0031308));
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let light = textureLoad(scene, vec2<i32>(position.xy), 0).rgb;
    // Negative or not-a-number light would spread through the curve.
    let mapped = clamp(pbr_neutral(max(light, vec3<f32>(0.0)) * settings.exposure), vec3<f32>(0.0), vec3<f32>(1.0));
    if settings.encode == 1u {
        return vec4<f32>(srgb(mapped), 1.0);
    }
    return vec4<f32>(mapped, 1.0);
}
