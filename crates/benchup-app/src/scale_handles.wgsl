struct Uniform {
    matrix: mat4x4<f32>,
    lo: vec4<f32>,
    hi: vec4<f32>,
    viewport: vec4<f32>,
    origin: vec4<f32>,
}
@group(0) @binding(0) var<uniform> u: Uniform;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) uv: vec2<f32>,
    @location(1) @interpolate(flat) kind: u32,
}
@vertex fn vertex_main(@builtin(vertex_index) vertex: u32) -> Output {
    let instance = vertex / 6u;
    let corners = array<vec2<f32>, 6>(vec2(-1., -1.), vec2(1., -1.), vec2(-1., 1.), vec2(-1., 1.), vec2(1., -1.), vec2(1., 1.));
    let uv = corners[vertex % 6u];
    var out: Output;
    out.uv = uv;
    out.kind = 0u;
    var clip = vec4<f32>(2., 2., 2., 1.);
    if instance < 28u {
        let c = vec3<f32>(f32(instance % 3u), f32((instance / 3u) % 3u), f32(instance / 9u));
        let degenerate = abs(u.hi.xyz - u.lo.xyz) <= vec3<f32>(0.0000001);
        // WebGL 2 cannot use the GLSL mix emitted for a boolean-vector select.
        let hidden = instance == 13u
            || (degenerate.x && c.x != 1.)
            || (degenerate.y && c.y != 1.)
            || (degenerate.z && c.z != 1.);
        if (instance < 27u && !hidden) || (instance == 27u && u.viewport.w > 0.) {
            var point = mix(u.lo.xyz, u.hi.xyz, c * 0.5);
            if instance == 27u { point = u.origin.xyz; out.kind = 2u; }
            if f32(instance) == u.viewport.z { out.kind = 1u; }
            clip = u.matrix * vec4<f32>(point, 1.);
            let radius = select(4.5, 6., out.kind > 0u);
            clip = vec4<f32>(clip.xy + uv * radius * 2. / u.viewport.xy * clip.w, clip.zw);
        }
    } else {
        out.kind = 3u;
        let edge = instance - 28u;
        let axis = edge / 4u;
        let other = (axis + 1u) % 3u;
        let third = (axis + 2u) % 3u;
        var a = u.lo.xyz;
        a[other] = select(u.lo[other], u.hi[other], (edge % 4u & 1u) != 0u);
        a[third] = select(u.lo[third], u.hi[third], (edge % 4u & 2u) != 0u);
        var b = a;
        b[axis] = u.hi[axis];
        let ca = u.matrix * vec4<f32>(a, 1.);
        let cb = u.matrix * vec4<f32>(b, 1.);
        if ca.w > 0. && cb.w > 0. {
            let delta = (cb.xy / cb.w - ca.xy / ca.w) * u.viewport.xy;
            let normal = vec2<f32>(-delta.y, delta.x) / max(length(delta), 0.001);
            clip = mix(ca, cb, (uv.x + 1.) * 0.5);
            clip = vec4<f32>(clip.xy + normal * uv.y / u.viewport.xy * clip.w, clip.zw);
        }
    }
    out.position = clip;
    return out;
}
@fragment fn fragment_main(in: Output) -> @location(0) vec4<f32> {
    if in.kind == 3u { return vec4<f32>(0.85, 0.58, 0.01, 1.); }
    if max(abs(in.uv.x), abs(in.uv.y)) > 0.75 { return vec4<f32>(0.12, 0.09, 0.01, 1.); }
    if in.kind == 2u { return vec4<f32>(0.2, 1., 0.3, 1.); }
    if in.kind == 1u { return vec4<f32>(1., 0.95, 0.45, 1.); }
    return vec4<f32>(1., 0.78, 0.015, 1.);
}
