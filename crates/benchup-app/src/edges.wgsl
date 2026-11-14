struct EdgeVertex {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) lateral: f32,
    @location(2) shadow_color: vec4<f32>,
    @location(3) uv: vec2<f32>,
    @location(4) dash_phase: f32,
    @location(5) grain: vec4<f32>,
    @location(6) @interpolate(flat) finish: vec4<f32>,
    @location(7) @interpolate(flat) srgb: u32,
}

fn edge_vertex(position: vec3<f32>, color: vec4<f32>, lateral: f32, shadow_color: vec4<f32>, dash_phase: f32, grain: vec4<f32>, finish: vec4<f32>, srgb: u32) -> EdgeVertex {
    var output: EdgeVertex;
    output.position = vec4<f32>(position, 1.0);
    output.color = color;
    output.lateral = lateral;
    output.dash_phase = dash_phase;
    output.shadow_color = shadow_color;
    output.grain = grain;
    output.finish = finish;
    output.srgb = srgb;
    output.uv = position.xy * vec2<f32>(0.5, -0.5) + vec2<f32>(0.5);
    return output;
}

fn edge_vertex_srgb(position: vec3<f32>, color: vec4<f32>, lateral: f32,
    shadow_color: vec4<f32>, dash_phase: f32) -> EdgeVertex {
    let linear = finish_srgb_to_linear(color.rgb);
    let shadow_linear = finish_srgb_to_linear(shadow_color.rgb);
    return edge_vertex(position, vec4<f32>(linear, color.a), lateral,
        vec4<f32>(shadow_linear, shadow_color.a), dash_phase, vec4<f32>(0.0), vec4<f32>(0.0), 0u);
}

@vertex
fn vertex_main(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>, @location(2) lateral: f32,
    @location(3) shadow_color: vec4<f32>, @location(4) dash_phase: f32) -> EdgeVertex {
    return edge_vertex(position, color, lateral, shadow_color, dash_phase, vec4<f32>(0.0), vec4<f32>(0.0), 0u);
}

@vertex
fn vertex_main_srgb(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>, @location(2) lateral: f32,
    @location(3) shadow_color: vec4<f32>, @location(4) dash_phase: f32) -> EdgeVertex {
    return edge_vertex_srgb(position, color, lateral, shadow_color, dash_phase);
}

@vertex
fn vertex_material(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>, @location(2) lateral: f32,
    @location(3) shadow_color: vec4<f32>, @location(4) dash_phase: f32,
    @location(5) grain: vec4<f32>, @location(6) finish: vec4<f32>) -> EdgeVertex {
    return edge_vertex(position, color, lateral, shadow_color, dash_phase, grain, finish, 0u);
}

@vertex
fn vertex_material_srgb(@location(0) position: vec3<f32>, @location(1) color: vec4<f32>, @location(2) lateral: f32,
    @location(3) shadow_color: vec4<f32>, @location(4) dash_phase: f32,
    @location(5) grain: vec4<f32>, @location(6) finish: vec4<f32>) -> EdgeVertex {
    // Finish shading uses the same color space as in the retained shader.
    if grain.w > 0.0 {
        return edge_vertex(position, color, lateral, shadow_color, dash_phase, grain, finish, 1u);
    }
    return edge_vertex_srgb(position, color, lateral, shadow_color, dash_phase);
}

fn stroke_coverage(input: EdgeVertex) -> f32 {
    // The CPU expands the strip by this pixel footprint, including diagonals.
    // Limit subpixel strokes to their true coverage rather than darkening them.
    let gradient = max(fwidth(input.lateral), 0.0001);
    let peak = min(1.0, 2.0 / gradient - 1.0);
    let coverage = clamp((1.0 - abs(input.lateral)) / gradient, 0.0, peak);
    // The same discard mask is used for color and depth, so gaps cannot
    // occlude other geometry. Negative phase marks ordinary solid strokes.
    let dash = select(1.0, 0.0, input.dash_phase >= 0.0 && fract(input.dash_phase) > 0.5);
    return coverage * dash;
}

@fragment
fn fragment_edge_depth(input: EdgeVertex) -> @location(0) vec4<f32> {
    // Resolve the entire visible footprint, including the antialias fringe.
    // Otherwise rear selection strokes leak through the fringe of nearer edges.
    // Screen overlays and translucent strokes do not occlude world geometry.
    let coverage = stroke_coverage(input);
    if input.position.z <= 0.0 || input.position.z >= 1.0 ||
       input.color.a < 1.0 || coverage < 0.001 { discard; }
    return vec4<f32>(0.0);
}

@fragment
fn fragment_main(input: EdgeVertex) -> @location(0) vec4<f32> {
    let coverage = stroke_coverage(input);
    if coverage < 0.001 { discard; }
    return input.color * coverage;
}

fn shade_finish(input: EdgeVertex, color: vec4<f32>, shadow: f32) -> vec4<f32> {
    let p = input.grain.xyz / max(input.grain.w, 0.000000001);
    let dx = dpdx(input.grain);
    let dy = dpdy(input.grain);
    let px = dpdx(p.xy);
    let py = dpdy(p.xy);
    let footprint = fwidth(p);
    if all(input.finish.xyz == vec3<f32>(254.0 / 255.0, 1.0, 254.0 / 255.0)) {
        let coverage = grain_direction_coverage(input.grain, dx, dy, input.finish.w * 8.0);
        if coverage < 0.001 { discard; }
        var ink = input.color.rgb;
        if input.srgb != 0u { ink = finish_srgb_to_linear(ink); }
        return vec4<f32>(ink, input.color.a) * coverage;
    }
    let rgb = finish_color_with_derivatives(color.rgb, color.a, p,
        vec4<f32>(input.finish.xyz, input.finish.w * (1.0 - shadow * 0.65)), px, py, footprint);
    if input.srgb != 0u { return vec4<f32>(finish_srgb_to_linear(rgb), color.a); }
    return vec4<f32>(rgb, color.a);
}

@fragment
fn fragment_material_faces(input: EdgeVertex) -> @location(0) vec4<f32> {
    let coverage = stroke_coverage(input);
    let color = shade_finish(input, input.color, 0.0);
    if coverage < 0.001 { discard; }
    return color * coverage;
}

// Multisample a scalar shadow mask, then shade the original single-sample
// receiving faces. Neither face silhouettes nor viewport strokes are resolved.
@group(0) @binding(0) var shadow_mask: texture_2d<f32>;
@group(0) @binding(1) var shadow_sampler: sampler;

@fragment
fn fragment_mask_base(input: EdgeVertex) -> @location(0) vec4<f32> {
    // Shadow receivers are replaced by their lit/shaded partitions in the
    // coverage pass. Never put their overlapping original depth in this mask.
    if input.shadow_color.a > 0.0 { discard; }
    return vec4<f32>(0.0, 0.0, 0.0, 1.0);
}

@fragment
fn fragment_mask_shadow(input: EdgeVertex) -> @location(0) vec4<f32> {
    return input.color;
}

@fragment
fn fragment_shadow_faces(input: EdgeVertex) -> @location(0) vec4<f32> {
    let coverage = textureSample(shadow_mask, shadow_sampler, input.uv).r;
    return mix(input.color, input.shadow_color, coverage * select(0.0, 1.0, input.shadow_color.a > 0.0));
}

@fragment
fn fragment_material_shadow_faces(input: EdgeVertex) -> @location(0) vec4<f32> {
    let coverage = textureSample(shadow_mask, shadow_sampler, input.uv).r;
    let amount = coverage * select(0.0, 1.0, input.shadow_color.a > 0.0);
    return shade_finish(input, mix(input.color, input.shadow_color, amount), amount);
}

// Match grain_variation's wrapping u64 hash using portable u32 limbs. WebGL
// has no 64-bit integers; keeping the hash exact prevents pattern changes
// between backends, previews, zoom levels and the bounded CPU fallback.
fn grain_mul64(a: vec2<u32>, b: vec2<u32>) -> vec2<u32> {
    let a0 = a.x & 65535u;
    let a1 = a.x >> 16u;
    let b0 = b.x & 65535u;
    let b1 = b.x >> 16u;
    let p0 = a0 * b0;
    let p1 = a0 * b1;
    let p2 = a1 * b0;
    let carry = (p0 >> 16u) + (p1 & 65535u) + (p2 & 65535u);
    return vec2<u32>((p0 & 65535u) | (carry << 16u),
        a.y * b.x + a.x * b.y + a1 * b1 + (p1 >> 16u) + (p2 >> 16u) + (carry >> 16u));
}
fn grain_signed64(value: i32) -> vec2<u32> {
    return vec2<u32>(bitcast<u32>(value), select(0u, 4294967295u, value < 0));
}
fn grain_variation(column: i32, row: i32) -> f32 {
    let key = grain_mul64(grain_signed64(column), vec2<u32>(0x7f4a7c15u, 0x9e3779b9u)) ^
        grain_mul64(grain_signed64(row), vec2<u32>(0x1ce4e5b9u, 0xbf58476du));
    let shifted = vec2<u32>((key.x >> 30u) | (key.y << 2u), key.y >> 30u);
    let mixed = grain_mul64(key ^ shifted, vec2<u32>(0x133111ebu, 0x94d049bbu));
    let upper = (mixed.y ^ (mixed.y >> 31u)) >> 16u;
    return f32(upper) / 32767.5 - 1.0;
}

// Perspective-correct screen displacement from this fragment to a fixed
// surface anchor. Packed coordinates and reciprocal W are affine in pixels.
fn grain_pixel_delta(anchor_uv: vec2<f32>, packed: vec4<f32>, dx: vec4<f32>, dy: vec4<f32>) -> vec2<f32> {
    let x = dx.xy - anchor_uv * dx.w;
    let y = dy.xy - anchor_uv * dy.w;
    let det = x.x * y.y - x.y * y.x;
    let safe_det = select(-1e-25, 1e-25, det >= 0.0);
    let divisor = select(safe_det, det, abs(det) > 1e-25);
    let delta = anchor_uv * packed.w - packed.xy;
    return vec2<f32>(y.y * delta.x - y.x * delta.y, x.x * delta.y - x.y * delta.x) / divisor;
}
fn grain_direction_coverage(packed: vec4<f32>, dx: vec4<f32>, dy: vec4<f32>, width: f32) -> f32 {
    let p = packed.xy / max(packed.w, 1e-20);
    let end_grain = packed.z / max(packed.w, 1e-20) > 0.5;
    let across = select(0.66, 1.0, end_grain);
    let nearest_row = i32(clamp(floor(p.y / across), -1000000.0, 1000000.0));
    var coverage = 0.0;
    for (var r = -1; r <= 1; r++) {
        let row = nearest_row + r;
        var pitch = 1.35;
        var offset = 0.0;
        var y = f32(row) * across;
        if end_grain {
            pitch = 1.0;
        } else if row != 0 {
            pitch += 0.09 * grain_variation(row, 11);
            offset = 0.54 * grain_variation(row, 29);
            y += 0.13 * grain_variation(row, 47);
        }
        let nearest_column = i32(clamp(floor((p.x - offset) / pitch), -1000000.0, 1000000.0));
        for (var c = -1; c <= 1; c++) {
            let column = nearest_column + c;
            var jitter = 0.0;
            if !end_grain && (row != 0 || column != 0) { jitter = 0.06 * grain_variation(column, row); }
            let anchor = vec2<f32>(f32(column) * pitch + offset + jitter, y);
            if end_grain {
                let q = grain_pixel_delta(anchor, packed, dx, dy);
                let rotated = abs(vec2<f32>(0.92387953*q.x + 0.38268343*q.y, -0.38268343*q.x + 0.92387953*q.y));
                let edge = max(max(rotated.x, rotated.y), (rotated.x + rotated.y) * 0.70710678);
                coverage = max(coverage, select(0.0, 1.0, edge <= width * 0.65 * 0.92387953));
            } else {
                let half_length = 0.40 + 0.06 * grain_variation(column + 31, row);
                let a = grain_pixel_delta(anchor - vec2<f32>(half_length, 0.0), packed, dx, dy);
                let b = grain_pixel_delta(anchor + vec2<f32>(half_length, 0.0), packed, dx, dy);
                let length = max(distance(a, b), 1e-8);
                let direction = (b - a) / length;
                let normal = vec2<f32>(-direction.y, direction.x);
                let along = -dot(a, direction);
                let fringe = max(abs(normal.x) + abs(normal.y), 1e-8);
                let amount = clamp(((width + fringe) * 0.5 - abs(dot(a, normal))) / fringe, 0.0, min(1.0, width / fringe));
                coverage = max(coverage, select(0.0, amount, along >= 0.0 && along <= length));
            }
        }
    }
    return coverage;
}
