// One coloured triangle, with no vertex buffer: the corners come from the vertex number.

struct Corner {
    @builtin(position) position: vec4<f32>,
    @location(0) colour: vec3<f32>,
}

@vertex
fn vertex(@builtin(vertex_index) index: u32) -> Corner {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.6),
        vec2<f32>(0.6, -0.6),
        vec2<f32>(-0.6, -0.6),
    );
    var colours = array<vec3<f32>, 3>(
        vec3<f32>(1.0, 0.25, 0.2),
        vec3<f32>(0.25, 1.0, 0.3),
        vec3<f32>(0.25, 0.4, 1.0),
    );
    var corner: Corner;
    corner.position = vec4<f32>(positions[index], 0.0, 1.0);
    corner.colour = colours[index];
    return corner;
}

@fragment
fn fragment(corner: Corner) -> @location(0) vec4<f32> {
    return vec4<f32>(corner.colour, 1.0);
}
