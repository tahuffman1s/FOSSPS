// Draws a mesh in its material's colour, lit by a plain sky so that shapes
// can be told apart, and writes how far each pixel moved since the last
// frame. Textures, materials and real lights come later.

// The same for every mesh in a frame.
struct View {
    // Takes a point in the world to where it is drawn this frame, including
    // the part-of-a-pixel shift the upscaler asks for.
    clip_from_world: mat4x4<f32>,
    // The same without the shift, and the same for the last frame: the
    // difference between the two is how far a point moved.
    steady_from_world: mat4x4<f32>,
    before_from_world: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> view: View;

struct Draw {
    // Takes a corner point of the mesh to where it stands in the world.
    world_from_local: mat4x4<f32>,
    // Turns the mesh's surface directions the way the mesh is turned.
    normal_from_local: mat3x3<f32>,
    // The material's base colour.
    colour: vec4<f32>,
}

var<immediate> draw: Draw;

struct Corner {
    @builtin(position) position: vec4<f32>,
    @location(0) normal: vec3<f32>,
    @location(1) colour: vec4<f32>,
    // Where the point is this frame and was last frame, before dividing by w.
    @location(2) steady: vec4<f32>,
    @location(3) before: vec4<f32>,
}

@vertex
fn vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec4<f32>,
) -> Corner {
    // Nothing in a level moves yet, so a point was where it is now.
    let world = draw.world_from_local * vec4<f32>(position, 1.0);
    var corner: Corner;
    corner.position = view.clip_from_world * world;
    corner.steady = view.steady_from_world * world;
    corner.before = view.before_from_world * world;
    corner.normal = draw.normal_from_local * normal;
    corner.colour = colour * draw.colour;
    return corner;
}

struct Pixel {
    @location(0) colour: vec4<f32>,
    // Where the pixel was last frame minus where it is now, as a fraction of
    // the picture: x to the right, y down.
    @location(1) motion: vec2<f32>,
}

@fragment
fn fragment(corner: Corner, @builtin(front_facing) front: bool) -> Pixel {
    var normal = normalize(corner.normal);
    // The back of a two-sided surface faces the other way.
    if !front {
        normal = -normal;
    }
    // Full colour for a surface facing up, about two thirds for a wall and a
    // third for one facing down, with walls that face +X a little brighter
    // than those that face -X so a box's sides differ.
    let light = 0.65 + 0.35 * normal.y + 0.1 * normal.x;

    var pixel: Pixel;
    pixel.colour = vec4<f32>(corner.colour.rgb * light, 1.0);
    // A point that was behind the eye last frame was nowhere on the picture.
    pixel.motion = vec2<f32>(0.0);
    if corner.before.w > 0.0 && corner.steady.w > 0.0 {
        let moved = corner.before.xy / corner.before.w - corner.steady.xy / corner.steady.w;
        // The picture is 2 wide and 2 high here, with y running up.
        pixel.motion = moved * vec2<f32>(0.5, -0.5);
    }
    return pixel;
}
