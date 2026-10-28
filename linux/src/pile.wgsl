//! A drift is a height field of snow columns lying on a bed: the screen bottom, or the top edge
//! of a window whose rounded corners curve the bed down at both ends.

struct Uniforms {
    view_size: vec2<f32>,
    scale: f32,
    time: f32,
    column_width: f32,
    column_count: i32,
    pile_top: f32,
    occluder_count: i32,
    origin: vec2<f32>,
    pile_bottom: f32,
    height_offset: i32,
    light: vec4<f32>,
    shadow_tint: vec4<f32>,
    occluders: array<vec4<f32>, 16>,
};

@group(0) @binding(0) var<uniform> u: Uniforms;
@group(0) @binding(1) var<storage, read> heights: array<f32>;

struct PileOut {
    @builtin(position) position: vec4<f32>,
    @location(0) local: vec2<f32>,
};

@vertex
fn pile_vertex(@builtin(vertex_index) index: u32) -> PileOut {
    let corner = vec2<f32>(f32(index & 1u), f32(index >> 1u));
    let p = vec2<f32>(corner.x * f32(u.column_count) * u.column_width, mix(u.pile_bottom, u.pile_top, corner.y));
    var out: PileOut;
    out.position = vec4<f32>((p + u.origin) / u.view_size * 2.0 - 1.0, 0.0, 1.0);
    out.local = p;
    return out;
}

fn height_at(start: i32, x: f32) -> f32 {
    let f = x / u.column_width - 0.5;
    let base = floor(f);
    let i = clamp(i32(base), 0, u.column_count - 1);
    let j = min(i + 1, u.column_count - 1);
    let left = heights[u32(u.height_offset + start + i)];
    let right = heights[u32(u.height_offset + start + j)];
    return mix(left, right, clamp(f - base, 0.0, 1.0));
}

fn hash(p: vec2<f32>) -> f32 {
    var q = fract(p * vec2<f32>(123.34, 456.21));
    q += dot(q, q + vec2<f32>(45.32));
    return fract(q.x * q.y);
}

fn saturate(x: f32) -> f32 {
    return clamp(x, 0.0, 1.0);
}

@fragment
fn pile_fragment(in: PileOut) -> @location(0) vec4<f32> {
    let world = in.local + u.origin;
    for (var k: i32 = 0; k < u.occluder_count; k = k + 1) {
        let occluder = u.occluders[k];
        if (world.x >= occluder.x && world.x < occluder.z && world.y >= occluder.y && world.y < occluder.w) {
            discard;
        }
    }

    let x = in.local.x;
    let level = height_at(0, x);
    let bed = height_at(u.column_count, x);
    let depth = level - in.local.y;
    let coverage = saturate(depth * u.scale + 0.5)
        * saturate((in.local.y - bed) * u.scale + 0.5)
        * saturate((level - bed) / 0.75);
    if (coverage <= 0.0) {
        discard;
    }

    let slope = (height_at(0, x + 2.0) - height_at(0, x - 2.0)) * 0.25;
    let lit = dot(normalize(vec2<f32>(-slope, 1.0)), normalize(vec2<f32>(-0.5, 1.0)));

    let shadow = mix(vec3<f32>(0.72, 0.78, 0.88), u.shadow_tint.rgb, u.shadow_tint.a);
    let surface = mix(vec3<f32>(0.925, 0.94, 0.965), min(u.shadow_tint.rgb * 1.18, vec3<f32>(1.0, 1.0, 1.0)), u.shadow_tint.a * 0.2);
    let shade = (1.0 - exp(-depth / 30.0)) * 0.32 + (1.0 - lit) * 0.6 * exp(-depth / 5.0);
    var color = mix(surface, shadow, saturate(shade));
    color = mix(color, vec3<f32>(0.975, 0.982, 0.995), exp(-depth * 0.9) * 0.6);
    color += vec3<f32>((hash(floor(in.position.xy)) - 0.5) * 0.02);

    let seed = hash(floor(in.position.xy * 0.5) + vec2<f32>(31.7));
    if (seed > 0.992) {
        let twinkle = pow(saturate(sin(u.time * (0.6 + seed * 1.8) + seed * 90.0)), 28.0);
        color = mix(color, vec3<f32>(1.0, 1.0, 1.0), twinkle * exp(-depth / 7.0));
    }

    color *= u.light.rgb;
    return vec4<f32>(color * coverage, coverage);
}
