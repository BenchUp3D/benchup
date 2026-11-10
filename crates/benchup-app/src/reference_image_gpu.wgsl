struct Uniform {
    matrix: mat4x4<f32>,
    // fitted width/height and frame width/height
    size: vec4<f32>,
    // content anchor xy, pan xy
    placement: vec4<f32>,
    // cos, sin, horizontal flip, vertical flip
    rotation: vec4<f32>,
    // opacity, brightness, contrast, grayscale
    appearance: vec4<f32>,
    // two sided, srgb target
    flags: vec4<f32>,
    depth: vec4<f32>,
    section: vec4<f32>,
}
@group(0) @binding(0) var<uniform> settings: Uniform;
@group(0) @binding(1) var picture: texture_2d<f32>;
@group(0) @binding(2) var filtering: sampler;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) frame: vec2<f32>,
    @location(1) local: vec3<f32>,
}
@vertex
fn vertex_main(@location(0) position: vec3<f32>, @location(1) frame: vec2<f32>) -> Output {
    var out: Output;
    out.position = settings.matrix * vec4<f32>(position, 1.0);
    if settings.depth.w != 0.0 {
        let z = out.position.z / out.position.w;
        out.position.z = (settings.depth.z + 0.8 * ((z - settings.depth.x) / settings.depth.y)) * out.position.w;
    }
    out.frame = frame;
    out.local = position;
    return out;
}
@fragment
fn fragment_main(input: Output, @builtin(front_facing) front: bool) -> @location(0) vec4<f32> {
    let extent = settings.size.xy;
    let top_left = (vec2<f32>(1.0) - extent) * settings.placement.xy + settings.placement.zw;
    let center = top_left + extent * 0.5;
    // Rotate in physical frame units so portrait and landscape pictures keep
    // their proportions; the rectangular frame itself performs GPU clipping.
    let delta = (input.frame - center) * settings.size.zw;
    let rotated = vec2<f32>(settings.rotation.x * delta.x + settings.rotation.y * delta.y,
        -settings.rotation.y * delta.x + settings.rotation.x * delta.y);
    var uv = rotated / (extent * settings.size.zw) + vec2<f32>(0.5);
    uv = (uv - vec2<f32>(0.5)) * settings.rotation.zw + vec2<f32>(0.5);
    // Sample before discard for uniform derivatives on WebGL/WebGPU.
    let sampled = textureSample(picture, filtering, uv);
    if settings.flags.x == 0.0 && !front { discard; }
    if any(input.local > settings.section.xyz + vec3<f32>(0.00000001)) { discard; }
    if any(uv < vec2<f32>(0.0)) || any(uv > vec2<f32>(1.0)) { discard; }
    var rgb = sampled.rgb;
    if settings.appearance.w != 0.0 { rgb = vec3<f32>(dot(rgb, vec3<f32>(0.2126, 0.7152, 0.0722))); }
    rgb = clamp(((rgb - vec3<f32>(0.5)) * settings.appearance.z + vec3<f32>(0.5)) * settings.appearance.y, vec3<f32>(0.0), vec3<f32>(1.0));
    if settings.flags.y != 0.0 {
        rgb = select(rgb / 12.92, pow((rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), rgb > vec3<f32>(0.04045));
    }
    let alpha = sampled.a * settings.appearance.x;
    if alpha < 0.001 { discard; }
    return vec4<f32>(rgb * alpha, alpha);
}
