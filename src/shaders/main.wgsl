// VR Shader with camera transform and video texture support
// Supports stereoscopic rendering with view/projection matrices

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    eye_offset: vec4<f32>,  // x = offset, y = has_video, z = time, w = content_scale
    video_info: vec4<f32>,  // x = aspect_ratio (w/h), y = width, z = height, w = unused
};

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// Video textures (Y and UV planes)
@group(1) @binding(0)
var texture_y: texture_2d<f32>;
@group(1) @binding(1)
var texture_uv: texture_2d<f32>;
@group(1) @binding(2)
var video_sampler: sampler;
@group(1) @binding(3)
var ui_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// ... vertex shader unchanged ...

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Dynamic screen quad based on video aspect ratio
    // Base screen: 2.0m height at Z = -2.0
    let base_height = 1.8;
    let aspect = camera.video_info.x;
    
    // Calculate width based on aspect ratio
    // aspect = width/height, so width = height * aspect
    var half_w = base_height * 0.5 * aspect;
    var half_h = base_height * 0.5;
    
    // Clamp max width to 3.2m for comfort
    if (half_w > 1.6) {
        let scale = 1.6 / half_w;
        half_w = 1.6;
        half_h = half_h * scale;
    }
    
    // For vertical videos (aspect < 1), limit height and adjust
    if (aspect < 1.0) {
        half_h = base_height * 0.5;
        half_w = half_h * aspect;
    }
    
    var positions = array<vec3<f32>, 6>(
        vec3<f32>(-half_w,  half_h, -2.0), // TL
        vec3<f32>(-half_w, -half_h, -2.0), // BL
        vec3<f32>( half_w,  half_h, -2.0), // TR
        vec3<f32>( half_w,  half_h, -2.0), // TR
        vec3<f32>(-half_w, -half_h, -2.0), // BL
        vec3<f32>( half_w, -half_h, -2.0), // BR
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
        // YUV to RGB Conversion (BT.601 Limited Range)
        let y_raw = textureSample(texture_y, video_sampler, uv).r;
        let uv_val = textureSample(texture_uv, video_sampler, uv).rg;
        
        // Adjust for Limited Range (16-235 for Y, 16-240 for UV)
        // 1.164 = 255 / (235-16)
        // 0.0625 = 16 / 256
        let y = 1.1643 * (y_raw - 0.0625);
        let u = uv_val.r - 0.5;
        let v = uv_val.g - 0.5;
        
        let r = y + 1.596 * v;
        let g = y - 0.391 * u - 0.813 * v;
        let b = y + 2.018 * u;
        
        var rgb = vec3<f32>(r, g, b);

        // Mix UI Overlay (Composite in sRGB space)
        let ui_color = textureSample(ui_texture, video_sampler, uv);
        rgb = mix(rgb, ui_color.rgb, ui_color.a);

        // Linearize (Approximate Gamma 2.2 decoding) to prevent Double Gamma
        rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(2.2));
        
        return vec4<f32>(rgb, 1.0);
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
        
        // Mix UI Overlay (same as video path)
        let ui_color = textureSample(ui_texture, video_sampler, uv);
        final_color = mix(final_color, ui_color.rgb, ui_color.a);
        
        return vec4<f32>(final_color, 1.0);
    }
}
