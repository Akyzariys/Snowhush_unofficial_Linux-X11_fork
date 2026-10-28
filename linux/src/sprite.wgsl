//! Shared by the main shader: the snow is drawn in DIPs and scaled up to the panel's pixels.

struct Uniforms {
    view_size: vec2<f32>,
    scale: f32,
    time: f32,
    light: vec4<f32>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;

struct Instance {
    @location(0) position: vec2<f32>,
    @location(1) velocity: vec2<f32>,
    @location(2) size: f32,
    @location(3) alpha: f32,
    @location(4) soft: f32,
    @location(5) tint_amount: f32,
    @location(6) tint: vec3<f32>,
    @location(7) lump: f32,
};

struct SpriteOut {
    @builtin(position) position: vec4<f32>,
    @location(0) coord: vec2<f32>,
    @location(1) alpha: f32,
    @location(2) soft: f32,
    @location(3) tint: vec4<f32>,
    @location(4) streak: vec2<f32>,
    @location(5) radius: f32,
    @location(6) lump: f32,
    @location(7) spin: f32,
};

@vertex
fn sprite_vertex(sprite: Instance, @builtin(vertex_index) index: u32) -> SpriteOut {
    let pixels = sprite.size * u.scale;
    // About a 1/55 s exposure, capped so even the fastest grain stays a short smear and never reads as rain.
    let travel = sprite.velocity * 0.018;
    let smear = min(length(travel), sprite.size * select(2.2, 1.2, sprite.lump > 0.0));
    var streak = vec2<f32>(0.0);
    if (smear > 0.25) {
        streak = normalize(travel) * smear * u.scale;
    }
    // A lump's ragged edge reaches past its nominal size.
    let reach = select(1.0, 1.3, sprite.lump > 0.0);
    let extent = max(pixels * reach + length(streak), 2.0);
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let center = sprite.position / u.view_size * 2.0 - 1.0;

    var out: SpriteOut;
    out.position = vec4<f32>(center + (corner - 0.5) * extent * 2.0 / (u.view_size * u.scale), 0.0, 1.0);
    out.coord = vec2<f32>(corner.x, 1.0 - corner.y);
    out.streak = streak / extent;
    out.radius = pixels / extent * 0.5;
    // A streak spreads the same flake over more pixels, so each is fainter.
    out.alpha = sprite.alpha * min(pixels / 2.0, 1.0) * pixels / max(pixels + length(streak) * 0.6, 0.001);
    out.soft = sprite.soft;
    out.tint = vec4<f32>(sprite.tint, sprite.tint_amount);
    out.lump = sprite.lump;
    out.spin = select(0.0, u.time * (sprite.lump - 0.5) * 9.0 + sprite.lump * 40.0, sprite.lump > 0.0);
    return out;
}

fn saturate(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

@fragment
fn sprite_fragment(in: SpriteOut) -> @location(0) vec4<f32> {
    // Quad coordinates run down the screen; velocities run up.
    let p = vec2<f32>(in.coord.x - 0.5, 0.5 - in.coord.y);
    let half_ = in.streak * 0.5;
    let span = dot(half_, half_);
    var t = 0.0;
    if (span > 0.0) {
        t = clamp(dot(p, half_) / span, -1.0, 1.0);
    }
    let d = p - half_ * t;
    var r = length(d) / max(in.radius, 0.001);
    var shading = 0.0;
    if (in.lump > 0.0) {
        let angle = atan2(d.y, d.x) + in.spin;
        r /= 1.0 + 0.14 * sin(3.0 * angle) + 0.08 * sin(5.0 * angle + in.lump * 21.0) + 0.05 * sin(2.0 * angle + in.lump * 9.0);
        shading = saturate(dot(d / max(in.radius, 0.001), vec2<f32>(0.45, -0.6))) * 0.4;
    }
    // Brighter at the head, where the flake is now, than at the tail, where it was.
    var head = 1.0;
    if (span > 0.0) {
        head = 0.15 + 0.85 * pow(t * 0.5 + 0.5, 1.5);
    }
    let a = (1.0 - smoothstep(mix(0.6, 0.0, in.soft), 1.0, r)) * in.alpha * head;
    if (a < 0.002) {
        discard;
    }
    let core = vec3<f32>(0.965, 0.975, 0.995);
    let rim = vec3<f32>(0.62, 0.67, 0.77);
    var color = mix(core, rim, saturate(smoothstep(0.35, 1.0, r) * 0.65 + shading));
    color = mix(color, in.tint.rgb, in.tint.a) * u.light.rgb;
    return vec4<f32>(color * a, a);
}
