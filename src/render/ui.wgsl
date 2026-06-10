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

// ---- Instanced chrome path (SDF rounded rect) ----------------------------
//
// One unit-quad base mesh, one instance per button-like "chrome" rect. The
// fragment computes a rounded-rect signed distance, so fill + border + crisp
// anti-aliased corners come from a single instanced draw regardless of size.
// Replaces re-tessellating identical button geometry into the vertex soup every
// frame (see benches/ui_stress.rs / the instancing work).

struct ChromeVsIn {
    // Base mesh: unit-quad corner in [0,1]^2.
    @location(0) corner: vec2<f32>,
    // Per-instance:
    @location(1) rect: vec4<f32>,    // x, y, w, h  (post-transform world space)
    @location(2) bg: vec4<f32>,      // fill color
    @location(3) border: vec4<f32>,  // border color
    @location(4) clip: vec4<f32>,    // clip rect x, y, w, h
    @location(5) params: vec4<f32>,  // radius, thickness, clip_enabled, _pad
};

struct ChromeVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) local: vec2<f32>,   // px from the rect's top-left
    @location(1) size: vec2<f32>,
    @location(2) bg: vec4<f32>,
    @location(3) border: vec4<f32>,
    @location(4) clip: vec4<f32>,
    @location(5) params: vec4<f32>,
    @location(6) frag_pos: vec2<f32>,
};

@vertex
fn vs_chrome(in: ChromeVsIn) -> ChromeVsOut {
    var out: ChromeVsOut;
    let world = in.rect.xy + in.corner * in.rect.zw;
    out.clip_position = uniforms.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.local = in.corner * in.rect.zw;
    out.size = in.rect.zw;
    out.bg = in.bg;
    out.border = in.border;
    out.clip = in.clip;
    out.params = in.params;
    out.frag_pos = world;
    return out;
}

@fragment
fn fs_chrome(in: ChromeVsOut) -> @location(0) vec4<f32> {
    // Per-pixel scissor (same convention as fs_color).
    if (in.params.z > 0.5) {
        let pc = in.frag_pos;
        if (pc.x < in.clip.x || pc.x > in.clip.x + in.clip.z
            || pc.y < in.clip.y || pc.y > in.clip.y + in.clip.w) {
            discard;
        }
    }

    let half = in.size * 0.5;
    let max_r = min(half.x, half.y);
    let radius = clamp(in.params.x, 0.0, max_r);
    let thickness = clamp(in.params.y, 0.0, max_r);

    // Signed distance to the rounded rect (negative inside).
    let p = in.local - half;
    let q = abs(p) - half + vec2<f32>(radius);
    let d = length(max(q, vec2<f32>(0.0))) + min(max(q.x, q.y), 0.0) - radius;

    let aa = max(fwidth(d), 1e-4);
    let outer = 1.0 - smoothstep(-aa, aa, d);              // coverage inside outer edge
    let inner = 1.0 - smoothstep(-aa, aa, d + thickness);  // coverage inside the fill region
    var color = mix(in.border, in.bg, inner);
    let alpha = outer * color.a;
    if (alpha <= 0.0) {
        discard;
    }
    return vec4<f32>(color.rgb, alpha);
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
