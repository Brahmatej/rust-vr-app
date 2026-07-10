// Floating UI panel — the dock / Media Center, drawn as its OWN curved panel using
// the SAME spherical-dome curvature as the main screen, but at a closer radius and a
// fixed comfortable size so it's always front-and-centre when invoked. Alpha-blended
// (egui premultiplied) on top of the screen. Draw call requests COLS*ROWS*6.

const COLS: u32 = 32u;
const ROWS: u32 = 32u;

struct CameraUniforms {
    view_proj: mat4x4<f32>,
    eye_offset: vec4<f32>,  // x = eye offset, y = has_video, z = time, w = content_scale
    video_info: vec4<f32>,
    stereo: vec4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(1) @binding(0) var ui_tex: texture_2d<f32>;
@group(1) @binding(1) var ui_samp: sampler;

struct VertexOutput {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vertex_index: u32) -> VertexOutput {
    // Fixed, comfortable, centred square panel at a close radius, curved on both axes
    // (same dome math as the screen). Scales gently with content zoom, capped so it
    // never swallows the view.
    let radius  = 2.0;
    let zoom    = clamp(camera.eye_offset.w, 0.8, 2.6);
    let panel_h = 1.7 * zoom;                 // square (ui texture is square)
    let arc     = panel_h / radius;

    let quad  : u32 = vertex_index / 6u;
    let local : u32 = vertex_index % 6u;
    let col   : u32 = quad % COLS;
    let row   : u32 = quad / COLS;
    let du = select(0u, 1u, local == 2u || local == 3u || local == 5u);
    let is_top = (local == 0u || local == 2u || local == 3u);
    let dv = select(1u, 0u, is_top);
    let u = f32(col + du) / f32(COLS);
    let v = f32(row + dv) / f32(ROWS);

    let theta = (u - 0.5) * arc;
    let phi   = (0.5 - v) * arc;
    var world_pos = vec3<f32>(
        radius * cos(phi) * sin(theta),
        radius * sin(phi),
        -radius * cos(phi) * cos(theta));
    world_pos.x += camera.eye_offset.x;       // stereo eye shift

    var out: VertexOutput;
    out.position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    out.uv = vec2<f32>(u, v);
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // egui outputs premultiplied alpha; pipeline blend is (One, OneMinusSrcAlpha).
    return textureSample(ui_tex, ui_samp, input.uv);
}
