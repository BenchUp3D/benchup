// Shared by the retained world renderer and the clipped/annotation renderer.
// Coordinates are in metres: part space for wood, face-plane space for glue.
fn finish_color(rgb: vec3<f32>, alpha: f32, p: vec3<f32>, finish: vec4<f32>) -> vec3<f32> {
    let dx = dpdx(p.xy);
    let dy = dpdy(p.xy);
    let footprint = fwidth(p);
    if all(finish == vec4<f32>(0.0)) { return rgb; }
    // Glue marks scale with zoom around fixed surface anchors. Keep a
    // readable minimum size and limit close-up overlap; camera derivatives
    // never rephase the surface coordinates (which would slide the pattern).
    if all(finish.xyz == vec3<f32>(1.0)) {
        // A translucent pale-yellow film preserves the receiving face color
        // and alpha. Both whole-face and contact-patch coatings use this path.
        let coated = mix(rgb, vec3<f32>(1.0, 0.94, 0.62) * alpha, 0.18);
        let det = dx.x * dy.y - dx.y * dy.x;
        if abs(det) < 1e-20 { return coated; }
        let u = vec2<f32>(dy.y, -dx.y) / det;
        let along_axis = normalize(u);
        let across_axis = vec2<f32>(-along_axis.y, along_axis.x);
        let glyph_scale = clamp(length(u) * 0.002, 0.25, 1.25);
        let pitch = vec2<f32>(0.135, 0.066);
        // Match density to the projected face, with room around each glyph.
        // Nested levels add/remove anchors without sliding surviving marks.
        // Allow finer levels when zoomed in as well as coarser levels far away.
        let needed = vec2<f32>(
            64.0 / length(u) + 12.0 * abs(dot(vec2<f32>(dx.x, dy.x), across_axis)),
            36.0 * length(vec2<f32>(dx.y, dy.y)));
        let stride = exp2(ceil(log2(needed / pitch)));
        // Evaluate every anchor whose pixel-sized curve can cover this
        // fragment, rather than clipping the curve to the nearest cell.
        let surface_along = dx * along_axis.x + dy * along_axis.y;
        let surface_across = dx * across_axis.x + dy * across_axis.y;
        let extent = (abs(surface_along) * 24.0 + abs(surface_across) * 6.0) * max(glyph_scale, 1.0);
        let pixel = 1.0 / glyph_scale;
        var outline = 0.0;
        var bead = 0.0;
        var row = ceil((p.y - extent.y) / (pitch.y * stride.y)) * stride.y;
        let last_row = floor((p.y + extent.y) / (pitch.y * stride.y)) * stride.y;
        loop {
            if row > last_row { break; }
            let stagger = fract(row * 0.37);
            var column = ceil(((p.x - extent.x) / pitch.x + stagger) / stride.x) * stride.x;
            let last_column = floor(((p.x + extent.x) / pitch.x + stagger) / stride.x) * stride.x;
            loop {
                if column > last_column { break; }
                let anchor = vec2<f32>((column - stagger) * pitch.x, row * pitch.y);
                let delta = p.xy - anchor;
                let screen = vec2<f32>(dy.y * delta.x - dy.x * delta.y,
                    dx.x * delta.y - dx.y * delta.x) / det;
                let q = vec2<f32>(dot(screen, along_axis), dot(screen, across_axis) * sign(det)) / glyph_scale;
                let x = clamp(q.x, -22.0, 22.0);
                let bend = 4.0 * sin(x * 0.14);
                let slope = 0.56 * cos(x * 0.14);
                let distance = length(vec2<f32>(q.x - x, (q.y - bend) / sqrt(1.0 + slope * slope)));
                outline = max(outline, 1.0 - smoothstep(0.7 * max(pixel, 1.0), 1.6 * max(pixel, 1.0), distance));
                bead = max(bead, 1.0 - smoothstep(0.25 * max(pixel, 1.0), max(pixel, 1.0), distance));
                // Exact early out: further overlaps cannot add coverage.
                if bead == 1.0 { break; }
                column += stride.x;
            }
            if bead == 1.0 { break; }
            row += stride.y;
        }
        // An amber rim keeps pale yellow visible on light wood, while the
        // independent bead color also remains visible on dark painted faces.
        let rimmed = mix(coated, vec3<f32>(0.63, 0.40, 0.06) * alpha, outline * 0.8);
        return mix(rimmed, vec3<f32>(0.98, 0.78, 0.22) * alpha, bead * 0.96);
    }
    // An off-center trunk creates long curved stripes on the sides and
    // growth rings on end grain. The same function continues across edges.
    let bend = 0.007 * sin(p.x * 16.0 + p.z * 13.0) + 0.003 * sin(p.x * 43.0);
    let radius = length(p.yz + vec2<f32>(0.035 + bend, 0.082));
    let phase = radius * 650.0 + 0.8 * sin(radius * 83.0);
    let ring = sin(phase);
    let latewood = smoothstep(0.33, 0.96, ring);
    let pores = sin(phase * 3.0 + 0.62 * sin(p.x * 79.0 + p.y * 113.0));
    let grain_px = dot(footprint, vec3<f32>(170.0, 650.0, 650.0));
    let grain = (0.052 - 0.15 * latewood) * (1.0 - smoothstep(0.38, 2.5, grain_px))
        + pores * 0.023 * (1.0 - smoothstep(0.17, 0.8, grain_px));
    // Coarser sanding leaves deeper, wider traces. Fade detail once it is
    // smaller than a pixel so it does not shimmer while the camera moves.
    let frequency = mix(315.0, 2650.0, finish.z);
    let scratch_axis = p.y + p.z * 0.68 + p.x * 0.045;
    let scratch_phase = scratch_axis * frequency + sin(p.x * 64.0 + p.z * 48.0);
    let scratch_px = dot(footprint, vec3<f32>(0.045, 1.0, 0.68)) * frequency;
    let scratches = (smoothstep(0.74, 1.0, sin(scratch_phase)) - 0.12) * 0.087
        * (1.0 - smoothstep(0.38, 2.0, scratch_px));
    let diffuse = rgb * (1.0 + grain * finish.x - scratches * finish.y);
    return min(vec3<f32>(alpha), mix(diffuse, vec3<f32>(alpha), finish.w));
}

fn finish_srgb_to_linear(rgb: vec3<f32>) -> vec3<f32> {
    return select(rgb / 12.92, pow((rgb + vec3<f32>(0.055)) / 1.055, vec3<f32>(2.4)),
        rgb > vec3<f32>(0.04045));
}
