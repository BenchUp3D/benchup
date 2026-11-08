struct Camera {
    matrix: mat4x4<f32>,
    plane_matrix: mat4x4<f32>,
    eye: vec4<f32>,
    light: vec4<f32>,
    direction: vec4<f32>,
    viewport: vec4<f32>,
    flags: vec4<f32>,
    interaction: vec4<u32>,
    face_selected: vec4<f32>,
    face_hovered: vec4<f32>,
    face_inactive: vec4<f32>,
    edge_selected: vec4<f32>,
    edge_hovered: vec4<f32>,
    edge_inactive: vec4<f32>,
    edge_muted: vec4<f32>,
    edge_normal: vec4<f32>,
}
@group(0) @binding(0) var<uniform> camera: Camera;
@group(0) @binding(1) var interaction_states: texture_2d<u32>;
fn interaction_state(pair: vec2<u32>) -> u32 {
    let owner=textureLoad(interaction_states,vec2<i32>(i32(pair.x%512u),i32(pair.x/512u)),0).x;
    let own=textureLoad(interaction_states,vec2<i32>(i32(pair.y%512u),i32(pair.y/512u)),0).x;
    var state=owner|own;
    if camera.interaction.x != 0u && camera.interaction.x == pair.y {state|=2u;}
    if (state&8u)!=0u {state|=2u;}
    return state;
}
// Keep face IDs and exact depth bits in an integer color target: the GLSL
// backend cannot translate textureLoad from a depth texture.
@group(1) @binding(0) var visible_faces: texture_2d<u32>;
struct Output {
    @builtin(position) position: vec4<f32>,
    @location(0) color: vec4<f32>,
    @location(1) lateral: f32,
    @location(2) center: vec3<f32>,
    @location(3) @interpolate(flat) faces: vec4<u32>,
}
fn target_color(color: vec4<f32>) -> vec4<f32> {
    if camera.flags.x == 0.0 { return color; }
    let linear = select(color.rgb / 12.92,
        pow((color.rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)), color.rgb > vec3<f32>(0.04045));
    return vec4<f32>(linear, color.a);
}
@vertex
fn face_vertex(@location(0) position: vec3<f32>, @location(1) normal: vec3<f32>,
    @location(2) anchor: vec3<f32>, @location(3) color: vec4<f32>, @location(4) face_id: u32, @location(5) interaction: vec2<u32>) -> Output {
    var n = normal;
    var toward_eye = camera.eye.xyz - anchor;
    if camera.flags.y != 0.0 {
        // Perspective division is disabled, but lighting still faces the eye.
        toward_eye = camera.direction.xyz;
    }
    if dot(n, toward_eye) < 0.0 { n = -n; }
    let state=interaction_state(interaction);
    var material=color;
    if (state&4u)==0u && (state&8u)==0u {material=camera.face_inactive;}
    else if (state&1u)!=0u {material=camera.face_selected;}
    else if (state&2u)!=0u {material=camera.face_hovered;}
    let brightness = 0.62 + 0.38 * max(dot(n, camera.light.xyz), 0.0);
    let shaded = vec4<f32>(floor(material.rgb * brightness * 255.0 + vec3<f32>(0.5)) / 255.0, material.a);
    var output: Output;
    output.position = camera.matrix * vec4<f32>(position,1.0);
    if camera.interaction.y!=0u && (state&4u)==0u {output.position=vec4<f32>(2.0,2.0,2.0,1.0);}
    output.color = target_color(shaded);
    output.lateral = 0.0;
    output.center = vec3<f32>(0.0);
    output.faces = vec4<u32>(face_id,0u,0u,0u);
    return output;
}
// Offset depth only across the actual incident face, never its extension past
// a silhouette. Using homogeneous planes also works at near-plane crossings.
fn face_depth_offset(a: vec3<f32>, normal: vec3<f32>, inward: vec3<f32>, clip: vec4<f32>, offset: vec2<f32>) -> vec2<f32> {
    if dot(normal,normal) == 0.0 { return vec2<f32>(0.0,0.0); }
    let delta = camera.matrix * vec4<f32>(inward,0.0);
    let direction = delta.xy * clip.w - clip.xy * delta.w;
    if dot(direction,offset) < -0.0000001 { return vec2<f32>(0.0,0.0); }
    let plane = camera.plane_matrix * vec4<f32>(normal,-dot(normal,a));
    if abs(plane.z) < 0.0000000001 { return vec2<f32>(0.0,0.0); }
    return vec2<f32>(-dot(plane.xy,offset)/plane.z,1.0);
}
@vertex
fn edge_vertex(@location(0) world_a: vec3<f32>, @location(1) world_b: vec3<f32>,
    @location(2) normal0: vec3<f32>, @location(3) inward0: vec3<f32>,
    @location(4) normal1: vec3<f32>, @location(5) inward1: vec3<f32>,
    @location(6) normal2: vec3<f32>, @location(7) inward2: vec3<f32>,
    @location(8) normal3: vec3<f32>, @location(9) inward3: vec3<f32>,
    @location(10) color: vec4<f32>, @location(11) width: f32, @location(12) faces: vec4<u32>, @location(13) interaction: vec2<u32>,
    @builtin(vertex_index) vertex: u32) -> Output {
    var a = camera.matrix * vec4<f32>(world_a,1.0);
    var b = camera.matrix * vec4<f32>(world_b,1.0);
    var output: Output;
    let state=interaction_state(interaction);
    let stroke_active=(state&3u)!=0u;
    let outside=camera.interaction.y!=0u && (state&4u)==0u;
    var edge_color=color;
    if outside {edge_color=camera.edge_muted;}
    else if (state&4u)==0u && (state&8u)==0u {edge_color=camera.edge_inactive;}
    else if (state&16u)!=0u {edge_color=vec4<f32>(90.0/255.0,133.0/255.0,176.0/255.0,1.0);}
    else if (state&1u)!=0u {edge_color=camera.edge_selected;}
    else if (state&2u)!=0u {edge_color=camera.edge_hovered;}
    output.color = target_color(edge_color);
    output.center = vec3<f32>(0.0);
    output.faces = faces;
    output.lateral = 0.0;
    output.position = vec4<f32>(2.0,2.0,2.0,1.0);
    if camera.interaction.z==0u && camera.interaction.y==0u && !stroke_active {return output;}
    if a.z < 0.0 && b.z < 0.0 { return output; }
    if a.z < 0.0 { a = mix(a,b,a.z/(a.z-b.z)); }
    if b.z < 0.0 { b = mix(b,a,b.z/(b.z-a.z)); }
    let screen_delta = (b.xy/b.w - a.xy/a.w) * camera.viewport.xy * vec2<f32>(0.5,-0.5);
    if dot(screen_delta,screen_delta) < 0.01 { return output; }
    let normal = normalize(vec2<f32>(-screen_delta.y,screen_delta.x));
    let fringe = (abs(normal.x) + abs(normal.y)) / camera.viewport.z;
    let stroke_width=select(select(width,3.0,stroke_active),1.0,outside);
    let radius = (stroke_width + fringe) * 0.5;
    let offsets = array<vec2<f32>,12>(
        vec2<f32>(0.0,1.0),vec2<f32>(0.0,0.0),vec2<f32>(1.0,1.0),
        vec2<f32>(1.0,1.0),vec2<f32>(0.0,0.0),vec2<f32>(1.0,0.0),
        vec2<f32>(0.0,0.0),vec2<f32>(0.0,-1.0),vec2<f32>(1.0,0.0),
        vec2<f32>(1.0,0.0),vec2<f32>(0.0,-1.0),vec2<f32>(1.0,-1.0));
    let selector = offsets[vertex];
    var clip = a;
    if selector.x > 0.5 { clip = b; }
    // WebGL2 cannot use linear (noperspective) interpolation. Carry clip-space
    // coordinates and cancel perspective interpolation in hidden_center instead.
    output.center = clip.xyz;
    let offset = normal * radius * selector.y * vec2<f32>(2.0,-2.0) / camera.viewport.xy;
    let first = face_depth_offset(world_a,normal0,inward0,clip,offset);
    let second = face_depth_offset(world_a,normal1,inward1,clip,offset);
    var depth_offset = 0.0;
    if first.y > 0.0 { depth_offset = first.x; }
    if second.y > 0.0 {
        depth_offset = second.x;
        if first.y > 0.0 { depth_offset = min(first.x,second.x); }
    }
    let third = face_depth_offset(world_a,normal2,inward2,clip,offset);
    let fourth = face_depth_offset(world_a,normal3,inward3,clip,offset);
    var bounded = first.y + second.y > 0.0;
    for(var i = 0u; i < 2u; i++) {
        let candidate = select(third,fourth,i == 1u);
        if candidate.y > 0.0 {
            depth_offset = select(candidate.x,min(depth_offset,candidate.x),bounded);
            bounded = true;
        }
    }
    let depth = clamp(clip.z/clip.w + depth_offset,0.0,1.0);
    let biased = bitcast<f32>(bitcast<u32>(depth) - min(1u,bitcast<u32>(depth)));
    output.position = vec4<f32>(clip.xy + offset * clip.w,biased * clip.w,clip.w);
    output.lateral = selector.y;
    return output;
}
fn coverage(input: Output) -> f32 {
    let gradient = max(fwidth(input.lateral),0.0001);
    let peak = min(1.0,2.0/gradient-1.0);
    return clamp((1.0-abs(input.lateral))/gradient,0.0,peak);
}
@fragment
fn fragment_main(input: Output) -> @location(0) vec4<f32> {
    let amount = coverage(input);
    if amount < 0.001 { discard; }
    return input.color * amount;
}
// A steep incident plane must not pull a hidden edge through another object.
// Test the unexpanded centerline as well as the stroke footprint. Incident-face
// IDs avoid falsely clipping a visible boundary against its own pixel sample.
fn hidden_center(input: Output) -> bool {
    // Fragment position.w is reciprocal clip w. Multiplying the perspective-
    // interpolated center by it recovers screen-linear centerline NDC.
    let center = input.center * input.position.w;
    let size = vec2<i32>(textureDimensions(visible_faces));
    let uv = center.xy * vec2<f32>(0.5,-0.5) + vec2<f32>(0.5);
    let pixel = clamp(vec2<i32>(uv * vec2<f32>(size)),vec2<i32>(0),size-vec2<i32>(1));
    let visible = textureLoad(visible_faces,pixel,0).xy;
    let face = visible.x;
    if face == 0u || any(input.faces == vec4<u32>(face)) { return false; }
    return bitcast<f32>(visible.y) < center.z - 0.00000024;
}
@fragment
fn fragment_visibility(input: Output) -> @location(0) vec2<u32> {
    return vec2<u32>(input.faces.x,bitcast<u32>(input.position.z));
}
@fragment
fn edge_color(input: Output) -> @location(0) vec4<f32> {
    let amount = coverage(input);
    if amount < 0.001 || hidden_center(input) { discard; }
    return input.color * amount;
}
@fragment
fn edge_depth(input: Output) -> @location(0) vec4<f32> {
    let amount = coverage(input);
    if input.position.z <= 0.0 || input.position.z >= 1.0 || input.color.a < 1.0 || amount < 0.001 || hidden_center(input) { discard; }
    return vec4<f32>(0.0);
}
