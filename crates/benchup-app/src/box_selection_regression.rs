//! Compare cached area selection against the original exhaustive selector.
use super::*;
use std::collections::BTreeSet;

pub(super) fn validate() -> Result<usize, String> {
    let mut model = Model::default();
    let mut groups = Vec::new();
    for i in 0..3 {
        let before = model.root_entities();
        let p = DVec3::new(i as f64 * 3., 0., i as f64 * 0.25);
        model
            .draw_polyline(
                &[
                    p,
                    p + DVec3::X * 2.,
                    p + DVec3::X * 2. + DVec3::Y * 2.,
                    p + DVec3::Y * 2.,
                    p,
                ],
                true,
            )
            .map_err(str::to_owned)?;
        model
            .add_dimension(p, p + DVec3::X, DVec3::Y * 0.4)
            .map_err(str::to_owned)?;
        model
            .add_angular_dimension(p, p + DVec3::X, p + DVec3::Y)
            .map_err(str::to_owned)?;
        model
            .add_text(p, DVec3::Z * 0.5, "Selection label")
            .map_err(str::to_owned)?;
        model
            .add_guide(p + DVec3::Y * 0.8, Some(DVec3::X))
            .map_err(str::to_owned)?;
        model
            .add_guide(p + DVec3::Y * 0.9, None)
            .map_err(str::to_owned)?;
        let ids = model.root_entities().difference(&before).copied().collect();
        groups.push(
            model
                .make_object(&ids, i == 1, "Selection test")
                .ok_or("Group failed")?,
        );
    }
    let outer = model
        .make_object(&groups[..2].iter().copied().collect(), false, "Nested")
        .ok_or("Nested group failed")?;
    let inner = *model.objects[&outer]
        .entities
        .iter()
        .find(|id| model.objects.contains_key(id))
        .ok_or("Missing inner group")?;
    model
        .add_edge(-DVec3::X, DVec3::Y * 3.)
        .map_err(str::to_owned)?;
    let original = model.clone();
    let size = DVec2::new(1064., 748.);
    let mut camera = Camera::default();
    camera.fit(model.bounds(), size);
    let mut entities = EntityCache::default();
    let mut checks = 0;
    for visibility in 0..5 {
        model = original.clone();
        match visibility {
            1 => {
                model.hidden.insert(*model.edges.keys().next().unwrap());
            }
            2 => {
                model.hidden.insert(inner);
            }
            3 => {
                let tag = model.add_tag("Hidden parent").map_err(str::to_owned)?;
                model.entity_tags.insert(outer, tag);
                model.tags.get_mut(&tag).unwrap().visible = false;
            }
            4 => {
                model.locked.insert(outer);
            }
            _ => {}
        }
        for context in [None, Some(outer), Some(inner), Some(groups[2])] {
            model.edit_context = context;
            entities.update(&model);
            for projection in 0..3 {
                let mut camera = camera.clone();
                if projection == 1 {
                    camera.projection = benchup_core::Projection::Parallel;
                }
                if projection == 2 {
                    camera.distance = 0.5;
                }
                for (lo, hi) in [
                    (DVec2::ZERO, size),
                    (size * 0.3, size * 0.7),
                    (size * 0.49, size * 0.51),
                ] {
                    for (a, b) in [
                        (lo, hi),
                        (hi, lo),
                        (DVec2::new(lo.x, hi.y), DVec2::new(hi.x, lo.y)),
                        (DVec2::new(hi.x, lo.y), DVec2::new(lo.x, hi.y)),
                    ] {
                        for guides in [false, true] {
                            let expected = reference(&model, &camera, size, a, b, guides);
                            let indexed = selection::box_select_with_guides(
                                &model, &camera, size, a, b, guides,
                            );
                            let cached = selection::box_select_with_state(
                                &model,
                                &camera,
                                size,
                                a,
                                b,
                                guides,
                                |id| {
                                    let s = entities.get(id);
                                    (s.hidden, s.context)
                                },
                            );
                            if expected != indexed || expected != cached {
                                return Err(format!(
                                    "Box selection mismatch: visibility {visibility}, context {context:?}, projection {projection}, guides {guides}, {a:?}..{b:?}: expected {expected:?}, indexed {indexed:?}, cached {cached:?}"
                                ));
                            }
                            checks += 2;
                        }
                    }
                }
            }
        }
    }
    Ok(checks)
}

fn reference(
    model: &Model,
    camera: &Camera,
    size: DVec2,
    start: DVec2,
    end: DVec2,
    include_guides: bool,
) -> BTreeSet<u64> {
    let lo = start.min(end);
    let hi = start.max(end);
    let inside = |p: DVec2| p.cmpge(lo).all() && p.cmple(hi).all();
    let mut selected: BTreeSet<_> = model
        .edges
        .values()
        .filter(|e| !model.is_hidden(e.id) && model.entity_in_context(e.id).is_some())
        .filter_map(|e| {
            let (a, b) = model.points(e);
            let (a, b) = (camera.project(a, size)?, camera.project(b, size)?);
            (inside(a) && inside(b)).then_some(e.id)
        })
        .collect();
    for face in model
        .faces
        .values()
        .filter(|f| !model.is_hidden(f.id) && model.entity_in_context(f.id).is_some())
    {
        let hit = face.vertices.iter().all(|id| {
            camera
                .project(model.vertices[id].position, size)
                .is_some_and(inside)
        });
        if hit {
            selected.insert(face.id);
        }
    }
    for dimension in model.dimensions.values().filter(|d| !model.is_hidden(d.id)) {
        let points = dimension.annotation_points(model);
        let hit = points
            .iter()
            .all(|p| camera.project(*p, size).is_some_and(inside));
        if hit {
            selected.insert(dimension.id);
        }
    }
    for text in model.texts.values().filter(|t| !model.is_hidden(t.id)) {
        let [a, b] = text.layout(model);
        let hit = camera.project(a, size).is_some_and(inside)
            && camera.project(b, size).is_some_and(|center| {
                let half = text.label_size() * 0.5;
                inside(center - half) && inside(center + half)
            });
        if hit {
            selected.insert(text.id);
        }
    }
    for (id, guide) in model
        .guides
        .iter()
        .filter(|(id, _)| include_guides && !model.is_hidden(**id))
    {
        // An infinite guide cannot be enclosed by a finite marquee. Select it
        // when the visible line crosses the box; points must be inside it.
        let hit = if guide.direction.is_some() {
            guide
                .screen_segment(camera, size)
                .is_some_and(|(a, b)| selection::intersects_rect(a, b, lo, hi))
        } else {
            camera.project(guide.origin, size).is_some_and(inside)
        };
        if hit {
            selected.insert(*id);
        }
    }
    let hits = selected.clone();
    selected = hits
        .iter()
        .filter_map(|id| model.entity_in_context(*id))
        .collect();
    selected.retain(|id| {
        !model.objects.contains_key(id)
            || model
                .expand_entities(&BTreeSet::from([*id]))
                .iter()
                .filter(|id| !model.objects.contains_key(id) && !model.is_hidden(**id))
                .all(|id| hits.contains(id))
    });
    selected
}
