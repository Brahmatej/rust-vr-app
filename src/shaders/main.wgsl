// Basic WGSL shader for VR environment
// Will be expanded to include glassmorphism effects and 3D panels

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec3<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Simple triangle for initial test
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(0.0, 0.5),
        vec2<f32>(-0.5, -0.5),
        vec2<f32>(0.5, -0.5),
    );
    
    var colors = array<vec3<f32>, 3>(
        vec3<f32>(1.0, 0.2, 0.5),  // Pink
        vec3<f32>(0.2, 0.8, 1.0),  // Cyan
        vec3<f32>(0.8, 0.3, 1.0),  // Purple
    );
    
    var output: VertexOutput;
    output.position = vec4<f32>(positions[vertex_index], 0.0, 1.0);
    output.color = colors[vertex_index];
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);
}
