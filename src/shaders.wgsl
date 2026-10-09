// GPU rasterizer for isotop's screen-space display list. Every formula here mirrors the CPU
// rasterizer in render.rs so both backends draw the same frame.

struct Globals {
    size: vec2<f32>,
    // Scene depth mapped to [0.01, 0.99]: (minimum, 1 / range).
    depth: vec2<f32>,
    // Pressure haze in 0-255 units, and animation time in w.
    haze: vec4<f32>,
    // x: the backdrop, 0 dusk gradient, 1 flat space, 2 sea gradient, 3 black.
    sky: vec4<f32>,
};

@group(0) @binding(0) var<uniform> globals: Globals;

const NONE: u32 = 0xffffffffu;

struct Targets {
    @location(0) color: vec4<f32>,
    @location(1) pick: u32,
};

fn clip(position: vec2<f32>) -> vec2<f32> {
    return vec2<f32>(position.x / globals.size.x * 2.0 - 1.0, 1.0 - position.y / globals.size.y * 2.0);
}

fn normalized(depth: f32) -> f32 {
    return clamp(0.01 + 0.98 * (depth - globals.depth.x) * globals.depth.y, 0.0, 1.0);
}

fn profile(t: f32, phase: f32) -> f32 {
    let time = globals.haze.w;
    return 0.5 + 0.5 * sin(t * 7.3 + time * 0.11 + phase) * cos(t * 2.9 - time * 0.07 + phase * 1.7);
}

@vertex
fn sky_vertex(@builtin(vertex_index) index: u32) -> @builtin(position) vec4<f32> {
    let corner = vec2<f32>(f32((index << 1u) & 2u), f32(index & 2u));
    return vec4<f32>(corner * 2.0 - 1.0, 0.0, 1.0);
}

@fragment
fn sky_fragment(@builtin(position) position: vec4<f32>) -> Targets {
    let x = floor(position.x) / globals.size.x;
    let y = floor(position.y) / globals.size.y;
    var base = vec3<f32>(5.0, 8.0, 18.0);
    if globals.sky.x < 0.5 {
        base = vec3<f32>(8.0 + y * 6.0, 13.0 + y * 7.0, 25.0 + y * 8.0);
    } else if globals.sky.x > 2.5 {
        base = vec3<f32>(0.0);
    } else if globals.sky.x > 1.5 {
        base = vec3<f32>(24.0 - y * 18.0, 72.0 - y * 50.0, 96.0 - y * 58.0);
    }
    let density = profile(x, 0.0) * profile(y, 1.9);
    let color = min(base + globals.haze.xyz * density, vec3<f32>(255.0));
    return Targets(vec4<f32>(floor(color) / 255.0, 1.0), NONE);
}

struct Vertex {
    @location(0) position: vec2<f32>,
    @location(1) depth: f32,
    @location(2) color: vec4<f32>,
    @location(3) pick: u32,
};

struct Varying {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) @interpolate(flat) pick: u32,
};

@vertex
fn geometry_vertex(vertex: Vertex) -> Varying {
    return Varying(vec4<f32>(clip(vertex.position), normalized(vertex.depth), 1.0), vertex.color, vertex.pick);
}

@fragment
fn geometry_fragment(input: Varying) -> Targets {
    return Targets(input.color, input.pick);
}

const CORNERS = array<vec2<f32>, 6>(
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, -1.0), vec2<f32>(1.0, 1.0),
    vec2<f32>(-1.0, -1.0), vec2<f32>(1.0, 1.0), vec2<f32>(-1.0, 1.0),
);

struct Ball {
    @location(0) center: vec3<f32>,
    @location(1) radius: f32,
    @location(2) world: f32,
    @location(3) color: vec4<f32>,
    @location(4) pick: u32,
    @location(5) selected: u32,
};

struct BallVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) center: vec3<f32>,
    @location(1) @interpolate(flat) radius: f32,
    @location(2) @interpolate(flat) world: f32,
    @location(3) @interpolate(flat) color: vec4<f32>,
    @location(4) @interpolate(flat) pick: u32,
    @location(5) @interpolate(flat) selected: u32,
};

struct BallTargets {
    @location(0) color: vec4<f32>,
    @location(1) pick: u32,
    @builtin(frag_depth) depth: f32,
};

@vertex
fn ball_vertex(@builtin(vertex_index) index: u32, ball: Ball) -> BallVarying {
    let reach = ball.radius + 4.0;
    let position = ball.center.xy + CORNERS[index] * reach;
    return BallVarying(
        vec4<f32>(clip(position), 0.5, 1.0),
        ball.center, ball.radius, ball.world, ball.color, ball.pick, ball.selected,
    );
}

@fragment
fn ball_fragment(input: BallVarying) -> BallTargets {
    // The CPU rasterizer samples at integer pixel coordinates; do the same.
    let n = (floor(input.position.xy) - input.center.xy) / input.radius;
    let d = dot(n, n);
    let outer = select(input.radius + 1.0, input.radius + 3.0, input.selected != 0u) / input.radius;
    var out: BallTargets;
    if d <= 1.0 {
        let nz = sqrt(1.0 - d);
        let light = min(0.25 + max(-n.x * 0.4 - n.y * 0.5 + nz * 0.65, 0.0), 1.2);
        let color = clamp(floor(input.color.rgb * 255.0 * light), vec3<f32>(0.0), vec3<f32>(255.0));
        out = BallTargets(vec4<f32>(color / 255.0, 1.0), input.pick, normalized(input.center.z + nz * input.world));
    } else if input.selected != 0u && d <= outer * outer {
        out = BallTargets(vec4<f32>(237.0, 220.0, 154.0, 255.0) / 255.0, input.pick, normalized(input.center.z));
    } else {
        discard;
    }
    return out;
}

struct Glow {
    @location(0) center: vec2<f32>,
    @location(1) radius: f32,
    @location(2) strength: f32,
    @location(3) color: vec4<f32>,
};

struct GlowVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) center: vec2<f32>,
    @location(1) @interpolate(flat) radius: f32,
    @location(2) @interpolate(flat) strength: f32,
    @location(3) @interpolate(flat) color: vec4<f32>,
};

@vertex
fn glow_vertex(@builtin(vertex_index) index: u32, glow: Glow) -> GlowVarying {
    let position = glow.center + CORNERS[index] * (glow.radius + 1.0);
    return GlowVarying(vec4<f32>(clip(position), 0.5, 1.0), glow.center, glow.radius, glow.strength, glow.color);
}

@fragment
fn glow_fragment(input: GlowVarying) -> Targets {
    let d = length(input.position.xy - input.center) / input.radius;
    if d >= 1.0 {
        discard;
    }
    let weight = input.strength * (1.0 - d) * (1.0 - d);
    return Targets(vec4<f32>(input.color.rgb, weight), NONE);
}

// Glyph cells are 7 by 14 font pixels; font pixel (column, row) is bit row * 7 + column of the
// 128-bit mask, split little-endian across four words.
const GLYPH = vec2<f32>(7.0, 14.0);

struct Glyph {
    @location(0) origin: vec2<f32>,
    @location(1) scale: f32,
    @location(2) color: vec4<f32>,
    @location(3) bits: vec4<u32>,
};

struct GlyphVarying {
    @builtin(position) position: vec4<f32>,
    @location(0) @interpolate(flat) origin: vec2<f32>,
    @location(1) @interpolate(flat) scale: f32,
    @location(2) @interpolate(flat) color: vec4<f32>,
    @location(3) @interpolate(flat) bits: vec4<u32>,
};

@vertex
fn glyph_vertex(@builtin(vertex_index) index: u32, glyph: Glyph) -> GlyphVarying {
    let position = glyph.origin + (CORNERS[index] * 0.5 + 0.5) * GLYPH * glyph.scale;
    return GlyphVarying(vec4<f32>(clip(position), 0.5, 1.0), glyph.origin, glyph.scale, glyph.color, glyph.bits);
}

@fragment
fn glyph_fragment(input: GlyphVarying) -> Targets {
    let cell = floor((floor(input.position.xy) - input.origin) / input.scale);
    if any(cell < vec2<f32>(0.0)) || any(cell >= GLYPH) {
        discard;
    }
    let bit = u32(cell.y) * 7u + u32(cell.x);
    if ((input.bits[bit / 32u] >> (bit % 32u)) & 1u) == 0u {
        discard;
    }
    return Targets(vec4<f32>(input.color.rgb, 1.0), NONE);
}
