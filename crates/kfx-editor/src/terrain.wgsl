// Terrain and overlay shaders.

struct Globals {
    view_proj: mat4x4<f32>,
    // xyz: unit vector from the target towards the eye (for face shading)
    eye_dir: vec4<f32>,
    // x: animation frame 0..7; y: flags (1 ownership tint, 2 slab grid, 4 lighting);
    // z: local light count
    params: vec4<u32>,
    // x: exposure of the tone curve (1 = unchanged)
    tone: vec4<f32>,
    // local lights: xyz position, w range
    lights: array<vec4<f32>, 16>,
    // intensity of each local light, four per vector
    light_power: array<vec4<f32>, 4>,
};

@group(0) @binding(0) var<uniform> g: Globals;
@group(0) @binding(1) var atlas: texture_2d_array<f32>;
@group(0) @binding(2) var lut: texture_2d<u32>;

const BLOCKS: u32 = 1544u;
const LAYER_TILES: u32 = 1088u;

struct VIn {
    @location(0) xz: vec2<u32>,
    // x: height, y: uv bits, z: face, w: owner
    @location(1) huvf: vec4<u32>,
    @location(2) slot: u32,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) tile: u32,
    @location(2) shade: f32,
    @location(3) world: vec3<f32>,
    @location(4) @interpolate(flat) face_owner: vec2<u32>,
};

fn face_normal(f: u32) -> vec3<f32> {
    switch f {
        case 1u: { return vec3<f32>(0.0, 0.0, -1.0); }
        case 2u: { return vec3<f32>(1.0, 0.0, 0.0); }
        case 3u: { return vec3<f32>(0.0, 0.0, 1.0); }
        case 4u: { return vec3<f32>(-1.0, 0.0, 0.0); }
        case 5u: { return vec3<f32>(0.0, -1.0, 0.0); }
        default: { return vec3<f32>(0.0, 1.0, 0.0); }
    }
}

// Player colours (player_instances.h order: red, blue, green, yellow, white, neutral, purple,
// black, orange).
fn player_color(o: u32) -> vec3<f32> {
    switch o {
        case 0u: { return vec3<f32>(0.85, 0.12, 0.10); }
        case 1u: { return vec3<f32>(0.18, 0.35, 0.95); }
        case 2u: { return vec3<f32>(0.15, 0.75, 0.22); }
        case 3u: { return vec3<f32>(0.95, 0.85, 0.15); }
        case 4u: { return vec3<f32>(0.95, 0.95, 0.95); }
        case 6u: { return vec3<f32>(0.62, 0.22, 0.85); }
        case 7u: { return vec3<f32>(0.05, 0.05, 0.06); }
        case 8u: { return vec3<f32>(0.98, 0.52, 0.10); }
        default: { return vec3<f32>(0.5, 0.5, 0.5); }
    }
}

@vertex
fn vs_terrain(v: VIn) -> VOut {
    var o: VOut;
    let world = vec3<f32>(f32(v.xz.x), f32(v.huvf.x), f32(v.xz.y));
    o.pos = g.view_proj * vec4<f32>(world, 1.0);
    o.world = world;
    o.uv = vec2<f32>(f32(v.huvf.y & 1u), f32((v.huvf.y >> 1u) & 1u));
    let row = g.params.x * 32u + v.slot / BLOCKS;
    let col = v.slot % BLOCKS;
    o.tile = textureLoad(lut, vec2<i32>(i32(col), i32(row)), 0).x;
    let f = v.huvf.z;
    o.face_owner = vec2<u32>(f, v.huvf.w);
    let n = face_normal(f);
    if (g.params.y & 4u) != 0u {
        // Ambient plus one directional light from above the south-east (behind the default
        // isometric camera), so the sides the default view shows are lit, and a slight height
        // cue so wall tops stand out from floors.
        let l = normalize(vec3<f32>(0.5, 1.0, 0.7));
        let d = max(dot(n, l), 0.0);
        let h = 0.96 + 0.025 * min(world.y, 5.0);
        o.shade = (0.62 + 0.62 * d) * h;
    } else if f == 0u {
        o.shade = 1.0;
    } else if f == 5u {
        o.shade = 0.55;
    } else {
        // Sides facing the viewer are lit more, like the game's normal_shade_* (engine_render.c:477-490).
        let e = normalize(vec3<f32>(g.eye_dir.x, 0.0, g.eye_dir.z));
        o.shade = 0.62 + 0.33 * max(dot(n, e), 0.0);
    }
    return o;
}

@fragment
fn fs_terrain(i: VOut) -> @location(0) vec4<f32> {
    // Texel-space derivatives choose the mip level; every level is fetched inside the tile.
    let t = i.uv * 32.0;
    let rho = max(length(dpdx(t)), length(dpdy(t)));
    // slab grid distance, computed in uniform control flow
    let gs = i.world.xz / 3.0;
    let gw = max(fwidth(gs), vec2<f32>(1e-4));
    let lod = clamp(i32(floor(log2(max(rho, 1e-6)))), 0, 5);
    let ts = 32 >> u32(lod);
    let layer = i32(i.tile / LAYER_TILES);
    let local = i.tile % LAYER_TILES;
    let base = vec2<i32>(i32(local % 32u) * ts, i32(local / 32u) * ts);
    let p = clamp(vec2<i32>(floor(i.uv * f32(ts))), vec2<i32>(0), vec2<i32>(ts - 1));
    let tex = textureLoad(atlas, base + p, layer, lod).rgb;
    var c = tex * i.shade;
    // local lights (the map's lights near the camera, close-up views): warm additive light
    // with a smooth quadratic falloff, only on surfaces facing it
    let nl = min(g.params.z, 16u);
    var add = vec3<f32>(0.0);
    for (var k = 0u; k < nl; k = k + 1u) {
        let l = g.lights[k];
        let d = l.xyz - i.world;
        let dist = length(d);
        let f = clamp(1.0 - dist / l.w, 0.0, 1.0);
        let pw = g.light_power[k / 4u][k % 4u];
        let facing = select(0.6, 1.0, i.face_owner.x == 0u);
        add = add + vec3<f32>(1.0, 0.82, 0.6) * (f * f * pw * facing);
    }
    c = c + tex * add * 1.4;
    // tone curve: e*c / (1 + (e-1)*c) lifts dark texels and keeps white white
    let e = g.tone.x;
    if e > 1.001 {
        c = e * c / (vec3<f32>(1.0) + (e - 1.0) * c);
    }
    let owner = i.face_owner.y;
    if (g.params.y & 1u) != 0u && owner < 9u && owner != 5u {
        c = mix(c, player_color(owner), 0.42);
    }
    if (g.params.y & 2u) != 0u && i.face_owner.x == 0u {
        let d = abs(fract(gs - 0.5) - 0.5) / gw;
        let line = 1.0 - clamp(min(d.x, d.y), 0.0, 1.0);
        c = mix(c, vec3<f32>(0.95, 0.85, 0.55), line * 0.45);
    }
    return vec4<f32>(c, 1.0);
}

struct LIn {
    @location(0) pos: vec3<f32>,
    @location(1) color: vec4<f32>,
};

struct LOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) color: vec4<f32>,
};

@vertex
fn vs_line(v: LIn) -> LOut {
    var o: LOut;
    o.pos = g.view_proj * vec4<f32>(v.pos, 1.0);
    o.color = v.color;
    return o;
}

@fragment
fn fs_line(i: LOut) -> @location(0) vec4<f32> {
    return i.color;
}
