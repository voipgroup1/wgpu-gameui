// wgpu-gameui main UI shader.
//
// Two entry points share an ortho-projection uniform:
//   - `vs_color`/`fs_color`: colored quads (DrawList vertices); supports per-vertex
//     scissor via the (clip, clip_enabled) attributes.
//   - `vs_tex`/`fs_tex`: textured quads (icons, nine-slice). Samples the bound atlas
//     and multiplies by a per-vertex tint. Also supports per-vertex scissor.
//
// Clipping is done per-pixel against the per-vertex clip rect (matches the
// DrawList::push_clip API). When `clip_enabled <= 0.5`, the rect is ignored.

struct Uniforms {
    view_proj: mat4x4<f32>,
};
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

// ---- Colored quad path ---------------------------------------------------

struct ColorVsIn {
    @location(0) position: vec2<f32>,
    @location(1) color: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) clip_enabled: f32,
};

struct ColorVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) clip: vec4<f32>,
    @location(2) clip_enabled: f32,
    @location(3) frag_pos: vec2<f32>,
};

@vertex
fn vs_color(in: ColorVsIn) -> ColorVsOut {
    var out: ColorVsOut;
    out.clip_position = uniforms.view_proj * vec4<f32>(in.position, 0.0, 1.0);
    out.color = in.color;
    out.clip = in.clip;
    out.clip_enabled = in.clip_enabled;
    out.frag_pos = in.position;
    return out;
}

@fragment
fn fs_color(in: ColorVsOut) -> @location(0) vec4<f32> {
    if (in.clip_enabled > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }
    return in.color;
}

// ---- Textured quad path --------------------------------------------------

@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

struct TexVsIn {
    @location(0) position: vec2<f32>,
    @location(1) uv: vec2<f32>,
    @location(2) tint: vec4<f32>,
    @location(3) clip: vec4<f32>,
    @location(4) clip_enabled: f32,
};

struct TexVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) clip_enabled: f32,
    @location(4) frag_pos: vec2<f32>,
};

@vertex
fn vs_tex(in: TexVsIn) -> TexVsOut {
    var out: TexVsOut;
    out.clip_position = uniforms.view_proj * vec4<f32>(in.position, 0.0, 1.0);
    out.uv = in.uv;
    out.tint = in.tint;
    out.clip = in.clip;
    out.clip_enabled = in.clip_enabled;
    out.frag_pos = in.position;
    return out;
}

@fragment
fn fs_tex(in: TexVsOut) -> @location(0) vec4<f32> {
    if (in.clip_enabled > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }
    let sampled = textureSample(atlas_tex, atlas_sampler, in.uv);
    return sampled * in.tint;
}
