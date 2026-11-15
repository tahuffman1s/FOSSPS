// Draws a mesh in its material's colour, lit by a plain sky so that shapes
// can be told apart. Textures, materials and real lights come later.

struct Draw {
    // Takes a corner point of the mesh to where it is drawn.
    clip_from_local: mat4x4<f32>,
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
}

@vertex
fn vertex(
    @location(0) position: vec3<f32>,
    @location(1) normal: vec3<f32>,
    @location(2) colour: vec4<f32>,
) -> Corner {
    var corner: Corner;
    corner.position = draw.clip_from_local * vec4<f32>(position, 1.0);
    corner.normal = draw.normal_from_local * normal;
    corner.colour = colour * draw.colour;
    return corner;
}

@fragment
fn fragment(corner: Corner, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    var normal = normalize(corner.normal);
    // The back of a two-sided surface faces the other way.
    if !front {
        normal = -normal;
    }
    // Full colour for a surface facing up, about two thirds for a wall and a
    // third for one facing down, with walls that face +X a little brighter
    // than those that face -X so a box's sides differ.
    let light = 0.65 + 0.35 * normal.y + 0.1 * normal.x;
    return vec4<f32>(corner.colour.rgb * light, 1.0);
}
