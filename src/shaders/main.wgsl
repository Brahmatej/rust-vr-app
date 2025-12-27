// VR Shader with camera transform support
// Supports stereoscopic rendering with view/projection matrices

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    eye_offset: vec4<f32>,  // x = left/right offset, y = is_vr_mode
};

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Simple triangle in world space
    var positions = array<vec3<f32>, 3>(
        vec3<f32>(0.0, 0.5, -2.0),   // Top
        vec3<f32>(-0.5, -0.5, -2.0), // Bottom left
        vec3<f32>(0.5, -0.5, -2.0),  // Bottom right
    );
    
    var colors = array<vec3<f32>, 3>(
        vec3<f32>(1.0, 0.2, 0.5),  // Pink
        vec3<f32>(0.2, 0.8, 1.0),  // Cyan
        vec3<f32>(0.8, 0.3, 1.0),  // Purple
    );
    
    // Apply eye offset for stereo rendering
    var world_pos = positions[vertex_index];
    world_pos.x += camera.eye_offset.x;
    
    var output: VertexOutput;
    output.position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    output.color = colors[vertex_index];
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}
