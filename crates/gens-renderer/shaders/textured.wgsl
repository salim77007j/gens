// Gens textured-mesh pipeline (Phase 2).
//
// Same binding scheme as colored.wgsl plus the sampled texture in group 1:
//   group 0: camera uniform (view_proj)
//   group 1: { 0: model uniform (dynamic offset), 1: sampler, 2: texture }
//
// Fragment output = vertex color * texture sample, so tinting comes free
// by setting vertex colors (white = untinted).

struct CameraUniform {
    view_proj: mat4x4<f32>,
};

struct ModelUniform {
    model: mat4x4<f32>,
};

@group(0) @binding(0) var<uniform> camera: CameraUniform;
@group(1) @binding(0) var<uniform> model: ModelUniform;
@group(1) @binding(1) var samp: sampler;
@group(1) @binding(2) var tex: texture_2d<f32>;

struct VsIn {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
    @location(2) uv: vec2<f32>,
};

struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) uv: vec2<f32>,
};

@vertex
fn vs_main(in: VsIn) -> VsOut {
    var out: VsOut;
    let world = model.model * vec4<f32>(in.position, 1.0);
    out.clip = camera.view_proj * world;
    out.color = in.color;
    out.uv = in.uv;
    return out;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    return in.color * textureSample(tex, samp, in.uv);
}
