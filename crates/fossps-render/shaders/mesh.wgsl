// Draws a mesh with its material: textures, how metallic and how rough it
// is, a normal map, baked shadowing and its own glow, lit by one sun and by
// the sky above and the ground below. Also writes how far each pixel moved
// since the last frame.
//
// The light sums are the ones the glTF specification gives for its
// metallic-roughness material (appendix B).

// The same for every mesh in a frame.
struct View {
    // Takes a point in the world to where it is drawn this frame, including
    // the part-of-a-pixel shift the upscaler asks for.
    clip_from_world: mat4x4<f32>,
    // The same without the shift, and the same for the last frame: the
    // difference between the two is how far a point moved.
    steady_from_world: mat4x4<f32>,
    before_from_world: mat4x4<f32>,
    // Where the eye is, in metres. The fourth number is how many sizes
    // sharper than usual textures are read, for when the picture is upscaled.
    eye: vec4<f32>,
    // The direction from any surface to the sun.
    towards_sun: vec4<f32>,
    // The sun's light on a surface that faces it.
    sun: vec4<f32>,
    // The light a matte surface gets from the sky above and the ground below.
    sky: vec4<f32>,
    ground: vec4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;

// How one texture is laid over a mesh.
struct Placing {
    // The texture's u is `u.xy` times the mesh's UV plus `u.z`, and its v the
    // same with `v`. `u.w` is 1 if the mesh's second UV map is the one used.
    u: vec4<f32>,
    v: vec4<f32>,
}

struct Material {
    base_colour: vec4<f32>,
    // The glow, and in the fourth number the alpha below which a cut-out
    // surface is not drawn.
    emissive: vec4<f32>,
    // Metallic, roughness, the normal map's strength and the occlusion's.
    factors: vec4<f32>,
    // In the first number, one bit for each texture the material has, in
    // the order below.
    flags: vec4<u32>,
    placings: array<Placing, 5>,
}

const BASE: u32 = 0u;
const METAL_ROUGH: u32 = 1u;
const NORMAL: u32 = 2u;
const OCCLUSION: u32 = 3u;
const EMISSIVE: u32 = 4u;

@group(1) @binding(0) var<uniform> material: Material;
@group(1) @binding(1) var base_texture: texture_2d<f32>;
@group(1) @binding(2) var metal_rough_texture: texture_2d<f32>;
@group(1) @binding(3) var normal_texture: texture_2d<f32>;
@group(1) @binding(4) var occlusion_texture: texture_2d<f32>;
@group(1) @binding(5) var emissive_texture: texture_2d<f32>;
@group(1) @binding(6) var base_sampler: sampler;
@group(1) @binding(7) var metal_rough_sampler: sampler;
@group(1) @binding(8) var normal_sampler: sampler;
@group(1) @binding(9) var occlusion_sampler: sampler;
@group(1) @binding(10) var emissive_sampler: sampler;

struct Draw {
    // Takes a corner point of the mesh to where it stands in the world.
    world_from_local: mat4x4<f32>,
    // Turns the mesh's surface directions the way the mesh is turned.
    normal_from_local: mat3x3<f32>,
    // -1 if the mesh is placed mirrored, which turns its normal map's
    // sideways direction around, and 1 if not.
    handedness: f32,
}

var<immediate> draw: Draw;

struct Corner {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) colour: vec4<f32>,
    // Where the point is this frame and was last frame, before dividing by w.
    @location(2) steady: vec4<f32>,
    @location(3) before: vec4<f32>,
    // Where the point is in the world.
    @location(4) world: vec3<f32>,
    // The direction the texture's u runs along the surface; the fourth
    // number says which way its v runs.
    @location(5) tangent: vec4<f32>,
    // The first UV map, then the second.
    @location(6) uv: vec4<f32>,
}

// A direction stored as a point on a square (octahedral encoding), unfolded.
fn unfolded(folded: vec2<f32>) -> vec3<f32> {
    var direction = vec3<f32>(folded, 1.0 - abs(folded.x) - abs(folded.y));
    let under = max(-direction.z, 0.0);
    direction.x += select(under, -under, direction.x >= 0.0);
    direction.y += select(under, -under, direction.y >= 0.0);
    return normalize(direction);
}

@vertex
fn vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec2<f32>,
    @location(2) tangent: vec4<f32>,
    @location(3) colour: vec4<f32>,
    @location(4) uv0: vec2<f32>,
    @location(5) uv1: vec2<f32>,
) -> Corner {
    // Nothing in a level moves yet, so a point was where it is now.
    let world = draw.world_from_local * vec4<f32>(position, 1.0);
    var corner: Corner;
    corner.position = view.clip_from_world * world;
    corner.steady = view.steady_from_world * world;
    corner.before = view.before_from_world * world;
    corner.world = world.xyz;
    corner.normal = draw.normal_from_local * unfolded(normal);
    // A direction along the surface turns with the mesh itself.
    corner.tangent = vec4<f32>(
        (draw.world_from_local * vec4<f32>(tangent.xyz, 0.0)).xyz,
        tangent.w * draw.handedness,
    );
    corner.colour = colour;
    corner.uv = vec4<f32>(uv0, uv1);
    return corner;
}

struct Pixel {
    @location(0) colour: vec4<f32>,
    // Where the pixel was last frame minus where it is now, as a fraction of
    // the picture: x to the right, y down.
    @location(1) motion: vec2<f32>,
}

fn has(texture: u32) -> bool {
    return (material.flags.x & (1u << texture)) != 0u;
}

// Where on a texture this pixel is.
fn placed(texture: u32, corner: Corner) -> vec2<f32> {
    let placing = material.placings[texture];
    let uv = select(corner.uv.xy, corner.uv.zw, placing.u.w > 0.5);
    return vec2<f32>(
        dot(placing.u.xy, uv) + placing.u.z,
        dot(placing.v.xy, uv) + placing.v.z,
    );
}

// The surface's colour and alpha before any light.
fn base_colour(corner: Corner) -> vec4<f32> {
    var base = material.base_colour * corner.colour;
    if has(BASE) {
        base *= textureSampleBias(base_texture, base_sampler, placed(BASE, corner), view.eye.w);
    }
    return base;
}

const PI: f32 = 3.14159265;
// Nothing is a perfect mirror: below this a highlight would be a single
// flickering pixel.
const SMOOTHEST: f32 = 0.045;

// The light from above and below that reaches a surface facing `direction`.
fn surroundings(direction: vec3<f32>) -> vec3<f32> {
    return mix(view.ground.rgb, view.sky.rgb, direction.y * 0.5 + 0.5);
}

// How much of the light from all around is mirrored towards the eye, worked
// out without a table (Karis, "Physically Based Shading on Mobile", 2014).
fn mirrored_share(mirror: vec3<f32>, roughness: f32, facing: f32) -> vec3<f32> {
    let c0 = vec4<f32>(-1.0, -0.0275, -0.572, 0.022);
    let c1 = vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let r = roughness * c0 + c1;
    let a004 = min(r.x * r.x, exp2(-9.28 * facing)) * r.x + r.y;
    let scale_bias = vec2<f32>(-1.04, 1.04) * a004 + r.zw;
    return mirror * scale_bias.x + scale_bias.y;
}

// The light the surface sends to the eye.
fn lit(corner: Corner, front: bool, base: vec3<f32>) -> vec3<f32> {
    var normal = normalize(corner.normal);
    // The back of a two-sided surface faces the other way.
    let side = select(-1.0, 1.0, front);
    normal *= side;
    if has(NORMAL) {
        let sample = textureSampleBias(
            normal_texture, normal_sampler, placed(NORMAL, corner), view.eye.w,
        ).xyz * 2.0 - 1.0;
        // The tangent is kept at right angles to the smoothed normal.
        let leaning = corner.tangent.xyz - normal * dot(normal, corner.tangent.xyz);
        // A mesh with no tangents has nothing to bend its surface along.
        if dot(leaning, leaning) > 1e-12 {
            let along = normalize(leaning) * side;
            let across = cross(normal, along) * corner.tangent.w * side;
            let bend = sample.xy * material.factors.z;
            normal = normalize(along * bend.x + across * bend.y + normal * sample.z);
        }
    }

    var metallic = material.factors.x;
    var roughness = material.factors.y;
    if has(METAL_ROUGH) {
        let sample = textureSampleBias(
            metal_rough_texture, metal_rough_sampler, placed(METAL_ROUGH, corner), view.eye.w,
        );
        roughness *= sample.g;
        metallic *= sample.b;
    }
    metallic = clamp(metallic, 0.0, 1.0);
    roughness = clamp(roughness, SMOOTHEST, 1.0);

    // Metal mirrors light in its own colour and has no matte colour;
    // anything else mirrors a little white light.
    let matte = base * (1.0 - metallic);
    let mirror = mix(vec3<f32>(0.04), base, metallic);

    let to_eye = normalize(view.eye.xyz - corner.world);
    let to_sun = view.towards_sun.xyz;
    let halfway = normalize(to_eye + to_sun);
    let n_eye = max(dot(normal, to_eye), 1e-4);
    let n_sun = clamp(dot(normal, to_sun), 0.0, 1.0);
    let n_half = clamp(dot(normal, halfway), 0.0, 1.0);
    let eye_half = clamp(dot(to_eye, halfway), 0.0, 1.0);

    // The sun: a matte part and a highlight (GGX, with Smith's
    // height-correlated visibility and Schlick's Fresnel).
    let alpha = roughness * roughness;
    let alpha2 = alpha * alpha;
    let spread = n_half * n_half * (alpha2 - 1.0) + 1.0;
    let density = alpha2 / (PI * spread * spread);
    let visible = 0.5 / max(
        n_sun * sqrt(n_eye * n_eye * (1.0 - alpha2) + alpha2)
            + n_eye * sqrt(n_sun * n_sun * (1.0 - alpha2) + alpha2),
        1e-5,
    );
    let fresnel = mirror + (1.0 - mirror) * pow(1.0 - eye_half, 5.0);
    let from_sun = ((1.0 - fresnel) * matte / PI + fresnel * density * visible)
        * view.sun.rgb * n_sun;

    // The sky and the ground, dimmed in crevices by the baked shadowing.
    var from_around = matte * surroundings(normal)
        + surroundings(reflect(-to_eye, normal)) * mirrored_share(mirror, roughness, n_eye);
    if has(OCCLUSION) {
        let shadowing = textureSampleBias(
            occlusion_texture, occlusion_sampler, placed(OCCLUSION, corner), view.eye.w,
        ).r;
        from_around *= 1.0 + material.factors.w * (shadowing - 1.0);
    }

    var glow = material.emissive.rgb;
    if has(EMISSIVE) {
        glow *= textureSampleBias(
            emissive_texture, emissive_sampler, placed(EMISSIVE, corner), view.eye.w,
        ).rgb;
    }
    return from_sun + from_around + glow;
}

// How far the pixel moved since the last frame.
fn motion(corner: Corner) -> vec2<f32> {
    // A point that was behind the eye last frame was nowhere on the picture.
    if corner.before.w > 0.0 && corner.steady.w > 0.0 {
        let moved = corner.before.xy / corner.before.w - corner.steady.xy / corner.steady.w;
        // The picture is 2 wide and 2 high here, with y running up.
        return moved * vec2<f32>(0.5, -0.5);
    }
    return vec2<f32>(0.0);
}

// A solid surface.
@fragment
fn fragment(corner: Corner, @builtin(front_facing) front: bool) -> Pixel {
    let base = base_colour(corner);
    var pixel: Pixel;
    pixel.colour = vec4<f32>(lit(corner, front, base.rgb), 1.0);
    pixel.motion = motion(corner);
    return pixel;
}

// A surface with holes cut in it by its alpha. Kept apart from the solid
// one, which can then be tested against the depth before it is shaded.
@fragment
fn fragment_cutout(corner: Corner, @builtin(front_facing) front: bool) -> Pixel {
    let base = base_colour(corner);
    var pixel: Pixel;
    pixel.colour = vec4<f32>(lit(corner, front, base.rgb), 1.0);
    pixel.motion = motion(corner);
    // After every texture has been read: a pixel that is thrown away can
    // no longer tell its neighbours how fast its textures are changing,
    // and they would pick the wrong size of texture along a hole's edge.
    if base.a < material.emissive.w {
        discard;
    }
    return pixel;
}

struct Mixed {
    @location(0) colour: vec4<f32>,
    @location(1) motion: vec4<f32>,
}

// A see-through surface, mixed over what is behind it by its alpha. Both
// images are mixed the same way (not every graphics card can mix them
// differently), so the movement is written with no alpha at all, which
// leaves what was there: the pixel moves with what is behind.
@fragment
fn fragment_blend(corner: Corner, @builtin(front_facing) front: bool) -> Mixed {
    let base = base_colour(corner);
    var pixel: Mixed;
    pixel.colour = vec4<f32>(lit(corner, front, base.rgb), base.a);
    pixel.motion = vec4<f32>(0.0);
    return pixel;
}
