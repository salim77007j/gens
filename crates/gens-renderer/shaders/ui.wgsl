// UI pipeline: egui tessellated meshes, already in clip space.
//
// Fragment = vertex color x texture sample, both premultiplied alpha;
// the pipeline blends with one/one-minus-src-alpha (egui's blending
// convention). No uniforms: vertices arrive in NDC (converted from
// pixels on the CPU by `UiVertex::ndc_from_pixel`, unit-tested there).

struct UiIn {
    @location(0) pos: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) color: vec4<f32>,
}

struct UiOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) color: vec4<f32>,
}

@group(0) @binding(0) var ui_texture: texture_2d<f32>;
@group(0) @binding(1) var ui_sampler: sampler;

@vertex
fn vs_ui(in: UiIn) -> UiOut {
    var out: UiOut;
    out.pos = vec4<f32>(in.pos, 0.0, 1.0);
    out.uv = in.uv;
    out.color = in.color;
    return out;
}

@fragment
fn fs_ui(in: UiOut) -> @location(0) vec4<f32> {
    let texel = textureSample(ui_texture, ui_sampler, in.uv);
    return in.color * texel;
}
