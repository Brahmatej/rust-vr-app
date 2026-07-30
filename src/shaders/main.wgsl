// VR Shader with camera transform and video texture support
// Supports stereoscopic rendering with view/projection matrices

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    eye_offset: vec4<f32>,  // x = offset, y = has_video, z = time, w = content_scale
    video_info: vec4<f32>,  // x = aspect_ratio (w/h), y = width, z = height, w = unused
    stereo: vec4<f32>,      // x = mode (0 mono,1 SBS,2 over-under), y = eye_index
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
@group(1) @binding(4)
var web_texture: texture_2d<f32>;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

// ... vertex shader unchanged ...

// The screen is a tessellated GRID mapped onto a spherical dome section, so it
// curves on BOTH axes (not a flat quad). Draw call requests SCREEN_COLS*SCREEN_ROWS*6.
const SCREEN_COLS: u32 = 64u;
const SCREEN_ROWS: u32 = 36u;

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Stereo frames pack two eyes in one texture, so a single eye is half as wide
    // (SBS) or half as tall (over-under). Correct the aspect so it isn't stretched.
    let smode = camera.stereo.x;
    var aspect = camera.video_info.x;
    if (smode > 0.5 && smode < 1.5) { aspect = aspect * 0.5; }       // SBS
    else if (smode > 1.5) { aspect = aspect * 2.0; }                 // over-under

    let scale  = max(camera.eye_offset.w, 0.1);   // content_scale (zoom)
    let radius = 5.3;
    let base_h = 1.6;
    let screen_h = base_h * scale;                // grows uniformly with zoom
    let screen_w = screen_h * aspect;

    // Decode grid cell + corner from the vertex index (two triangles per quad).
    let quad  : u32 = vertex_index / 6u;
    let local : u32 = vertex_index % 6u;
    let col   : u32 = quad % SCREEN_COLS;
    let row   : u32 = quad / SCREEN_COLS;
    let du = select(0u, 1u, local == 2u || local == 3u || local == 5u);
    let is_top = (local == 0u || local == 2u || local == 3u);
    let dv = select(1u, 0u, is_top);              // is_top → v = 0
    let u_coord = f32(col + du) / f32(SCREEN_COLS);
    let v_coord = f32(row + dv) / f32(SCREEN_ROWS);

    // Projection mode (camera.stereo.z): 0 = flat curved screen, 1 = 180° dome,
    // 2 = 360° dome, 3 = vertical/portrait panel.
    //
    // For the dome modes the source is equirectangular, so the angular span is fixed
    // by the FORMAT (pi for 180, 2pi for 360) rather than by the screen size - the
    // image has to wrap the viewer at true scale or the geometry doesn't line up.
    // This is what makes SBS 180/360 content actually work: each eye samples its half
    // of the frame (handled in fs_main) mapped over a real hemisphere.
    let pmode = camera.stereo.z;
    var arc_h = screen_w / radius;
    var arc_v = screen_h / radius;

    if (pmode > 0.5 && pmode < 1.5) {             // 180° dome
        arc_h = 3.14159265;
        arc_v = 3.14159265 * 0.5;
    } else if (pmode > 1.5 && pmode < 2.5) {      // 360° dome
        arc_h = 6.28318531;
        arc_v = 3.14159265;
    } else if (pmode > 2.5) {                     // vertical / portrait
        // Tall panel: swap the aspect so portrait video fills the height.
        let vert_h = base_h * scale * 1.9;
        arc_v = vert_h / radius;
        arc_h = (vert_h / max(aspect, 0.1)) / radius;
    }

    let theta = (u_coord - 0.5) * arc_h;
    let phi   = (0.5 - v_coord) * arc_v;          // v=0 (top) → +phi

    // Point on the sphere (curves horizontally AND vertically), centred at -Z.
    var world_pos = vec3<f32>(
        radius * cos(phi) * sin(theta),
        radius * sin(phi),
        -radius * cos(phi) * cos(theta));
    world_pos.x += camera.eye_offset.x;           // stereo eye shift

    var output: VertexOutput;
    output.position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    output.uv = vec2<f32>(u_coord, v_coord);
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let uv = input.uv;
    let has_video = camera.eye_offset.y > 0.5;
    let is_web = camera.video_info.w > 0.5;

    // Stereo remap: each eye samples its half of the frame. eye_index 0/2 → first
    // half (left/top), 1 → second half (right/bottom). Shared by web + video.
    let smode = camera.stereo.x;
    let is_right = camera.stereo.y > 0.5 && camera.stereo.y < 1.5;
    var suv = uv;
    if (smode > 0.5 && smode < 1.5) {          // side-by-side
        suv.x = uv.x * 0.5 + select(0.0, 0.5, is_right);
    } else if (smode > 1.5) {                  // over-under
        suv.y = uv.y * 0.5 + select(0.0, 0.5, is_right);
    }

    if (is_web) {
        // Browser page (already RGB). sRGB texture auto-linearizes on sample; the
        // surface is sRGB too, so return linear directly (the UI is a separate panel).
        let rgb = textureSample(web_texture, video_sampler, suv).rgb;
        return vec4<f32>(rgb, 1.0);
    }

    if (has_video) {
        // YUV to RGB Conversion (BT.601 Limited Range)
        let y_raw = textureSample(texture_y, video_sampler, suv).r;
        let uv_val = textureSample(texture_uv, video_sampler, suv).rg;
        
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
        // Linearize (approximate gamma 2.2) to prevent double gamma on the sRGB surface.
        rgb = pow(max(rgb, vec3<f32>(0.0)), vec3<f32>(2.2));
        return vec4<f32>(rgb, 1.0);
    } else {
        // Idle backdrop. The old default was a scrolling cyan test grid with a pink
        // dot - fine as a "is the renderer alive" probe, unpleasant to actually sit
        // in. This is a calm M3-style graded surface instead: a soft vertical ramp
        // through the dark violet-neutral tones, a gentle radial lift behind the
        // centre so the panel has something to sit against, and a vignette to keep
        // the edges from glowing in the lenses.
        let time = camera.eye_offset.z;

        // M3 dark surface tones, linearised (~gamma 2.2) since the target is sRGB.
        let top    = vec3<f32>(0.055, 0.050, 0.070);
        let bottom = vec3<f32>(0.015, 0.014, 0.022);
        var final_color = mix(top, bottom, uv.y);

        // Soft centre glow, drifting very slowly so it never looks like a dead pixel field.
        let drift = vec2<f32>(0.5 + sin(time * 0.08) * 0.02,
                              0.42 + cos(time * 0.06) * 0.02);
        let d = distance(uv, drift);
        let glow = 1.0 - smoothstep(0.0, 0.55, d);
        final_color += vec3<f32>(0.050, 0.038, 0.080) * glow;

        // Vignette.
        let vig = 1.0 - smoothstep(0.45, 0.95, distance(uv, vec2<f32>(0.5, 0.5)));
        final_color *= mix(0.55, 1.0, vig);

        return vec4<f32>(final_color, 1.0);
    }
}
