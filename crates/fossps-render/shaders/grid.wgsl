// Draws the editor's ground grid: a line every metre and a stronger one
// every ten, on the plane at height 0, with the X axis red and the Z axis
// blue. It is one large square that follows the eye and fades out before
// its edge, so it looks endless.
//
// The lines are drawn as Ben Golus's "pristine grid" does: a line is never
// drawn thinner than a pixel, and is made fainter instead, so far-off lines
// neither flicker nor turn into a moiré pattern.

// The same block the mesh shader reads.
struct View {
    clip_from_world: mat4x4<f32>,
    steady_from_world: mat4x4<f32>,
    before_from_world: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;

struct Grid {
    // Where the eye is, in metres. The fourth number is not used.
    eye: vec4<f32>,
}

var<immediate> grid: Grid;

// How far the square reaches from the eye, in metres.
const REACH: f32 = 1000.0;
// Lines every metre are gone by this distance, and all of the grid by the next.
const SMALL_FADE: f32 = 120.0;
const FADE: f32 = 600.0;
// The widths of the lines, in metres.
const SMALL_WIDTH: f32 = 0.02;
const LARGE_WIDTH: f32 = 0.08;
const AXIS_WIDTH: f32 = 0.10;
// How many metres lie between two of the stronger lines.
const LARGE: f32 = 10.0;
// The grid is drawn this much nearer than it is, as a share of its
// distance, so that it wins against a floor lying at the same height
// instead of flickering with it.
const NEARER: f32 = 1.002;

struct Corner {
    @builtin(position) position: vec4<f32>,
    // Where the point is on the ground, in metres: x, then z.
    @location(0) ground: vec2<f32>,
    // Where the point is this frame and was last frame, before dividing by w.
    @location(1) steady: vec4<f32>,
    @location(2) before: vec4<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Corner {
    // Two triangles that make a square.
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
    );
    let ground = grid.eye.xz + corners[index] * REACH;
    let world = vec4<f32>(ground.x, 0.0, ground.y, 1.0);
    var corner: Corner;
    corner.position = view.clip_from_world * world;
    corner.position.z *= NEARER;
    corner.ground = ground;
    corner.steady = view.steady_from_world * world;
    corner.before = view.before_from_world * world;
    return corner;
}

// How much of a pixel at `place` is covered by lines `width` wide that lie
// on every whole number, along each of the two directions. `across` is how
// much `place` changes over one pixel.
fn lines(place: vec2<f32>, across: vec2<f32>, width: f32) -> vec2<f32> {
    let drawn = min(max(vec2<f32>(width), across), vec2<f32>(0.5));
    let soft = max(across, vec2<f32>(0.000001)) * 1.5;
    // 0 on a line, 1 half way between two.
    let from_line = 1.0 - abs(fract(place) * 2.0 - 1.0);
    var covered = 1.0 - smoothstep(drawn - soft, drawn + soft, from_line);
    // A line made wider to fill a pixel is made fainter to match.
    covered *= clamp(vec2<f32>(width) / drawn, vec2<f32>(0.0), vec2<f32>(1.0));
    // So far away that lines run together: an even tint of what they cover.
    return mix(covered, vec2<f32>(width), clamp(across * 2.0 - 1.0, vec2<f32>(0.0), vec2<f32>(1.0)));
}

struct Pixel {
    @location(0) colour: vec4<f32>,
    // As the mesh shader writes it, with an opacity of 1 so that it
    // replaces what was there.
    @location(1) motion: vec4<f32>,
}

@fragment
fn fragment(corner: Corner) -> Pixel {
    let ground = corner.ground;
    let slope = vec4<f32>(dpdx(ground), dpdy(ground));
    // How many metres one pixel covers, along x and along z.
    let across = vec2<f32>(length(slope.xz), length(slope.yw));
    let away = distance(grid.eye.xyz, vec3<f32>(ground.x, 0.0, ground.y));

    let small = lines(ground, across, SMALL_WIDTH);
    let large = lines(ground / LARGE, across / LARGE, LARGE_WIDTH / LARGE);
    let small_cover = max(small.x, small.y) * (1.0 - smoothstep(0.4 * SMALL_FADE, SMALL_FADE, away));
    let large_cover = max(large.x, large.y);

    // The two lines through the middle of the world. The X axis is where z
    // is 0, and the other way round.
    // The X axis is as wide as a pixel is in z, and the other way round.
    let axis_across = across.yx;
    let axis_drawn = max(vec2<f32>(AXIS_WIDTH), axis_across);
    let axis = (1.0 - smoothstep(axis_drawn - axis_across, axis_drawn + axis_across, abs(ground.yx) * 2.0))
        * clamp(vec2<f32>(AXIS_WIDTH) / axis_drawn, vec2<f32>(0.0), vec2<f32>(1.0));

    var colour = vec3<f32>(1.0, 1.0, 1.0);
    var cover = max(small_cover * 0.35, large_cover * 0.8);
    if axis.x > 0.0 || axis.y > 0.0 {
        let red = vec3<f32>(0.90, 0.16, 0.14);
        let blue = vec3<f32>(0.16, 0.36, 0.95);
        let axis_colour = select(blue, red, axis.x >= axis.y);
        let axis_cover = max(axis.x, axis.y) * 0.9;
        colour = mix(colour, axis_colour, clamp(axis_cover / max(cover, 0.0001), 0.0, 1.0));
        cover = max(cover, axis_cover);
    }
    cover *= 1.0 - smoothstep(0.5 * FADE, FADE, away);
    if cover < 0.003 {
        discard;
    }

    var pixel: Pixel;
    pixel.colour = vec4<f32>(colour, cover);
    var moved = vec2<f32>(0.0);
    if corner.before.w > 0.0 && corner.steady.w > 0.0 {
        moved = (corner.before.xy / corner.before.w - corner.steady.xy / corner.steady.w)
            * vec2<f32>(0.5, -0.5);
    }
    pixel.motion = vec4<f32>(moved, 0.0, 1.0);
    return pixel;
}
