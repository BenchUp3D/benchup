//! Click selection expands model topology only on input; retained GPU highlighting
//! continues to consume the same selected entity IDs as single/box selection.
use super::*;
use std::collections::{HashMap, HashSet};

// Transient gesture state, never a document or preference. Retain the original
// selection so Shift applies once to the final extent of a multi-click sequence.
pub(super) struct Click {
    hit: u64,
    position: Pos2,
    time: f64,
    count: u8,
    context: Option<u64>,
    revision: u64,
    additive: bool,
    before: BTreeSet<u64>,
    after: BTreeSet<u64>,
}

impl BenchUpApp {
    /// Returns true when opening an editing context or text editor consumes input.
    pub(super) fn select_click(
        &mut self,
        ctx: &egui::Context,
        position: Pos2,
        hit: Option<u64>,
        additive: bool,
    ) -> bool {
        let Some(hit) = hit else {
            self.selection_click = None;
            if !additive {
                if self.doc.model.edit_context.is_some() {
                    self.close_edit_context(ctx);
                    return true;
                }
                self.selected.clear();
            }
            return false;
        };
        let time = ctx.input(|i| i.time);
        let options = ctx.options(|o| o.input_options);
        let revision = self.document_revision(ctx);
        let previous = self.selection_click.take().filter(|click| {
            click.hit == hit
                && click.context == self.doc.model.edit_context
                && click.revision == revision
                && click.additive == additive
                && click.after == self.selected
                && time - click.time < options.max_double_click_delay
                && position.distance(click.position) < options.max_click_dist
        });
        let (count, before) = match previous {
            Some(click) => ((click.count + 1).min(3), click.before),
            None => (
                1,
                if additive {
                    self.selected.clone()
                } else {
                    BTreeSet::new()
                },
            ),
        };
        if count == 2 && !additive {
            if self.doc.model.texts.contains_key(&hit) {
                self.selected = BTreeSet::from([hit]);
                self.begin_text_edit(hit, ctx);
                return true;
            }
            if self.doc.model.objects.contains_key(&hit) {
                self.enter_object(hit, ctx);
                return true;
            }
        }
        self.entity_cache
            .update_for_revision(&self.doc.model, revision);
        let hits = expand_click(&self.doc.model, hit, count, |id| {
            let state = self.entity_cache.get(id);
            !state.hidden && state.context == Some(id)
        });
        self.selected = if additive {
            before.symmetric_difference(&hits).copied().collect()
        } else {
            hits
        };
        self.selection_click = Some(Click {
            hit,
            position,
            time,
            count,
            context: self.doc.model.edit_context,
            revision,
            additive,
            before,
            after: self.selected.clone(),
        });
        false
    }
}

fn entity_vertices(model: &Model, id: u64) -> impl Iterator<Item = u64> + '_ {
    model
        .edges
        .get(&id)
        .into_iter()
        .flat_map(|edge| [edge.a, edge.b])
        .chain(model.faces.get(&id).into_iter().flat_map(|face| {
            face.vertices
                .iter()
                .chain(face.holes.iter().flatten())
                .copied()
        }))
}

fn expand_click(
    model: &Model,
    hit: u64,
    count: u8,
    mut selectable: impl FnMut(u64) -> bool,
) -> BTreeSet<u64> {
    let mut selected = BTreeSet::from([hit]);
    if count == 1 {
        return selected;
    }
    if count == 2 {
        if let Some(face) = model.faces.get(&hit) {
            let pairs: HashSet<_> = std::iter::once(&face.vertices)
                .chain(face.holes.iter())
                .flat_map(|ring| {
                    (0..ring.len()).map(|i| {
                        let (a, b) = (ring[i], ring[(i + 1) % ring.len()]);
                        (a.min(b), a.max(b))
                    })
                })
                .collect();
            selected.extend(
                model
                    .edges
                    .values()
                    .filter(|edge| {
                        pairs.contains(&(edge.a.min(edge.b), edge.a.max(edge.b)))
                            && selectable(edge.id)
                    })
                    .map(|edge| edge.id),
            );
        }
        return selected;
    }
    if !model.faces.contains_key(&hit) && !model.edges.contains_key(&hit) {
        return selected;
    }
    // Build incidence once per triple-click, using cached visibility/scope. Each
    // vertex's neighbors are visited once, including connections through holes.
    let mut adjacency: HashMap<u64, Vec<u64>> = HashMap::new();
    for &id in model.edges.keys().chain(model.faces.keys()) {
        if selectable(id) {
            for vertex in entity_vertices(model, id) {
                adjacency.entry(vertex).or_default().push(id);
            }
        }
    }
    let mut pending = vec![hit];
    while let Some(id) = pending.pop() {
        for vertex in entity_vertices(model, id) {
            for adjacent in adjacency.remove(&vertex).into_iter().flatten() {
                if selected.insert(adjacent) {
                    pending.push(adjacent);
                }
            }
        }
    }
    selected
}
