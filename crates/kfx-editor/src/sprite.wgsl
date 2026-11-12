// Camera-facing thing billboards with alpha cutout.
//
// Each instance is one sprite frame: the bitmap's rectangle in sprite pixels relative to the
// thing's anchor (y down, like the game's screen space), scaled by `scale` world units per sprite
// pixel and laid out along the camera's right / up axes, so it looks like the game's screen
// aligned sprites. The whole quad takes the depth of its anchor point moved towards the eye by
// `bias`, so a thing is hidden by terrain in front of its feet but never cut by the wall behind.

struct SpriteGlobals {
    view_proj: mat4x4<f32>,
    right: vec4<f32>,
    up: vec4<f32>,
    // xyz: unit vector towards the eye, w: depth bias in world units
    eye: vec4<f32>,
};

@group(0) @binding(0) var<uniform> g: SpriteGlobals;
@group(0) @binding(1) var tex: texture_2d<f32>;

struct IIn {
    @location(0) pos: vec3<f32>,
    @location(1) scale: f32,
    // x, y, w, h in sprite pixels relative to the anchor
    @location(2) rect: vec4<f32>,
    // texel rectangle x, y, w, h in the atlas
    @location(3) uv: vec4<f32>,
    @location(4) tint: vec4<f32>,
    // bit 0: mirror, bit 1: always on top, bit 2: translucent
    @location(5) flags: u32,
};

struct VOut {
    @builtin(position) pos: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) tint: vec4<f32>,
};

@vertex
fn vs_sprite(@builtin(vertex_index) vi: u32, i: IIn) -> VOut {
    var corners = array<vec2<f32>, 6>(
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 0.0), vec2<f32>(1.0, 1.0),
        vec2<f32>(0.0, 0.0), vec2<f32>(1.0, 1.0), vec2<f32>(0.0, 1.0),
    );
    let c = corners[vi];
    let px = i.rect.x + c.x * i.rect.z;
    let py = i.rect.y + c.y * i.rect.w;
    let world = i.pos + g.right.xyz * (px * i.scale) - g.up.xyz * (py * i.scale);
    var clip = g.view_proj * vec4<f32>(world, 1.0);
    let anchor = g.view_proj * vec4<f32>(i.pos + g.eye.xyz * g.eye.w, 1.0);
    var z = anchor.z / anchor.w;
    if ((i.flags & 2u) != 0u) {
        z = 0.0;
    }
    clip.z = clamp(z, 0.0, 1.0) * clip.w;
    var u = c.x;
    if ((i.flags & 1u) != 0u) {
        u = 1.0 - c.x;
    }
    var o: VOut;
    o.pos = clip;
    o.uv = vec2<f32>(i.uv.x + u * i.uv.z, i.uv.y + c.y * i.uv.w);
    o.tint = i.tint;
    return o;
}

@fragment
fn fs_sprite(v: VOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(tex));
    let t = clamp(vec2<i32>(floor(v.uv)), vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    let c = textureLoad(tex, t, 0);
    if (c.a < 0.5) {
        discard;
    }
    // tint: rgb multiplies, alpha < 1 blends towards the tint colour (selection highlight)
    let rgb = mix(v.tint.rgb, c.rgb * v.tint.rgb, v.tint.a);
    return vec4<f32>(rgb, 1.0);
}

// Translucent sprites (the game's TransparencyFlags: ghosts, heart flames, glows).
@fragment
fn fs_sprite_blend(v: VOut) -> @location(0) vec4<f32> {
    let dims = vec2<i32>(textureDimensions(tex));
    let t = clamp(vec2<i32>(floor(v.uv)), vec2<i32>(0, 0), dims - vec2<i32>(1, 1));
    let c = textureLoad(tex, t, 0);
    if (c.a < 0.5) {
        discard;
    }
    let rgb = mix(v.tint.rgb, c.rgb * v.tint.rgb, v.tint.a);
    return vec4<f32>(rgb, 0.5);
}
