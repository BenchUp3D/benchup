//! Woodworking cut lists. Geometry is already in world metres, including instances.
//! Measure blanks, orient them to stock grain, and project joinery onto layouts.
use benchup_core::tr;
use crate::cutting_layout::{self, Settings as LayoutSettings};
use benchup_core::{DVec3, Model};
use std::collections::{BTreeMap, BTreeSet};

#[cfg(test)]
#[path = "cutting_plan_tests.rs"]
mod tests;

type MaterialKey = (
    Option<benchup_core::MaterialId>,
    benchup_core::StockMaterial,
);

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum CutReport {
    CuttingPlan,
    WorkshopLabels,
}

#[derive(Debug)]
pub(crate) struct CutGroup {
    pub material: benchup_core::StockMaterial,
    pub material_id: Option<benchup_core::MaterialId>,
    /// Thickness, width, cut length in tenths of a millimetre. A short cut can
    /// be shorter than its width when aligned with other parts in its group.
    pub dimensions: [u64; 3],
    pub rectangular: bool,
    /// One named group hierarchy per piece; repeated names still count as pieces.
    pub parts: Vec<String>,
    pub(crate) grain: Grain,
    pub(crate) joinery: BTreeMap<usize, Vec<JointLine>>,
}

#[derive(Debug)]
pub(crate) struct CuttingPlan {
    pub groups: Vec<CutGroup>,
    pub skipped: usize,
    pub(crate) joinery_omitted: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Grain {
    #[default]
    Unassigned,
    AlongLength,
    ThroughThickness,
}

pub(crate) type JointLine = [[u64; 2]; 2];

/// Purchasing quantities calculated with the same layout used by the PDF.
pub(crate) struct MaterialRequirement {
    pub material_id: Option<benchup_core::MaterialId>,
    pub material: benchup_core::StockMaterial,
    pub quantity: usize,
    pub excluded: usize,
}

// Both the estimate and PDF must pack identical marks in identical order.
fn stock_layout(
    stock: &benchup_core::StockMaterial,
    groups: &[(usize, &CutGroup)],
    settings: LayoutSettings,
) -> cutting_layout::Layout {
    let settings = LayoutSettings {
        sheet_width_mm: stock.dimensions[1] as f64 / 10.,
        sheet_length_mm: stock.dimensions[2] as f64 / 10.,
        ..settings
    };
    let mut fitting = Vec::new();
    let mut excluded = Vec::new();
    for &(mark, group) in groups {
        let piece = cutting_layout::Piece {
            mark,
            length: group.dimensions[2],
            width: group.dimensions[1],
            quantity: group.parts.len(),
            grain_along_length: group.grain == Grain::AlongLength,
        };
        // Purchase the full bounding blank, even when the finished part has
        // notches, holes, or other cuts. Shape checks are informational only;
        // stock fit depends on the blank's dimensions.
        if group.dimensions[0] <= stock.dimensions[0]
            && !(settings.respect_grain && group.grain == Grain::ThroughThickness)
        {
            fitting.push(piece);
        } else {
            excluded.push(piece);
        }
    }
    let mut layout = cutting_layout::sheets(&fitting, settings);
    layout.oversized.extend(excluded);
    layout
}

#[derive(Clone, Copy)]
struct MeasuredPart {
    dimensions: [u64; 3],
    axes: [DVec3; 3],
    origin: DVec3,
    bounds: (DVec3, DVec3),
    rectangular: bool,
    grain: Grain,
}

impl CuttingPlan {
    /// An empty selection means all visible parts in the project.
    /// Selecting a face/edge includes its connected visible geometry in that scope.
    #[cfg(test)]
    pub fn collect(model: &Model, selected: &BTreeSet<u64>) -> Result<Self, String> {
        Self::collect_with_layout(model, selected, LayoutSettings::default())
    }

    pub fn collect_with_layout(
        model: &Model,
        selected: &BTreeSet<u64>,
        settings: LayoutSettings,
    ) -> Result<Self, String> {
        Self::collect_parts(model, selected, false, settings)
    }

    /// Estimates ignore unassigned geometry entirely, including omission counts.
    pub fn collect_assigned(
        model: &Model,
        selected: &BTreeSet<u64>,
        settings: LayoutSettings,
    ) -> Result<Self, String> {
        Self::collect_parts(model, selected, true, LayoutSettings { show_joinery: false, ..settings })
    }

    fn collect_parts(
        model: &Model,
        selected: &BTreeSet<u64>,
        assigned_only: bool,
        settings: LayoutSettings,
    ) -> Result<Self, String> {
        let requested = model.expand_entities(selected);
        let parents: BTreeMap<_, _> = model
            .objects
            .values()
            .flat_map(|object| object.entities.iter().map(move |id| (*id, object.id)))
            .collect();
        let mut scopes: BTreeMap<Option<u64>, BTreeSet<u64>> = BTreeMap::new();
        for id in model.edges.keys().chain(model.faces.keys()) {
            if model.is_hidden(*id) {
                continue;
            }
            scopes
                .entry(parents.get(id).copied())
                .or_default()
                .insert(*id);
        }
        let mut parts = Vec::new();
        let mut skipped = 0;
        for (scope, mut remaining) in scopes {
            let grain = if settings.respect_grain {
                std::iter::successors(scope, |id| parents.get(id).copied())
                    .find_map(|id| model.objects[&id].world_grain_axis())
            } else {
                None
            };
            let vertices: BTreeMap<_, _> = remaining
                .iter()
                .map(|id| (*id, entity_vertices(model, *id)))
                .collect();
            let mut adjacency: BTreeMap<u64, Vec<u64>> = BTreeMap::new();
            for (id, points) in &vertices {
                for point in points {
                    adjacency.entry(*point).or_default().push(*id);
                }
            }
            while let Some(first) = remaining.pop_first() {
                let mut component = BTreeSet::from([first]);
                let mut pending = vec![first];
                let mut seen_vertices = BTreeSet::new();
                while let Some(id) = pending.pop() {
                    for vertex in &vertices[&id] {
                        if seen_vertices.insert(*vertex) {
                            for adjacent in &adjacency[vertex] {
                                if remaining.remove(adjacent) {
                                    component.insert(*adjacent);
                                    pending.push(*adjacent);
                                }
                            }
                        }
                    }
                }
                let included = selected.is_empty() || !component.is_disjoint(&requested);
                // A nested part inherits its nearest assigned assembly material.
                let mut owner = scope;
                let mut material = None;
                while let Some(id) = owner {
                    if let Some(stock) = model.object_stock_material(&model.objects[&id]) {
                        material = Some((model.objects[&id].stock_material_id, stock.clone()));
                        break;
                    }
                    owner = parents.get(&id).copied();
                }
                let Some(material) = material else {
                    if included && !assigned_only {
                        skipped += 1;
                    }
                    continue;
                };
                let Some(part) = measure(model, &component, grain) else {
                    if included {
                        skipped += 1;
                    }
                    continue;
                };
                let mut path = Vec::new();
                let mut parent = scope;
                while let Some(id) = parent {
                    let name = model.objects[&id].name.trim();
                    if !automatic_group_name(name) && !name.is_empty() {
                        path.push(name.to_owned());
                    }
                    parent = parents.get(&id).copied();
                }
                path.reverse();
                parts.push((path.join("/"), part, included, material, scope));
            }
        }
        // Infer a common cutting direction from the longest visible blank in
        // each named group and thickness. Include unselected parts so selecting
        // just a short cut does not change its width/length assignment.
        let mut references: BTreeMap<_, &MeasuredPart> = BTreeMap::new();
        for (path, part, _, material, _) in &parts {
            if path.is_empty() || !part.rectangular || part.grain != Grain::Unassigned {
                continue;
            }
            let reference = references
                .entry((path.as_str(), part.dimensions[0], material))
                .or_insert(part);
            if part.dimensions[2] > reference.dimensions[2] {
                *reference = part;
            }
        }
        let (cuts, joinery_omitted) = if settings.show_joinery {
            let scopes = parts.iter().filter(|(_, _, included, _, _)| *included)
                .filter_map(|(_, _, _, _, scope)| *scope).collect();
            joinery_cuts(model, &scopes)
        } else {
            (BTreeMap::new(), false)
        };
        let mut groups = BTreeMap::new();
        for (path, part, included, material, scope) in &parts {
            if !included {
                continue;
            }
            let mut part = *part;
            if part.rectangular && part.grain == Grain::Unassigned {
                if let Some(reference) = references.get(&(path.as_str(), part.dimensions[0], material)) {
                    if part.axes[0].dot(reference.axes[0]).abs() > 1. - 1e-8
                        && part.axes[1].dot(reference.axes[2]).abs() > 1. - 1e-8
                    {
                        part.dimensions.swap(1, 2);
                        part.axes.swap(1, 2);
                        let (lo, hi) = part.bounds;
                        part.bounds = (DVec3::new(lo.x, lo.z, lo.y), DVec3::new(hi.x, hi.z, hi.y));
                    }
                }
            }
            let group = groups.entry((part.dimensions, part.rectangular, material.clone(), part.grain))
                .or_insert_with(|| CutGroup {
                    material: material.1.clone(), material_id: material.0,
                    dimensions: part.dimensions, rectangular: part.rectangular,
                    parts: Vec::new(), grain: part.grain, joinery: BTreeMap::new(),
                });
            if let Some(segments) = scope.and_then(|id| cuts.get(&id)) {
                let lines = project_joinery(part, segments);
                if !lines.is_empty() {
                    group.joinery.insert(group.parts.len(), lines);
                }
            }
            group.parts.push(path.clone());
        }
        if groups.is_empty() {
            return Err(tr!("cutting_plan.no_visible_three_dimensional_parts_with_assigned_stock").into());
        }
        Ok(Self {
            groups: groups.into_values().collect(),
            skipped,
            joinery_omitted,
        })
    }

    fn ordered_groups(&self) -> Vec<&CutGroup> {
        let mut groups: Vec<_> = self.groups.iter().collect();
        groups.sort_by(|a, b| {
            group_names(a)
                .cmp(&group_names(b))
                .then_with(|| b.parts.len().cmp(&a.parts.len()))
                .then_with(|| b.dimensions[2].cmp(&a.dimensions[2]))
        });
        groups
    }

    pub fn material_requirements(&self, settings: LayoutSettings) -> Vec<MaterialRequirement> {
        let mut materials: BTreeMap<MaterialKey, Vec<(usize, &CutGroup)>> = BTreeMap::new();
        for (mark, group) in self.ordered_groups().into_iter().enumerate() {
            materials
                .entry((group.material_id, group.material.clone()))
                .or_default()
                .push((mark, group));
        }
        materials
            .into_iter()
            .map(|((material_id, material), groups)| {
                let layout = stock_layout(&material, &groups, settings);
                MaterialRequirement {
                    material_id,
                    material,
                    quantity: layout.stock.len(),
                    excluded: layout.oversized.iter().map(|piece| piece.quantity).sum(),
                }
            })
            .collect()
    }

    #[cfg(test)]
    pub fn pdf(&self, name: &str, selection: bool, group_stock: bool) -> Vec<u8> {
        self.pdf_with_layout(name, selection, group_stock, LayoutSettings::default())
    }

    pub fn pdf_with_layout(
        &self,
        name: &str,
        selection: bool,
        group_stock: bool,
        settings: LayoutSettings,
    ) -> Vec<u8> {
        let count: usize = self.groups.iter().map(|g| g.parts.len()).sum();
        let mut pdf = Pdf::new(name, tr!("cutting_plan.cutting_plan"), selection);
        pdf.heading(tr!("cutting_plan.cut_list"), 18.);
        pdf.paragraph(
            &tr!(
                "cutting_plan.cut_list_summary",
                pieces = tr!("common.pieces", count = count),
                sizes = tr!("cutting_plan.sizes", count = self.groups.len()),
            ),
            10.,
        );
        if self.skipped > 0 {
            pdf.paragraph(
                &tr!(
                    "cutting_plan.omitted_items",
                    count = self.skipped
                ),
                9.,
            );
        }
        if settings.show_joinery && self.joinery_omitted {
            pdf.paragraph(tr!("cutting_plan.joinery_omitted"), 9.);
        }
        pdf.gap(8.);
        let groups = self.ordered_groups();
        if group_stock {
            let mut stock_groups: BTreeMap<MaterialKey, Vec<(usize, &CutGroup)>> = BTreeMap::new();
            for (index, group) in groups.into_iter().enumerate() {
                stock_groups
                    .entry((group.material_id, group.material.clone()))
                    .or_default()
                    .push((index, group));
            }
            for ((_, stock), groups) in stock_groups {
                pdf.ensure(170.);
                let title = tr!(
                    "cutting_plan.material_x_x",
                    stock.name,
                    metric_length(stock.dimensions[0]),
                    metric_length(stock.dimensions[1]),
                    metric_length(stock.dimensions[2])
                );
                let settings = LayoutSettings {
                    sheet_width_mm: stock.dimensions[1] as f64 / 10.,
                    sheet_length_mm: stock.dimensions[2] as f64 / 10.,
                    board_length_mm: stock.dimensions[2] as f64 / 10.,
                    ..settings
                };
                pdf.heading(&title, 11.);
                let quantity: usize = groups.iter().map(|(_, g)| g.parts.len()).sum();
                let pieces = tr!("common.pieces", count = quantity);
                let packed = stock_layout(&stock, &groups, settings);
                let layout = Some(packed);
                let sheet_stock = true;
                let area: f64 = groups
                    .iter()
                    .map(|(_, g)| {
                        g.dimensions[1] as f64 * g.dimensions[2] as f64 * g.parts.len() as f64
                            / 100_000_000.
                    })
                    .sum();
                let summary = tr!("cutting_plan.net_cut_area_m2", pieces = pieces, area = area);
                pdf.paragraph(&summary, 9.);
                if let Some(layout) = &layout {
                    let count = layout.stock.len();
                    let kerf = metric_length(cutting_layout::tenths(settings.kerf_mm));
                    let summary = if sheet_stock {
                        tr!("cutting_plan.stock_required", count = count, kerf = kerf)
                    } else {
                        tr!("cutting_plan.boards_required", count = count, kerf = kerf)
                    };
                    pdf.paragraph(&summary, 9.);
                }
                if layout.as_ref().is_some_and(|l| !l.oversized.is_empty()) {
                    pdf.paragraph(tr!("cutting_plan.stock_count_covers_fitting_cuts_only_oversized_cuts"), 9.);
                }
                pdf.table_header();
                for &(index, group) in &groups {
                    if pdf.y < 115. {
                        pdf.new_page();
                        pdf.heading(&tr!("cutting_plan.continued", title = title), 10.);
                        pdf.table_header();
                    }
                    cut_row(&mut pdf, index, group, true);
                }
                if let Some(layout) = &layout {
                    draw_layout(&mut pdf, &title, layout, sheet_stock, settings, &groups);
                }
                pdf.gap(12.);
            }
        } else {
            pdf.table_header();
            for (index, group) in groups.into_iter().enumerate() {
                if pdf.y < 115. {
                    pdf.new_page();
                    pdf.table_header();
                }
                cut_row(&mut pdf, index, group, false);
            }
        }
        pdf.finish()
    }

    pub fn workshop_labels_pdf(&self, name: &str, selection: bool) -> Result<Vec<u8>, String> {
        const WIDTH: f64 = 247.5;
        const HEIGHT: f64 = 66.;
        const ROW_GAP: f64 = 10.;
        const COLUMN_GAP: f64 = 16.;

        let mut pdf = Pdf::new(name, tr!("cutting_plan.workshop_labels"), selection);
        let first_row = pdf.y - 6.;
        let rows = ((first_row - 48. + ROW_GAP) / (HEIGHT + ROW_GAP)).floor() as usize;
        if rows == 0 {
            return Err(tr!("cutting_plan.the_project_name_is_too_long_to_fit").into());
        }

        let mut slot = 0;
        for (index, group) in self.ordered_groups().into_iter().enumerate() {
            let mark = piece_mark(index);
            let [t, w, l] = group.dimensions.map(metric_length);
            let dimensions = format!("{t} x {w} x {l}");
            for part in &group.parts {
                if slot == rows * 2 {
                    pdf.new_page();
                    slot = 0;
                }
                let x = 42. + (slot % 2) as f64 * (WIDTH + COLUMN_GAP);
                let top = first_row - (slot / 2) as f64 * (HEIGHT + ROW_GAP);
                pdf.cut_guide(x, top - HEIGHT, WIDTH, HEIGHT);
                pdf.text(x + 10., top - 20., 17., &mark);

                // Courier at 10 pt uses 6 pt per character. Keep two lines
                // for the name and reserve the bottom line for dimensions.
                let part = if part.is_empty() { tr!("cutting_plan.unnamed_part") } else { part };
                let mut chars = part.chars();
                let first: String = chars.by_ref().take(37).collect();
                let mut second: String = chars.by_ref().take(37).collect();
                if chars.next().is_some() {
                    for _ in 0..3 {
                        second.pop();
                    }
                    second.push_str("...");
                }
                pdf.text(x + 10., top - 34., 10., &first);
                if !second.is_empty() {
                    pdf.text(x + 10., top - 45., 10., &second);
                }
                let size = (WIDTH - 20.) / (dimensions.len() as f64 * 0.6);
                pdf.text(x + 10., top - 58., size.min(9.5), &dimensions);
                slot += 1;
            }
        }
        Ok(pdf.finish())
    }
}

fn piece_mark(index: usize) -> String {
    format!("P{:03}", index + 1)
}

fn automatic_group_name(name: &str) -> bool {
    name.strip_prefix("Group")
        .and_then(|suffix| suffix.trim_start().strip_prefix('#'))
        .is_some_and(|id| !id.is_empty() && id.bytes().all(|c| c.is_ascii_digit()))
}

fn group_names(group: &CutGroup) -> BTreeSet<&str> {
    group
        .parts
        .iter()
        .map(|name| if name.is_empty() { "-" } else { name.as_str() })
        .collect()
}

fn group_lines(group: &CutGroup) -> Vec<String> {
    let mut lines = Vec::new();
    for name in group_names(group) {
        let characters: Vec<_> = name
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        // A fixed-width font keeps long paths inside the Group column.
        for chunk in characters.chunks(32) {
            lines.push(chunk.iter().collect());
        }
    }
    if lines.is_empty() {
        lines.push("-".into());
    }
    lines
}

fn cut_row(pdf: &mut Pdf, index: usize, group: &CutGroup, colored: bool) {
    let [t, w, l] = group.dimensions;
    let lines = group_lines(group);
    let mut offset = 0;
    while offset < lines.len() {
        let continued = offset > 0;
        if continued {
            pdf.new_page();
            pdf.table_header();
        }
        if colored {
            pdf.rectangle(42., pdf.y - 1., 7., 7., mark_color(index));
            pdf.cell(53., 41., &piece_mark(index));
        } else {
            pdf.cell(42., 52., &piece_mark(index));
        }
        if !continued {
            pdf.cell(286., 34., &group.parts.len().to_string());
            pdf.cell(
                330.,
                132.,
                &format!("{} x {}", metric_length(t), metric_length(w)),
            );
            pdf.cell(474., 79., &metric_length(l));
        } else {
            pdf.text(100., pdf.y, 8., tr!("cutting_plan.continued_2"));
            pdf.y -= 12.;
        }
        let available = ((pdf.y - 100.) / 12.).floor().max(1.) as usize;
        let count = available.min(lines.len() - offset);
        for (row, line) in lines[offset..offset + count].iter().enumerate() {
            pdf.text(100., pdf.y - row as f64 * 12., 9., line);
        }
        pdf.y -= (count as f64 * 12.).max(14.);
        if !continued {
            if let Some(stock) = nominal_stock(t, w) {
                pdf.text(330., pdf.y, 8., &tr!("cutting_plan.approx_nominal", stock = stock));
                pdf.y -= 12.;
            }
            if !group.rectangular {
                pdf.text(100., pdf.y, 8., tr!("cutting_plan.check_shape_bounding_blank_only"));
                pdf.y -= 12.;
            }
            if group.grain == Grain::ThroughThickness {
                pdf.paragraph(tr!("cutting_plan.grain_through_thickness"), 8.);
            }
        }
        pdf.separator();
        offset += count;
    }
}

/// Golden-angle hues are deterministic across stock groups and pages.
fn mark_color(mark: usize) -> [f64; 3] {
    let hue = (mark as f64 * 0.618033988749895).fract() * 6.;
    let fraction = hue.fract();
    let low = 0.42;
    let high = 0.92;
    let rising = low + (high - low) * fraction;
    let falling = high - (high - low) * fraction;
    match hue.floor() as u32 {
        0 => [high, rising, low],
        1 => [falling, high, low],
        2 => [low, high, rising],
        3 => [low, falling, high],
        4 => [rising, low, high],
        _ => [high, low, falling],
    }
}

fn joinery_cuts(model: &Model, scopes: &BTreeSet<u64>) -> (BTreeMap<u64, Vec<[DVec3; 2]>>, bool) {
    use benchup_core::connections::ConnectionOperation;
    let connections: Vec<_> = model.connections.iter().filter(|(_, connection)| {
        matches!(connection.operation, ConnectionOperation::Joinery(_))
            && connection.members.iter().any(|member| scopes.contains(member))
    }).collect();
    if connections.is_empty() {
        return (BTreeMap::new(), false);
    }
    let evaluated = model.evaluate_joinery();
    let failed: BTreeSet<_> = evaluated.errors.iter().map(|(id, _)| *id).collect();
    let mut omitted = connections.iter().any(|(id, _)| failed.contains(id));
    let members: BTreeSet<_> = connections.into_iter()
        .filter(|(id, _)| !failed.contains(id))
        .flat_map(|(_, connection)| connection.members)
        .filter(|member| scopes.contains(member))
        .collect();
    let mut cuts = BTreeMap::new();
    for member in members {
        match model.joinery_cut_edges(&evaluated.model, member) {
            Ok(edges) => { cuts.insert(member, edges); }
            Err(_) => omitted = true,
        }
    }
    (cuts, omitted)
}

fn project_joinery(part: MeasuredPart, segments: &[[DVec3; 2]]) -> Vec<JointLine> {
    let (lo, hi) = part.bounds;
    let mut lines = BTreeSet::new();
    for segment in segments {
        let [a, b] = segment.map(|point| project(point - part.origin, part.axes));
        let Some(segment) = clip_segment_to_bounds(a, b, lo, hi) else { continue };
        let mut line = segment.map(|point| {
            let point = (point - lo) * 10_000.;
            [(point.z.round() as u64).min(part.dimensions[2]),
             (point.y.round() as u64).min(part.dimensions[1])]
        });
        if line[0] != line[1] {
            line.sort_unstable();
            lines.insert(line);
        }
    }
    lines.into_iter().collect()
}

fn clip_segment_to_bounds(a: DVec3, b: DVec3, lo: DVec3, hi: DVec3) -> Option<[DVec3; 2]> {
    let delta = b - a;
    let mut start: f64 = 0.;
    let mut end: f64 = 1.;
    for axis in 0..3 {
        if delta[axis].abs() < 1e-9 {
            if a[axis] < lo[axis] - 1e-8 || a[axis] > hi[axis] + 1e-8 {
                return None;
            }
            continue;
        }
        let low = (lo[axis] - a[axis]) / delta[axis];
        let high = (hi[axis] - a[axis]) / delta[axis];
        start = start.max(low.min(high));
        end = end.min(low.max(high));
    }
    (start < end).then(|| [a + start * delta, a + end * delta])
}

fn draw_layout(
    pdf: &mut Pdf,
    title: &str,
    layout: &cutting_layout::Layout,
    sheet: bool,
    settings: LayoutSettings,
    groups: &[(usize, &CutGroup)],
) {
    let groups: BTreeMap<_, _> = groups.iter().copied().collect();
    if !layout.oversized.is_empty() {
        pdf.ensure(70.);
        pdf.heading(tr!("cutting_plan.does_not_fit_selected_stock"), 10.);
        for (mark, quantity) in layout.oversized_marks() {
            let group = groups[&mark];
            pdf.paragraph(
                &tr!(
                    "cutting_plan.oversized_cut",
                    count = quantity,
                    mark = piece_mark(mark),
                    width = metric_length(group.dimensions[1]),
                    length = metric_length(group.dimensions[2]),
                ),
                9.,
            );
        }
    }
    for (number, stock) in layout.stock.iter().enumerate() {
        let scale = (511. / stock.length as f64).min(320. / stock.width as f64);
        let width = if sheet {
            stock.length as f64 * scale
        } else {
            511.
        };
        let height = if sheet {
            stock.width as f64 * scale
        } else {
            48.
        };
        let has_grain = settings.respect_grain && stock.placements.iter().any(|placement| {
            groups[&placement.mark].grain == Grain::AlongLength
        });
        let has_joinery = settings.show_joinery && stock.placements.iter().any(|placement| {
            groups[&placement.mark].joinery.contains_key(&placement.part_index)
        });
        let legend_height = 16. * f64::from(u8::from(has_grain) + u8::from(has_joinery));
        pdf.ensure(height + 115. + legend_height);
        pdf.heading(
            &tr!(
                "cutting_plan.stock_page_heading",
                if sheet { tr!("cutting_plan.stock") } else { tr!("cutting_plan.board") },
                number + 1,
                layout.stock.len()
            ),
            12.,
        );
        pdf.paragraph(title, 9.);
        pdf.paragraph(
            &tr!(
                "cutting_plan.stock_x_kerf_gray_offcut_kerf",
                metric_length(stock.length),
                metric_length(stock.width),
                metric_length(cutting_layout::tenths(settings.kerf_mm))
            ),
            8.,
        );
        if has_grain {
            pdf.paragraph(tr!("cutting_plan.grain_along_stock_length"), 8.);
        }
        if has_joinery {
            pdf.paragraph(tr!("cutting_plan.joinery_dashed_outlines"), 8.);
        }
        let top = pdf.y;
        pdf.rectangle(42., top - height, width, height, [0.94; 3]);
        let x_scale = width / stock.length as f64;
        let y_scale = height / stock.width as f64;
        for placement in &stock.placements {
            let r = placement.rect;
            let x = 42. + r.x as f64 * x_scale;
            let y = top - (r.y + r.width) as f64 * y_scale;
            let w = r.length as f64 * x_scale;
            let h = r.width as f64 * y_scale;
            pdf.rectangle(x, y, w, h, mark_color(placement.mark));
            if settings.show_joinery {
                let group = groups[&placement.mark];
                if let Some(lines) = group.joinery.get(&placement.part_index) {
                    let position = |[length, width]: [u64; 2]| {
                        let [u, v] = if placement.rotated {
                            [group.dimensions[1] - width, length]
                        } else {
                            [length, width]
                        };
                        [x + u as f64 * x_scale, y + h - v as f64 * y_scale]
                    };
                    pdf.joinery_lines(lines.iter().map(|line| line.map(position)));
                }
            }
            let label = piece_mark(placement.mark);
            let size = (w / (label.len() as f64 * 0.6 + 1.)).min(9.);
            if size >= 5. && h >= size + 4. {
                pdf.text(
                    x + (w - label.len() as f64 * size * 0.6) * 0.5,
                    y + h * 0.5,
                    size,
                    &label,
                );
                let dimensions = if sheet {
                    format!("{} x {}", metric_length(r.length), metric_length(r.width))
                } else {
                    metric_length(r.length)
                };
                if h >= 30. && dimensions.len() as f64 * 4.2 + 4. <= w {
                    pdf.text(
                        x + (w - dimensions.len() as f64 * 4.2) * 0.5,
                        y + h * 0.5 - 12.,
                        7.,
                        &dimensions,
                    );
                }
            }
        }
        pdf.y = top - height - 20.;
        let mut legend = BTreeMap::new();
        for placement in &stock.placements {
            let entry = legend.entry(placement.mark).or_insert((0_usize, false));
            entry.0 += 1;
            entry.1 |= placement.rotated;
        }
        for (mark, (quantity, rotated)) in legend {
            pdf.ensure(22.);
            let group = groups[&mark];
            pdf.rectangle(42., pdf.y - 1., 7., 7., mark_color(mark));
            pdf.text(
                55.,
                pdf.y,
                9.,
                &format!(
                    "{} x{} - {} x {}{}",
                    piece_mark(mark),
                    quantity,
                    metric_length(group.dimensions[1]),
                    metric_length(group.dimensions[2]),
                    if rotated { tr!("cutting_plan.rotation_used") } else { "" }
                ),
            );
            pdf.y -= 17.;
        }
        pdf.gap(12.);
    }
}

fn entity_vertices(model: &Model, id: u64) -> BTreeSet<u64> {
    if let Some(edge) = model.edges.get(&id) {
        BTreeSet::from([edge.a, edge.b])
    } else if let Some(face) = model.faces.get(&id) {
        face.vertices
            .iter()
            .chain(face.holes.iter().flatten())
            .copied()
            .collect()
    } else {
        BTreeSet::new()
    }
}

fn measure(model: &Model, entities: &BTreeSet<u64>, grain: Option<DVec3>) -> Option<MeasuredPart> {
    if !entities.iter().any(|id| model.faces.contains_key(id)) {
        return None;
    }
    let vertices: BTreeSet<_> = entities
        .iter()
        .flat_map(|id| entity_vertices(model, *id))
        .collect();
    let origin = model.vertices.get(vertices.first()?)?.position;
    // Work relative to a nearby origin to preserve precision far from world zero.
    let points: Vec<_> = vertices
        .iter()
        .map(|id| model.vertices[id].position - origin)
        .collect();
    let mut directions: Vec<(DVec3, f64)> = Vec::new();
    let mut add_direction = |a: u64, b: u64| {
        let delta = model.vertices[&b].position - model.vertices[&a].position;
        let Some(direction) = delta.try_normalize() else {
            return;
        };
        if let Some((_, weight)) = directions
            .iter_mut()
            .find(|(d, _)| d.dot(direction).abs() > 1. - 1e-8)
        {
            *weight += delta.length();
        } else if directions.len() < 256 {
            directions.push((direction, delta.length()));
        }
    };
    for id in entities {
        if let Some(edge) = model.edges.get(id) {
            add_direction(edge.a, edge.b);
        }
        if let Some(face) = model.faces.get(id) {
            for ring in std::iter::once(&face.vertices).chain(face.holes.iter()) {
                for i in 0..ring.len() {
                    add_direction(ring[i], ring[(i + 1) % ring.len()]);
                }
            }
        }
    }
    directions.sort_by(|a, b| b.1.total_cmp(&a.1));
    directions.truncate(32);
    let mut basis = [DVec3::X, DVec3::Y, DVec3::Z];
    let mut bounds = projected_bounds(&points, basis);
    let mut volume = (bounds.1 - bounds.0).element_product();
    for (u, _) in &directions {
        for (v, _) in &directions {
            let cross = u.cross(*v);
            if cross.length_squared() < 1e-12 {
                continue;
            }
            let normal = cross.normalize();
            let axes = [*u, normal.cross(*u), normal];
            let candidate = projected_bounds(&points, axes);
            let candidate_volume = (candidate.1 - candidate.0).element_product();
            if candidate_volume < volume {
                basis = axes;
                bounds = candidate;
                volume = candidate_volume;
            }
        }
    }
    let sizes = (bounds.1 - bounds.0).to_array();
    if sizes.iter().any(|v| !v.is_finite() || *v < 0.00005) {
        return None;
    }
    let mut order = [0, 1, 2];
    order.sort_by(|a, b| sizes[*a].total_cmp(&sizes[*b]));
    let mut rectangular = is_box(model, entities, origin, basis, bounds);
    let mut axes = order.map(|axis| basis[axis]);
    let mut direction = Grain::Unassigned;
    if let Some(grain) = grain {
        let thickness = axes[0];
        let in_plane = grain - thickness * grain.dot(thickness);
        if in_plane.length_squared() > 1e-12 {
            let length = in_plane.normalize();
            rectangular &= axes[1].dot(length).abs().max(axes[2].dot(length).abs()) > 1. - 1e-8;
            axes = [thickness, thickness.cross(length), length];
            direction = Grain::AlongLength;
        } else {
            direction = Grain::ThroughThickness;
        }
    }
    let bounds = projected_bounds(&points, axes);
    Some(MeasuredPart {
        dimensions: (bounds.1 - bounds.0).to_array().map(|extent| (extent * 10_000.).round() as u64),
        axes,
        origin,
        bounds,
        rectangular,
        grain: direction,
    })
}

fn project(p: DVec3, axes: [DVec3; 3]) -> DVec3 {
    DVec3::new(p.dot(axes[0]), p.dot(axes[1]), p.dot(axes[2]))
}
fn projected_bounds(points: &[DVec3], axes: [DVec3; 3]) -> (DVec3, DVec3) {
    points.iter().fold(
        (DVec3::splat(f64::INFINITY), DVec3::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| {
            let p = project(*p, axes);
            (lo.min(p), hi.max(p))
        },
    )
}

fn is_box(
    model: &Model,
    entities: &BTreeSet<u64>,
    origin: DVec3,
    axes: [DVec3; 3],
    (lo, hi): (DVec3, DVec3),
) -> bool {
    let mut sides = BTreeSet::new();
    let mut boundaries: BTreeMap<(u64, u64), usize> = BTreeMap::new();
    for id in entities {
        let Some(face) = model.faces.get(id) else {
            continue;
        };
        if !face.holes.is_empty() || face.vertices.len() < 3 {
            return false;
        }
        let points: Vec<_> = face
            .vertices
            .iter()
            .map(|id| project(model.vertices[id].position - origin, axes))
            .collect();
        let side = (0..3)
            .flat_map(|axis| [(axis, false), (axis, true)])
            .find(|(axis, high)| {
                let coordinate = if *high { hi[*axis] } else { lo[*axis] };
                points.iter().all(|p| (p[*axis] - coordinate).abs() < 1e-7)
            });
        let Some(side) = side else { return false };
        sides.insert(side);
        for i in 0..face.vertices.len() {
            let a = face.vertices[i];
            let b = face.vertices[(i + 1) % face.vertices.len()];
            *boundaries.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    sides.len() == 6 && boundaries.values().all(|count| *count == 2)
}

fn metric_length(tenths: u64) -> String {
    if tenths > 1000 {
        let value = format!("{}.{:02}", tenths / 100, tenths % 100);
        return tr!("cutting_plan.cm", value.trim_end_matches('0').trim_end_matches('.'));
    }
    if tenths.is_multiple_of(10) {
        tr!("cutting_plan.mm", tenths / 10)
    } else {
        tr!("cutting_plan.mm_2", tenths / 10, tenths % 10)
    }
}

fn nominal_stock(thickness: u64, width: u64) -> Option<&'static str> {
    // Rounded metric S4S sizes plus exact inch equivalents where rounding differs.
    // AWC: https://awc.org/faq/how-do-i-convert-to-metric-lumber-sizes/
    const STOCK: &[(u64, u64, &str)] = &[
        (190, 635, "1x3"),
        (190, 640, "1x3"),
        (190, 890, "1x4"),
        (190, 1400, "1x6"),
        (380, 380, "2x2"),
        (380, 635, "2x3"),
        (380, 640, "2x3"),
        (380, 890, "2x4"),
        (380, 1400, "2x6"),
        (380, 1840, "2x8"),
        (380, 2350, "2x10"),
        (380, 2860, "2x12"),
        (635, 635, "3x3"),
        (640, 640, "3x3"),
        (635, 890, "3x4"),
        (640, 890, "3x4"),
        (890, 890, "4x4"),
        (1400, 1840, "6x8"),
    ];
    STOCK
        .iter()
        .find(|(t, w, _)| thickness.abs_diff(*t) <= 2 && width.abs_diff(*w) <= 2)
        .map(|(_, _, label)| *label)
}

/// Small dependency-free PDF 1.4 writer. Courier keeps wrapping and table widths
/// deterministic on desktop and WASM. Strings use escaped WinAnsi bytes.
struct Pdf {
    pages: Vec<String>,
    page: String,
    y: f64,
    name: String,
    title: &'static str,
    selection: bool,
    date: String,
}
impl Pdf {
    fn new(name: &str, title: &'static str, selection: bool) -> Self {
        let mut pdf = Self {
            pages: Vec::new(),
            page: String::new(),
            y: 0.,
            name: name.into(),
            title,
            selection,
            date: crate::estimate_pdf::generation_date().unwrap_or_else(|_| tr!("cutting_plan.date_unavailable").into()),
        };
        pdf.new_page();
        pdf
    }
    fn new_page(&mut self) {
        if !self.page.is_empty() {
            self.pages.push(std::mem::take(&mut self.page));
        }
        let header = crate::pdf_header::layout(
            &self.name,
            self.title,
            &self.date,
            [595., 842.],
            crate::pdf_header::base_width,
        );
        self.page.push_str(&header.graphics);
        for text in header.text {
            use std::fmt::Write;
            let _=writeln!(self.page,"0.15 0.20 0.17 rg BT /F2 {} Tf 1 0 0 1 {:.3} {:.3} Tm ({}) Tj ET",text.size,text.x,text.y,crate::pdf_header::literal(&text.value));
        }
        self.y=header.body_top;
        if self.selection {
            self.text(42., self.y, 9., tr!("cutting_plan.selected_parts_visible_only"));
            self.y -= 22.;
        }
    }
    fn ensure(&mut self, height: f64) {
        if self.y - height < 48. {
            self.new_page();
        }
    }
    fn gap(&mut self, height: f64) {
        self.y -= height;
    }
    fn heading(&mut self, value: &str, size: f64) {
        self.ensure(size + 20.);
        self.text(42., self.y, size, value);
        self.y -= size + 12.;
    }
    fn paragraph(&mut self, value: &str, size: f64) {
        // Hard wrapping also bounds long object names without spaces.
        let width = (511. / (size * 0.6)) as usize;
        let normalized: String = value
            .chars()
            .map(|c| if c.is_control() { ' ' } else { c })
            .collect();
        let chars: Vec<_> = normalized.chars().collect();
        let mut start = 0;
        while start < chars.len() {
            let mut end = (start + width).min(chars.len());
            if end < chars.len() {
                if let Some(space) = chars[start..end].iter().rposition(|c| *c == ' ') {
                    end = start + space + 1;
                }
            }
            self.ensure(size + 6.);
            self.text(
                42.,
                self.y,
                size,
                chars[start..end].iter().collect::<String>().trim_end(),
            );
            self.y -= size + 4.;
            start = end;
        }
        self.y -= 4.;
    }
    fn cell(&mut self, x: f64, width: f64, text: &str) {
        let size = (width / (text.chars().count().max(1) as f64 * 0.6)).min(10.);
        self.text(x, self.y, size, text);
    }
    fn text(&mut self, x: f64, y: f64, size: f64, text: &str) {
        use std::fmt::Write;
        let _ = writeln!(
            self.page,
            "0.12 0.14 0.15 rg BT /F1 {size} Tf {x:.2} {y:.2} Td ({}) Tj ET",
            pdf_string(text)
        );
    }
    fn rectangle(&mut self, x: f64, y: f64, width: f64, height: f64, color: [f64; 3]) {
        use std::fmt::Write;
        let [r, g, b] = color;
        let _ = writeln!(
            self.page,
            "q {r:.8} {g:.8} {b:.8} rg 0.35 G 0.4 w {x:.3} {y:.3} {width:.3} {height:.3} re B Q"
        );
    }
    fn cut_guide(&mut self, x: f64, y: f64, width: f64, height: f64) {
        use std::fmt::Write;
        let _ = writeln!(
            self.page,
            "q 0.6 G 0.5 w [2 2] 0 d {x:.2} {y:.2} {width:.2} {height:.2} re S Q"
        );
    }
    fn joinery_lines(&mut self, lines: impl Iterator<Item = [[f64; 2]; 2]>) {
        use std::fmt::Write;
        self.page.push_str("q 0.24 G 0.45 w [1.6 1.4] 0 d\n");
        for [[x1, y1], [x2, y2]] in lines {
            let _ = writeln!(self.page, "{x1:.3} {y1:.3} m {x2:.3} {y2:.3} l");
        }
        self.page.push_str("S Q\n");
    }
    fn line(&mut self, y: f64) {
        use std::fmt::Write;
        let _ = writeln!(self.page, "0.78 G 0.5 w 42 {y:.2} m 553 {y:.2} l S");
    }
    fn separator(&mut self) {
        self.gap(6.);
        self.line(self.y);
        self.gap(20.);
    }
    fn table_header(&mut self) {
        self.ensure(130.);
        self.gap(20.);
        for (x, title) in [
            (42., tr!("cutting_plan.mark")),
            (100., tr!("cutting_plan.group")),
            (286., tr!("cutting_plan.qty")),
            (330., tr!("cutting_plan.section_t_x_w")),
            (474., tr!("cutting_plan.cut_length")),
        ] {
            self.text(x, self.y, 9., title);
        }
        self.y -= 12.;
        self.separator();
    }
    fn finish(mut self) -> Vec<u8> {
        self.pages.push(self.page);
        let count = self.pages.len();
        let mut objects = vec![
            "<< /Type /Catalog /Pages 2 0 R >>".to_owned(),
            format!(
                "<< /Type /Pages /Count {count} /Kids [{}] >>",
                (0..count)
                    .map(|i| format!("{} 0 R", 4 + i * 2))
                    .collect::<Vec<_>>()
                    .join(" ")
            ),
            "<< /Type /Font /Subtype /Type1 /BaseFont /Courier /Encoding /WinAnsiEncoding >>"
                .to_owned(),
        ];
        let logo_id=4+count*2;
        let header_font_id=logo_id+1;
        for (i, page) in self.pages.iter_mut().enumerate() {
            use std::fmt::Write;
            let note = pdf_string(tr!("cutting_plan.footer_note"));
            let number = pdf_string(&tr!("cutting_plan.page_number", page = i + 1, count = count));
            let _ = writeln!(
                page,
                "0.4 g BT /F1 8 Tf 42 28 Td ({note}) Tj ET BT /F1 8 Tf 463 28 Td ({number}) Tj ET",
            );
            objects.push(format!("<< /Type /Page /Parent 2 0 R /MediaBox [0 0 595 842] /Resources << /Font << /F1 3 0 R /F2 {header_font_id} 0 R >> /XObject << /Logo {logo_id} 0 R >> >> /Contents {} 0 R >>", 5 + i * 2));
            objects.push(format!(
                "<< /Length {} >>\nstream\n{page}endstream",
                page.len()
            ));
        }
        objects.push(crate::pdf_header::logo_hex_object());
        objects.push("<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica /Encoding /WinAnsiEncoding >>".into());
        let mut bytes = b"%PDF-1.4\n".to_vec();
        let mut offsets = Vec::new();
        for (i, object) in objects.iter().enumerate() {
            offsets.push(bytes.len());
            bytes.extend_from_slice(format!("{} 0 obj\n{object}\nendobj\n", i + 1).as_bytes());
        }
        let xref = bytes.len();
        bytes.extend_from_slice(
            format!("xref\n0 {}\n0000000000 65535 f \n", objects.len() + 1).as_bytes(),
        );
        for offset in offsets {
            bytes.extend_from_slice(format!("{offset:010} 00000 n \n").as_bytes());
        }
        bytes.extend_from_slice(
            format!(
                "trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n",
                objects.len() + 1
            )
            .as_bytes(),
        );
        bytes
    }
}

fn pdf_string(text: &str) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for c in text.chars() {
        let byte = match c {
            ' '..='~' | '\u{a0}'..='\u{ff}' => c as u8,
            '\u{2013}' | '\u{2014}' => b'-',
            '\u{2018}' | '\u{2019}' => b'\'',
            '\u{201c}' | '\u{201d}' => b'"',
            _ if c.is_control() => b' ',
            _ => b'?',
        };
        if byte == b'(' || byte == b')' || byte == b'\\' || !byte.is_ascii() {
            let _ = write!(out, "\\{byte:03o}");
        } else {
            out.push(byte as char);
        }
    }
    out
}
