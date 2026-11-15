// Draws the HUD and menus over the finished picture: filled boxes with
// round corners, and letters from one picture that holds them all.
//
// Every box and every letter is one rectangle of four corners. Positions
// and sizes are in pixels of the picture, from its top-left corner.

struct Screen {
    // The size of the picture being drawn into, in pixels.
    size: vec2<f32>,
    // 1 when the image being drawn into applies the sRGB curve itself, so
    // colours have to be handed over without it. 0 when it stores numbers
    // as given.
    decode: u32,
}

var<immediate> screen: Screen;

// How much of each pixel a letter covers; every letter at every size in use.
@group(0) @binding(0) var atlas: texture_2d<f32>;
@group(0) @binding(1) var smooth_sampler: sampler;

struct Corner {
    @builtin(position) position: vec4<f32>,
    // From the middle of the rectangle to this pixel.
    @location(0) offset: vec2<f32>,
    // Where in the atlas to read, in its pixels.
    @location(1) atlas_at: vec2<f32>,
    @location(2) @interpolate(flat) half_size: vec2<f32>,
    @location(3) @interpolate(flat) colour: vec4<f32>,
    // How round the corners are, then 1 for a letter and 0 for a box.
    @location(4) @interpolate(flat) shape: vec2<f32>,
}

@vertex
fn vertex(
    @builtin(vertex_index) index: u32,
    // Left, top, width and height on the screen.
    @location(0) rect: vec4<f32>,
    // Left, top, width and height in the atlas; unused for a box.
    @location(1) source: vec4<f32>,
    @location(2) colour: vec4<f32>,
    @location(3) shape: vec2<f32>,
) -> Corner {
    // 0,0 then 1,0 then 0,1 then 1,1: a strip of two triangles.
    let unit = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let outward = unit * 2.0 - 1.0;
    // A box is drawn one pixel larger all round, so that the pixels its
    // edge only partly covers are drawn too, and come out soft.
    let grow = outward * (1.0 - shape.y);
    let pixel = rect.xy + unit * rect.zw + grow;

    var corner: Corner;
    // Pixels count down from the top; the shader's own space runs from -1
    // at the bottom to 1 at the top.
    let across = pixel / screen.size;
    corner.position = vec4<f32>(across.x * 2.0 - 1.0, 1.0 - across.y * 2.0, 0.0, 1.0);
    corner.offset = outward * rect.zw * 0.5 + grow;
    corner.atlas_at = source.xy + unit * source.zw;
    corner.half_size = rect.zw * 0.5;
    corner.colour = colour;
    corner.shape = shape;
    return corner;
}

// How far `point` is outside a box with round corners, in pixels; negative
// inside. Inigo Quilez's rounded-box distance (iquilezles.org, "2D distance
// functions").
fn outside_round_box(point: vec2<f32>, half_size: vec2<f32>, radius: f32) -> f32 {
    let from_corner = abs(point) - half_size + radius;
    return length(max(from_corner, vec2<f32>(0.0))) + min(max(from_corner.x, from_corner.y), 0.0) - radius;
}

// Undoes the sRGB curve, for images that apply it themselves.
fn linear(srgb: vec3<f32>) -> vec3<f32> {
    let low = srgb / 12.92;
    let high = pow((srgb + 0.055) / 1.055, vec3<f32>(2.4));
    return select(high, low, srgb <= vec3<f32>(0.04045));
}

@fragment
fn fragment(corner: Corner) -> @location(0) vec4<f32> {
    var covered: f32;
    if corner.shape.y > 0.5 {
        let size = vec2<f32>(textureDimensions(atlas));
        covered = textureSampleLevel(atlas, smooth_sampler, corner.atlas_at / size, 0.0).r;
    } else {
        // Half a pixel inside the edge is fully covered, half outside not at all.
        let outside = outside_round_box(corner.offset, corner.half_size, corner.shape.x);
        covered = clamp(0.5 - outside, 0.0, 1.0);
    }
    var colour = corner.colour.rgb;
    if screen.decode == 1u {
        colour = linear(colour);
    }
    return vec4<f32>(colour, corner.colour.a * covered);
}
