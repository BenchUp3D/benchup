use super::*;

fn piece(mark: usize, length: u64, width: u64, quantity: usize) -> Piece {
    Piece {
        mark,
        length,
        width,
        quantity,
        grain_along_length: false,
    }
}
fn grained(mark: usize, length: u64, width: u64) -> Piece {
    Piece {
        grain_along_length: true,
        ..piece(mark, length, width, 1)
    }
}
fn settings(length: f64, width: f64, kerf: f64) -> Settings {
    Settings {
        sheet_length_mm: length,
        sheet_width_mm: width,
        board_length_mm: length,
        kerf_mm: kerf,
        allow_rotation: false,
        ..Settings::default()
    }
}

#[test]
fn saw_kerf_changes_required_stock_counts() {
    let cuts = [piece(0, 5000, 1000, 2)];
    assert_eq!(sheets(&cuts, settings(1000., 100., 0.)).stock.len(), 1);
    assert_eq!(sheets(&cuts, settings(1000., 100., 3.)).stock.len(), 2);
    // Exact edge fits never require an extra kerf outside the stock.
    assert_eq!(
        sheets(&[piece(0, 10000, 1000, 1)], settings(1000., 100., 3.))
            .stock
            .len(),
        1
    );
}

#[test]
fn rotation_is_opt_in_and_oversized_pieces_never_enlarge_stock() {
    let cuts = [piece(7, 7000, 9000, 2)];
    let options = settings(1000., 800., 3.);
    let locked = sheets(&cuts, options);
    assert!(locked.stock.is_empty());
    assert_eq!(locked.oversized, cuts);
    let rotated = sheets(
        &cuts,
        Settings {
            allow_rotation: true,
            ..options
        },
    );
    assert_eq!(rotated.stock.len(), 2);
    assert!(rotated.oversized.is_empty());
    for stock in rotated.stock {
        assert_eq!((stock.length, stock.width), (10000, 8000));
        assert!(stock.placements[0].rotated);
        assert_eq!(stock.placements[0].rect.length, 9000);
        assert_eq!(stock.placements[0].rect.width, 7000);
    }
    let huge = [piece(0, 200_000_000_000_000, 200_000_000_000_000, 2)];
    assert_eq!(sheets(&huge, options).oversized, huge);
}

/// Rip first: each rip opens a strip at the usable stock edge. Strips tile the usable
/// width from the top, and every part sits at its own strip's y and inside its height.
/// A part from another strip cannot reach into a strip's band. Returns the strip count.
fn assert_two_stage_rips(stock: &Stock, kerf: u64) -> usize {
    let usable = stock.usable;
    let mut rips: Vec<Rect> = stock
        .placements
        .iter()
        .filter(|p| p.rect.x == usable.x)
        .map(|p| p.rect)
        .collect();
    rips.sort_by_key(|r| r.y);
    let mut next = usable.y;
    for rip in &rips {
        assert_eq!(rip.y, next, "strip starts where the previous strip ends");
        next = rip.y + rip.width + kerf;
    }
    for p in &stock.placements {
        assert!(
            p.rect.x >= usable.x && p.rect.x + p.rect.length <= usable.x + usable.length,
            "piece {p:?} leaves the usable length"
        );
        let strip = rips
            .iter()
            .find(|strip| strip.y == p.rect.y)
            .unwrap_or_else(|| panic!("piece {p:?} is not at a strip edge"));
        assert!(
            p.rect.y + p.rect.width <= strip.y + strip.width,
            "piece {p:?} leaves its strip"
        );
    }
    rips.len()
}

fn assert_packs_every_piece(cuts: &[Piece], layout: &Layout, kerf: u64) {
    let mut counts = BTreeMap::new();
    for stock in &layout.stock {
        for (index, p) in stock.placements.iter().enumerate() {
            let r = p.rect;
            assert!(r.x + r.length <= stock.length);
            assert!(r.y + r.width <= stock.width);
            let original = &cuts[p.mark];
            assert_eq!(
                (r.length, r.width),
                if p.rotated {
                    (original.width, original.length)
                } else {
                    (original.length, original.width)
                }
            );
            *counts.entry(p.mark).or_insert(0) += 1;
            for other in &stock.placements[index + 1..] {
                let q = other.rect;
                assert!(
                    r.x + r.length + kerf <= q.x
                        || q.x + q.length + kerf <= r.x
                        || r.y + r.width + kerf <= q.y
                        || q.y + q.width + kerf <= r.y,
                    "Pieces overlap or omit kerf: {r:?}, {q:?}"
                );
            }
        }
    }
    for cut in cuts {
        assert_eq!(counts.get(&cut.mark).copied().unwrap_or(0), cut.quantity);
    }
}

#[test]
fn sheet_layouts_conserve_quantities_keep_pieces_in_bounds_and_reserve_kerf() {
    let cuts: Vec<_> = (0..30)
        .map(|i| {
            piece(
                i,
                1000 + ((i * 197) % 9000) as u64,
                500 + ((i * 313) % 6000) as u64,
                i % 3 + 1,
            )
        })
        .collect();
    for strategy in [Strategy::Fewest, Strategy::RipFirst, Strategy::LargestOffcut] {
        let options = Settings {
            allow_rotation: true,
            strategy,
            ..Settings::default()
        };
        let layout = sheets(&cuts, options);
        assert!(layout.oversized.is_empty());
        assert_packs_every_piece(&cuts, &layout, tenths(options.kerf_mm));
    }
}

#[test]
fn best_of_strategy_is_deterministic_and_never_uses_more_sheets_than_the_current_packer() {
    let cuts: Vec<_> = (0..40)
        .map(|i| {
            piece(
                i,
                800 + ((i * 411) % 14000) as u64,
                300 + ((i * 257) % 7000) as u64,
                i % 4 + 1,
            )
        })
        .collect();
    let options = Settings {
        allow_rotation: true,
        ..Settings::default()
    };
    let first = sheets(&cuts, options);
    let second = sheets(&cuts, options);
    assert_eq!(first.stock.len(), second.stock.len());
    for (a, b) in first.stock.iter().zip(&second.stock) {
        assert_eq!(a.placements.len(), b.placements.len());
        for (p, q) in a.placements.iter().zip(&b.placements) {
            assert_eq!((p.mark, p.rect, p.rotated), (q.mark, q.rect, q.rotated));
        }
    }
    let current = pack(&cuts, options, Order::Area, Split::Strips);
    assert!(first.stock.len() <= current.stock.len());
    let largest = sheets(
        &cuts,
        Settings {
            strategy: Strategy::LargestOffcut,
            ..options
        },
    );
    assert_eq!(largest.stock.len(), first.stock.len());
}

#[test]
fn rip_first_makes_full_length_rips_then_crosscuts_each_strip() {
    let cuts = [
        piece(0, 12000, 4000, 2),
        piece(1, 6000, 3000, 3),
        piece(2, 9000, 2000, 2),
    ];
    let options = Settings {
        allow_rotation: false,
        strategy: Strategy::RipFirst,
        ..settings(2000., 2000., 0.)
    };
    let layout = sheets(&cuts, options);
    assert!(layout.oversized.is_empty());
    let placed: usize = layout.stock.iter().map(|s| s.placements.len()).sum();
    assert_eq!(placed, 7);
    let strips: usize = layout.stock.iter().map(|stock| assert_two_stage_rips(stock, 0)).sum();
    assert!(strips >= 2);
}

#[test]
fn rip_first_rips_run_the_full_stock_length() {
    // The 600 mm rip leaves the band above it at full stock length, so the 2000 mm part
    // still fits on the same sheet. A shorter rip would push it onto a second sheet.
    let cuts = [piece(0, 6000, 4000, 1), piece(1, 20000, 3000, 1)];
    let options = Settings {
        allow_rotation: false,
        strategy: Strategy::RipFirst,
        ..settings(2000., 2000., 0.)
    };
    let layout = sheets(&cuts, options);
    assert_eq!(layout.stock.len(), 1);
    assert_two_stage_rips(&layout.stock[0], 0);
    let long = layout.stock[0].placements.iter().find(|p| p.mark == 1).unwrap();
    assert_eq!((long.rect.x, long.rect.y, long.rect.length), (0, 4000, 20000));
}

#[test]
fn largest_offcut_is_at_least_as_large_as_fewest_sheets_and_counts_the_real_empty_rectangle() {
    let cuts: Vec<_> = (0..25)
        .map(|i| piece(i, 2000 + ((i * 531) % 9000) as u64, 1500 + ((i * 277) % 4000) as u64, i % 3 + 1))
        .collect();
    let options = Settings {
        allow_rotation: true,
        ..Settings::default()
    };
    let offcut = |layout: &Layout| {
        layout
            .stock
            .iter()
            .map(|stock| largest_offcut(stock, tenths(options.kerf_mm)))
            .max()
            .unwrap_or(0)
    };
    let fewest = sheets(&cuts, options);
    let largest = sheets(
        &cuts,
        Settings {
            strategy: Strategy::LargestOffcut,
            ..options
        },
    );
    assert_eq!(largest.stock.len(), fewest.stock.len());
    assert!(offcut(&largest) >= offcut(&fewest));
    // One part in the corner of a 1000 x 1000 mm sheet leaves a full 1000 x 700 mm strip.
    let single = sheets(
        &[piece(0, 3000, 3000, 1)],
        Settings {
            sheet_length_mm: 1000.,
            sheet_width_mm: 1000.,
            kerf_mm: 0.,
            allow_rotation: false,
            ..Settings::default()
        },
    );
    assert_eq!(largest_offcut(&single.stock[0], 0), 70_000_000);
}

#[test]
fn grained_parts_never_rotate_when_grain_is_respected() {
    let options = Settings {
        sheet_length_mm: 1000.,
        sheet_width_mm: 800.,
        allow_rotation: true,
        ..Settings::default()
    };
    // Only fits turned across the grain, so it is reported instead of rotated.
    let along = [grained(0, 7000, 9000)];
    let locked = sheets(&along, options);
    assert!(locked.stock.is_empty());
    assert_eq!(locked.oversized, along);
    let free = sheets(
        &along,
        Settings {
            respect_grain: false,
            ..options
        },
    );
    assert!(free.oversized.is_empty());
    assert!(free.stock[0].placements[0].rotated);
    // Ungrained parts may turn to fit.
    let turned = sheets(&[piece(1, 4000, 9000, 1)], options);
    assert!(turned.stock[0].placements[0].rotated);
}

#[test]
fn edge_trim_keeps_every_piece_inside_the_usable_stock() {
    let options = Settings {
        edge_trim_mm: 20.,
        allow_rotation: false,
        ..settings(1000., 800., 3.)
    };
    let cuts: Vec<_> = (0..6).map(|i| piece(i, 2000, 700, 1)).collect();
    let layout = sheets(&cuts, options);
    assert_eq!(layout.stock.len(), 1);
    for stock in &layout.stock {
        for p in &stock.placements {
            assert!(p.rect.x >= 200 && p.rect.y >= 200);
            assert!(p.rect.x + p.rect.length <= stock.length - 200);
            assert!(p.rect.y + p.rect.width <= stock.width - 200);
        }
    }
    assert!(options.validate().is_ok());
    assert!(Settings { edge_trim_mm: 500., ..options }.validate().is_err());
}

#[test]
fn edge_trim_does_not_apply_to_full_width_lumber() {
    // An 89 mm rail from 89 mm stock spans the whole board, so trim must not reject it.
    let options = Settings {
        sheet_width_mm: 89.,
        sheet_length_mm: 2440.,
        kerf_mm: 0.,
        allow_rotation: false,
        edge_trim_mm: 5.,
        ..Settings::default()
    };
    assert!(options.validate().is_ok());
    let rails = [piece(0, 24400, 890, 2)];
    let layout = sheets(&rails, options);
    assert!(layout.oversized.is_empty());
    assert_eq!(layout.stock.len(), 2);
    for stock in &layout.stock {
        assert_eq!(stock.placements[0].rect, Rect { x: 0, y: 0, length: 24400, width: 890 });
    }
}

#[test]
fn edge_trim_combined_with_kerf_keeps_pieces_inside_and_spaced() {
    let options = Settings {
        edge_trim_mm: 10.,
        kerf_mm: 3.,
        allow_rotation: false,
        ..settings(2440., 1220., 3.)
    };
    let cuts = [piece(0, 6000, 3000, 4), piece(1, 2000, 1000, 6)];
    let layout = sheets(&cuts, options);
    assert!(layout.oversized.is_empty());
    let kerf = tenths(options.kerf_mm);
    let trim = tenths(options.edge_trim_mm);
    for stock in &layout.stock {
        for (index, p) in stock.placements.iter().enumerate() {
            let r = p.rect;
            assert!(r.x >= trim && r.y >= trim);
            assert!(r.x + r.length + trim <= stock.length);
            assert!(r.y + r.width + trim <= stock.width);
            for other in &stock.placements[index + 1..] {
                let q = other.rect;
                assert!(
                    r.x + r.length + kerf <= q.x
                        || q.x + q.length + kerf <= r.x
                        || r.y + r.width + kerf <= q.y
                        || q.y + q.width + kerf <= r.y
                );
            }
        }
    }
}

#[test]
fn settings_saved_before_strategy_and_trim_keep_todays_defaults() {
    let old = r#"{"sheet_width_mm":1220.0,"sheet_length_mm":2440.0,"board_length_mm":2440.0,"kerf_mm":3.0,"allow_rotation":true}"#;
    let restored: Settings = serde_json::from_str(old).unwrap();
    assert_eq!(restored, Settings::default());
    assert_eq!(restored.strategy, Strategy::Fewest);
    assert_eq!(restored.edge_trim_mm, 0.);
}

#[test]
fn narrow_stock_layouts_keep_codes_quantities_and_kerf_for_mixed_lengths() {
    let cuts = [
        piece(5, 6000, 890, 7),
        piece(2, 21000, 890, 2),
        piece(8, 30000, 890, 3),
    ];
    let layout = sheets(&cuts, settings(2440., 89., 3.));
    assert_eq!(layout.oversized, [cuts[2]]);
    let mut counts = BTreeMap::new();
    for stock in &layout.stock {
        let mut previous_end = None;
        for p in &stock.placements {
            if let Some(end) = previous_end {
                assert!(p.rect.x >= end + 30);
            }
            assert!(p.rect.x + p.rect.length <= stock.length);
            previous_end = Some(p.rect.x + p.rect.length);
            *counts.entry(p.mark).or_insert(0) += 1;
        }
    }
    assert_eq!(counts[&5], 7);
    assert_eq!(counts[&2], 2);
}

#[test]
fn invalid_stock_settings_are_rejected() {
    for options in [
        Settings {
            kerf_mm: -1.,
            ..Settings::default()
        },
        Settings {
            sheet_width_mm: 0.,
            ..Settings::default()
        },
        Settings {
            board_length_mm: f64::NAN,
            ..Settings::default()
        },
        Settings {
            kerf_mm: f64::INFINITY,
            ..Settings::default()
        },
        Settings {
            edge_trim_mm: -1.,
            ..Settings::default()
        },
    ] {
        assert!(options.validate().is_err());
    }
    assert!(Settings::default().validate().is_ok());
}
