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
    let rgb = finish_color(color.rgb, color.a, p,
        vec4<f32>(input.finish.xyz, input.finish.w * (1.0 - shadow * 0.65)));
    if input.srgb != 0u { return vec4<f32>(finish_srgb_to_linear(rgb), color.a); }
    return vec4<f32>(rgb, color.a);
}

@fragment
fn fragment_material_faces(input: EdgeVertex) -> @location(0) vec4<f32> {
    let color = shade_finish(input, input.color, 0.0);
    let coverage = stroke_coverage(input);
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
