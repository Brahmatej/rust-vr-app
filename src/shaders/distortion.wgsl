// Distortion Shader for VR Barrel Distortion and Lens Masking with Chromatic Aberration

struct DistortionUniforms {
    lens_radius: f32,       // Vignette Falloff Radius (0.5 - 1.5)
    lens_center_offset: f32, // Horizontal shift per eye
    scale_factor: f32,      // Dynamic Zoom
    padding2: f32,
};

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> params: DistortionUniforms;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    var positions = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, 1.0), 
        vec2<f32>(-1.0, -3.0),
        vec2<f32>( 3.0, 1.0)  
    );
    
    var pos = positions[vertex_index];
    var output: VertexOutput;
    output.position = vec4<f32>(pos, 0.0, 1.0);
    // Standard UV: (0,0) bottom-left to (1,1) top-right ? 
    // Usually WGPU/Vulkan is (0,0) top-left.
    // The previous shader used: ((pos.x + 1.0) * 0.5, (1.0 - pos.y) * 0.5);
    output.uv = vec2<f32>((pos.x + 1.0) * 0.5, (1.0 - pos.y) * 0.5);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    var uv = input.uv;
    
    // 1. Determine Eye Center
    var center = vec2<f32>(0.25, 0.5);
    if (uv.x > 0.5) {
        center = vec2<f32>(0.75 + params.lens_center_offset, 0.5);
    } else {
        center = vec2<f32>(0.25 - params.lens_center_offset, 0.5);
    }
    
    // 2. Local UV in Eye Space
    var local_uv = (uv - center);
    local_uv.x = local_uv.x * 4.0;
    local_uv.y = local_uv.y * 2.0;
    
    let r2 = dot(local_uv, local_uv);
    let r = sqrt(r2);
    
    // 3. Distortion Coefficients (Standard Cardboard)
    let k1 = 0.35; // Slightly stronger for "glassy" feel
    let k2 = 0.20;
    let d = 1.0 + k1 * r2 + k2 * r2 * r2;
    
    // 4. Chromatic Aberration
    // We scale the distortion factor 'd' slightly differently for each channel
    // Blue is distorted MOST (closest to center), Red LEAST (outer edge)
    // or vice versa depending on lens material. Usually Blue bends more.
    let ca_strength = 0.02 * r; // CA increases at edges
    
    let d_red = d * (1.0 - ca_strength);
    let d_green = d;
    let d_blue = d * (1.0 + ca_strength);
    
    // Apply Dynamic Scale (Zoom)
    let s = params.scale_factor;
    
    let theta = atan2(local_uv.y, local_uv.x);
    let cost = cos(theta);
    let sint = sin(theta);
    
    // Calculate 3 sample positions
    let r_red = r * d_red * s;
    let r_green = r * d_green * s;
    let r_blue = r * d_blue * s;
    
    let uv_red = center + vec2<f32>(r_red * cost / 4.0, r_red * sint / 2.0);
    let uv_green = center + vec2<f32>(r_green * cost / 4.0, r_green * sint / 2.0);
    let uv_blue = center + vec2<f32>(r_blue * cost / 4.0, r_blue * sint / 2.0);
    
    // 5. Sampling with Bounds Check
    // Helper function (manual since closures aren't valid in WGSL 1.0 same way)
    // We'll just do it inline or check validity.
    
    // Soft Vignette based on Lens Radius
    // smoothstep(edge0, edge1, x): returns 0.0 if x < edge0, 1.0 if x > edge1
    // We want 1.0 at center, 0.0 at edge.
    // So 1.0 - smoothstep(radius - fade, radius, r)
    let fade_width = 0.2;
    let vignette = 1.0 - smoothstep(params.lens_radius - fade_width, params.lens_radius, r);
    
    var color = vec4<f32>(0.0, 0.0, 0.0, 1.0);
    
    // Check bounds for Green (dominant) to fail fast? No, simpler to just sample.
    // If any UV is out of bounds, it returns black via sampler border or clamping?
    // We enabled ClampToEdge in renderer, so it smears. We need manual black.
    
    // Valid check:
    let valid_r = (uv_red.x >= 0.0 && uv_red.x <= 1.0 && uv_red.y >= 0.0 && uv_red.y <= 1.0);
    let valid_g = (uv_green.x >= 0.0 && uv_green.x <= 1.0 && uv_green.y >= 0.0 && uv_green.y <= 1.0);
    let valid_b = (uv_blue.x >= 0.0 && uv_blue.x <= 1.0 && uv_blue.y >= 0.0 && uv_blue.y <= 1.0);
    
    if (valid_r) { color.r = textureSample(screen_texture, screen_sampler, uv_red).r; }
    if (valid_g) { color.g = textureSample(screen_texture, screen_sampler, uv_green).g; }
    if (valid_b) { color.b = textureSample(screen_texture, screen_sampler, uv_blue).b; }
    
    // Cross-eye bleed protection
    let left_eye = uv.x < 0.5;
    if (left_eye && (uv_red.x >= 0.5 || uv_green.x >= 0.5 || uv_blue.x >= 0.5)) { color = vec4<f32>(0.0, 0.0, 0.0, 1.0); }
    if (!left_eye && (uv_red.x < 0.5 || uv_green.x < 0.5 || uv_blue.x < 0.5)) { color = vec4<f32>(0.0, 0.0, 0.0, 1.0); }

    return color * vignette;
}
