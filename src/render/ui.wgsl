// wgpu-gameui main UI shader.
//
// Entry points share an ortho-projection uniform (group 0); the textured paths
// also bind the atlas (group 1):
//   - `vs_color`/`fs_color`: colored quads (DrawList vertices); supports per-vertex
//     scissor via the (clip, clip_enabled) attributes.
//   - `vs_chrome`/`fs_chrome`: instanced SDF rounded-rect button chrome.
//   - `vs_icon`/`fs_icon`: instanced textured quads (icons, sprites, images);
//     corners baked per-instance, bilinearly interpolated, atlas × tint.
//   - `vs_nine_slice`/`fs_nine_slice`: instanced nine-slice panels; the fragment
//     remaps local coords into the source UV (nine-region piecewise map).
//
// Clipping is done per-pixel against the per-vertex/instance clip rect (matches
// the DrawList::push_clip API). When `clip_enabled <= 0.5`, the rect is ignored.

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

// ---- Atlas bindings (shared by the icon + nine-slice paths) --------------

@group(1) @binding(0) var atlas_tex: texture_2d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;

// ---- Instanced icon / image path -----------------------------------------
//
// One unit-quad base mesh, one instance per icon/image (icons, sprites, and
// cropped images all flow through here). The 4 world-space corners are baked
// into the instance (the transform is applied DrawList-side), so the vertex
// shader bilinearly interpolates them by the unit-quad coord — handling
// rotation/scale/shear for free, no fallback. UV is a linear lerp of the
// instance's source rect. Replaces re-tessellating 6 verts/icon into the
// textured soup + re-uploading it every frame.

struct IconVsIn {
    // Base mesh: unit-quad corner in [0,1]^2.
    @location(0) corner: vec2<f32>,
    // Per-instance:
    @location(1) c_tl_tr: vec4<f32>,  // tl.x, tl.y, tr.x, tr.y (world space)
    @location(2) c_br_bl: vec4<f32>,  // br.x, br.y, bl.x, bl.y (world space)
    @location(3) uv_rect: vec4<f32>,  // u0, v0, u1, v1
    @location(4) tint: vec4<f32>,
    @location(5) clip: vec4<f32>,     // x, y, w, h
    @location(6) flags: vec4<f32>,    // clip_enabled, _pad, _pad, _pad
};

struct IconVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
    @location(2) clip: vec4<f32>,
    @location(3) clip_enabled: f32,
    @location(4) frag_pos: vec2<f32>,
};

@vertex
fn vs_icon(in: IconVsIn) -> IconVsOut {
    var out: IconVsOut;
    let tl = in.c_tl_tr.xy;
    let tr = in.c_tl_tr.zw;
    let br = in.c_br_bl.xy;
    let bl = in.c_br_bl.zw;
    let u = in.corner.x;
    let v = in.corner.y;
    // Bilinear interp of the four (possibly rotated/sheared) corners.
    let top = mix(tl, tr, u);
    let bot = mix(bl, br, u);
    let world = mix(top, bot, v);
    out.clip_position = uniforms.view_proj * vec4<f32>(world, 0.0, 1.0);
    out.uv = vec2<f32>(mix(in.uv_rect.x, in.uv_rect.z, u), mix(in.uv_rect.y, in.uv_rect.w, v));
    out.tint = in.tint;
    out.clip = in.clip;
    out.clip_enabled = in.flags.x;
    out.frag_pos = world;
    return out;
}

@fragment
fn fs_icon(in: IconVsOut) -> @location(0) vec4<f32> {
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

// ---- Instanced nine-slice path -------------------------------------------
//
// One unit-quad base mesh, one instance per nine-slice panel. The fragment
// remaps the quad's local coordinates into the source UV with the classic
// piecewise-linear nine-slice map (corners 1:1, edges stretched along one axis,
// center stretched both ways), then samples the atlas. Because the UV math is in
// the instance's LOCAL space, the full affine (incl. rotation/scale) is baked
// into the instance and applied in the vertex shader — so unlike the
// screen-space SDF chrome path, nine-slices need NO immediate-tessellation
// fallback. Replaces re-tessellating 9 quads (54 verts) per panel into the
// textured soup every frame.

struct NineVsIn {
    // Base mesh: unit-quad corner in [0,1]^2.
    @location(0) corner: vec2<f32>,
    // Per-instance:
    @location(1) lin: vec4<f32>,          // affine linear part: a, b, c, d (row-major)
    @location(2) tp: vec4<f32>,           // tx, ty, clip_enabled, _pad
    @location(3) origin_size: vec4<f32>,  // local x, y, w, h (pre-transform)
    @location(4) uv_outer: vec4<f32>,     // u0, v0, u3, v3 (outer edges)
    @location(5) uv_inner: vec4<f32>,     // u1, v1, u2, v2 (inner border seams)
    @location(6) border: vec4<f32>,       // bl, bt, br, bb (screen px)
    @location(7) tint: vec4<f32>,
    @location(8) clip: vec4<f32>,         // x, y, w, h
};

struct NineVsOut {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) cell: vec2<f32>,         // px from the panel's local top-left
    @location(1) size: vec2<f32>,
    @location(2) uv_outer: vec4<f32>,
    @location(3) uv_inner: vec4<f32>,
    @location(4) border: vec4<f32>,
    @location(5) tint: vec4<f32>,
    @location(6) clip: vec4<f32>,
    @location(7) clip_enabled: f32,
    @location(8) frag_pos: vec2<f32>,
};

@vertex
fn vs_nine_slice(in: NineVsIn) -> NineVsOut {
    var out: NineVsOut;
    let cell = in.corner * in.origin_size.zw;
    let local = in.origin_size.xy + cell;
    let wx = in.lin.x * local.x + in.lin.y * local.y + in.tp.x;
    let wy = in.lin.z * local.x + in.lin.w * local.y + in.tp.y;
    out.clip_position = uniforms.view_proj * vec4<f32>(wx, wy, 0.0, 1.0);
    out.cell = cell;
    out.size = in.origin_size.zw;
    out.uv_outer = in.uv_outer;
    out.uv_inner = in.uv_inner;
    out.border = in.border;
    out.tint = in.tint;
    out.clip = in.clip;
    out.clip_enabled = in.tp.z;
    out.frag_pos = vec2<f32>(wx, wy);
    return out;
}

// Map one axis: screen coord `c` in `[0, size]` to source UV, with `b0`/`b1` the
// near/far border widths (screen px) and `o0,i0,i1,o1` the outer/inner/inner/outer
// UV stops. Mirrors the CPU tessellator's per-region linear interpolation,
// including the collapse to the midpoint when the panel is narrower than its
// combined borders.
fn nine_axis(c: f32, size: f32, b0: f32, b1: f32, o0: f32, i0: f32, i1: f32, o1: f32) -> f32 {
    var x1 = b0;
    var x2 = size - b1;
    if (x1 > x2) {
        let m = (x1 + x2) * 0.5;
        x1 = m;
        x2 = m;
    }
    if (c <= x1) {
        let t = select(0.0, c / x1, x1 > 0.0);
        return mix(o0, i0, t);
    } else if (c >= x2) {
        let denom = size - x2;
        let t = select(0.0, (c - x2) / denom, denom > 0.0);
        return mix(i1, o1, t);
    }
    let denom = x2 - x1;
    let t = select(0.0, (c - x1) / denom, denom > 0.0);
    return mix(i0, i1, t);
}

@fragment
fn fs_nine_slice(in: NineVsOut) -> @location(0) vec4<f32> {
    if (in.clip_enabled > 0.5) {
        let p = in.frag_pos;
        if (p.x < in.clip.x || p.x > in.clip.x + in.clip.z
            || p.y < in.clip.y || p.y > in.clip.y + in.clip.w) {
            discard;
        }
    }
    let u = nine_axis(in.cell.x, in.size.x, in.border.x, in.border.z,
                      in.uv_outer.x, in.uv_inner.x, in.uv_inner.z, in.uv_outer.z);
    let v = nine_axis(in.cell.y, in.size.y, in.border.y, in.border.w,
                      in.uv_outer.y, in.uv_inner.y, in.uv_inner.w, in.uv_outer.w);
    let sampled = textureSample(atlas_tex, atlas_sampler, vec2<f32>(u, v));
    return sampled * in.tint;
}
