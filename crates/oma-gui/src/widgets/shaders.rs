//! WGSL for the pixel field: the site's `HeroPixelField` as a fragment shader.
//! A grid of square cells, each lit by drifting blob noise through an 8x8
//! Bayer dither, denser toward the edges and corners, with a cursor halo and
//! a data-driven pulse (thermal load).

pub const FIELD: &str = r#"
struct Uniforms {
    resolution: vec2<f32>,
    cursor: vec2<f32>,
    time: f32,
    cell: f32,
    load: f32,
    heat: f32,
    bg: vec4<f32>,
    dim: vec4<f32>,
    mid: vec4<f32>,
    lit: vec4<f32>,
    hover: vec4<f32>,
    crest: vec4<f32>,
    params: vec4<f32>, // alpha, hero_band (px from top where the field is denser), intensity, _
};
@group(0) @binding(0) var<uniform> u: Uniforms;

struct VsOut { @builtin(position) pos: vec4<f32>, @location(0) uv: vec2<f32> };

@vertex
fn vs_main(@builtin(vertex_index) i: u32) -> VsOut {
    var p = array<vec2<f32>, 3>(vec2(-1.0, -1.0), vec2(3.0, -1.0), vec2(-1.0, 3.0));
    var o: VsOut;
    o.pos = vec4(p[i], 0.0, 1.0);
    o.uv = (p[i] + vec2(1.0, 1.0)) * 0.5;
    return o;
}

fn hash(p: vec2<f32>) -> f32 {
    let h = dot(p, vec2(127.1, 311.7));
    return fract(sin(h) * 43758.5453123);
}

fn vnoise(p: vec2<f32>) -> f32 {
    let i = floor(p);
    let f = fract(p);
    let u2 = f * f * (3.0 - 2.0 * f);
    return mix(mix(hash(i), hash(i + vec2(1.0, 0.0)), u2.x), mix(hash(i + vec2(0.0, 1.0)), hash(i + vec2(1.0, 1.0)), u2.x), u2.y);
}

fn blobs(p: vec2<f32>) -> f32 {
    // Two octaves of smooth noise read as the site's box-blurred blobs.
    return 0.65 * vnoise(p) + 0.35 * vnoise(p * 2.1 + vec2(7.3, 2.9));
}

fn bayer(cx: i32, cy: i32) -> f32 {
    // 8x8 ordered dither, 0..63.
    let x = cx & 7;
    let y = cy & 7;
    var v = 0;
    v = v + ((x ^ y) & 1) * 32;
    v = v + (y & 1) * 16;
    v = v + (((x ^ y) >> 1) & 1) * 8;
    v = v + ((y >> 1) & 1) * 4;
    v = v + (((x ^ y) >> 2) & 1) * 2;
    v = v + ((y >> 2) & 1);
    return f32(v) / 64.0;
}

@fragment
fn fs_main(in: VsOut) -> @location(0) vec4<f32> {
    var uv = in.uv;
    uv.y = 1.0 - uv.y;
    let px = uv * u.resolution;
    let cell = max(u.cell, 4.0);
    let gx = i32(floor(px.x / cell));
    let gy = i32(floor(px.y / cell));
    let cxy = (vec2(f32(gx), f32(gy)) + vec2(0.5, 0.5)) * cell;
    // Square inside the cell, with a gap.
    let inner = fract(px / cell);
    let gap = 0.30;
    let in_square = step(gap * 0.5, inner.x) * step(inner.x, 1.0 - gap * 0.5) * step(gap * 0.5, inner.y) * step(inner.y, 1.0 - gap * 0.5);

    // Density field: drifting blobs, stronger at the edges and corners like the site,
    // and fading out over the hero band so text stays readable.
    let t = u.time * 0.05;
    let n = blobs(vec2(f32(gx), f32(gy)) / 9.0 + vec2(t, -t * 0.6));
    let edge_x = 1.0 - smoothstep(0.0, 0.5, min(uv.x, 1.0 - uv.x));
    let edge_y = 1.0 - smoothstep(0.0, 0.5, min(uv.y, 1.0 - uv.y));
    let corner = max(edge_x * edge_y * 1.4, 0.35 * max(edge_x, edge_y));
    var density = (n * n) * (0.06 + 0.94 * corner) * u.params.z * 0.9;
    density = density + 0.06 * u.load * n;
    // Cursor halo.
    let dc = distance(px, u.cursor) / (cell * 12.0);
    let halo = (1.0 - smoothstep(0.0, 1.0, dc));
    density = density + halo * 0.5;
    // Slow breathing tied to heat.
    density = density + 0.05 * sin(u.time * 0.8 + f32(gx) * 0.37 + f32(gy) * 0.21) * (0.3 + u.heat);

    let thr = bayer(gx, gy);
    let lit = step(thr, density * 1.15 - 0.12);
    // Band the colour by how far over the threshold the cell is.
    let over = clamp((density * 1.15 - 0.12 - thr) * 2.0, 0.0, 1.0);
    var col = u.dim.rgb;
    col = mix(col, u.mid.rgb, smoothstep(0.15, 0.45, over));
    col = mix(col, u.lit.rgb, smoothstep(0.45, 0.75, over));
    col = mix(col, u.hover.rgb, smoothstep(0.75, 0.92, over) * halo);
    col = mix(col, u.crest.rgb, smoothstep(0.92, 1.0, over) * halo);
    let a = lit * in_square * u.params.x;
    let out = mix(u.bg.rgb, col, a);
    let ba = u.params.x;
    return vec4(out * ba, ba);
}
"#;
