// VR Shader with camera transform and video texture support
// Supports stereoscopic rendering with view/projection matrices

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    eye_offset: vec4<f32>,  // x = offset, y = has_video, z = time, w = content_scale
};

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// Video texture (optional - bound only when video is playing)
@group(1) @binding(0)
var video_texture: texture_2d<f32>;
@group(1) @binding(1)
var video_sampler: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // 16:9 Cinema Screen Quad at Z = -2.0
    // Width = 3.2m, Height = 1.8m -> Large screen
    
    var positions = array<vec3<f32>, 6>(
        vec3<f32>(-1.6,  0.9, -2.0), // TL
        vec3<f32>(-1.6, -0.9, -2.0), // BL
        vec3<f32>( 1.6,  0.9, -2.0), // TR
        vec3<f32>( 1.6,  0.9, -2.0), // TR
        vec3<f32>(-1.6, -0.9, -2.0), // BL
        vec3<f32>( 1.6, -0.9, -2.0), // BR
    );
    
    // UVs standard
    var uvs = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), // TL
        vec2<f32>(0.0, 1.0), // BL
        vec2<f32>(1.0, 0.0), // TR
        vec2<f32>(1.0, 0.0), // TR
        vec2<f32>(0.0, 1.0), // BL
        vec2<f32>(1.0, 1.0), // BR
    );
    
    var world_pos = positions[vertex_index];
    
    // Apply Content Scale (Zoom)
    let scale = camera.eye_offset.w;
    if (scale > 0.0) {
        world_pos.x *= scale;
        world_pos.y *= scale;
    }

    // Apply eye offset
    world_pos.x += camera.eye_offset.x;
    
    var output: VertexOutput;
    output.position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    output.uv = uvs[vertex_index];
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;
    let has_video = camera.eye_offset.y > 0.5;
    
    if (has_video) {
        // Sample video texture
        let video_color = textureSample(video_texture, video_sampler, uv);
        return video_color;
    } else {
        // Fallback: Procedural test pattern
        let time = camera.eye_offset.z;
        
        let grid_scale = 10.0;
        let scroll_x = uv.x + time * 0.2;
        let scroll_y = uv.y + sin(time) * 0.1;
        
        let g_x = step(0.95, fract(scroll_x * grid_scale));
        let g_y = step(0.95, fract(scroll_y * grid_scale));
        let grid = max(g_x, g_y);
        
        let center_dist = distance(uv, vec2<f32>(0.5, 0.5));
        let circle = 1.0 - smoothstep(0.1, 0.11, center_dist);
        
        let base_color = vec3<f32>(0.1, 0.1, 0.2);
        let grid_color = vec3<f32>(0.0, 0.8, 1.0);
        let circle_color = vec3<f32>(1.0, 0.2, 0.4);
        
        var final_color = mix(base_color, grid_color, grid);
        final_color = mix(final_color, circle_color, circle);
        
        // Screen Border
        if (uv.x < 0.01 || uv.x > 0.99 || uv.y < 0.02 || uv.y > 0.98) {
            return vec4<f32>(0.8, 0.8, 0.8, 1.0);
        }
        
        return vec4<f32>(final_color, 1.0);
    }
}
