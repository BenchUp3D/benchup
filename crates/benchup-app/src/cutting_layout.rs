//! Deterministic, kerf-aware guillotine stock packing with edge trim.
//! Every strategy only makes straight, edge-to-edge cuts.
use benchup_core::tr;
use std::{cmp::Reverse, collections::BTreeMap};

/// Factory edges exist only on sheet goods, so narrower stock (dimensional lumber) is never trimmed.
const SHEET_MIN_MM: f64 = 600.;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Strategy {
    /// Best of several piece orders and split rules, keeping the fewest sheets.
    #[default]
    Fewest,
    /// Full-length rips along the stock length first, then crosscuts inside each strip.
    RipFirst,
    /// Fewest sheets, preferring the layout with the largest reusable offcut.
    LargestOffcut,
}

impl Strategy {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Fewest => tr!("cutting_layout.strategy_fewest_sheets"),
            Self::RipFirst => tr!("cutting_layout.strategy_rip_first"),
            Self::LargestOffcut => tr!("cutting_layout.strategy_largest_offcut"),
        }
    }

    pub(crate) fn hint(self) -> &'static str {
        match self {
            Self::Fewest => tr!("cutting_layout.strategy_fewest_sheets_hint"),
            Self::RipFirst => tr!("cutting_layout.strategy_rip_first_hint"),
            Self::LargestOffcut => tr!("cutting_layout.strategy_largest_offcut_hint"),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(crate) struct Settings {
    pub sheet_width_mm: f64,
    pub sheet_length_mm: f64,
    pub board_length_mm: f64,
    pub kerf_mm: f64,
    pub allow_rotation: bool,
    pub respect_grain: bool,
    pub show_joinery: bool,
    pub strategy: Strategy,
    /// Unusable factory edge removed from each side of sheet goods.
    pub edge_trim_mm: f64,
}
impl Default for Settings {
    fn default() -> Self {
        Self {
            sheet_width_mm: 1220.,
            sheet_length_mm: 2440.,
            board_length_mm: 2440.,
            kerf_mm: 3.,
            allow_rotation: true,
            respect_grain: true,
            show_joinery: true,
            strategy: Strategy::Fewest,
            edge_trim_mm: 0.,
        }
    }
}
impl Settings {
    pub fn validate(self) -> Result<(), String> {
        if [
            self.sheet_width_mm,
            self.sheet_length_mm,
            self.board_length_mm,
        ]
        .iter()
        .any(|v| !v.is_finite() || !(1. ..=1_000_000.).contains(v))
            || !self.kerf_mm.is_finite()
            || !(0. ..=100.).contains(&self.kerf_mm)
        {
            return Err(tr!("cutting_layout.stock_dimensions_must_be_1_1_000_000").into());
        }
        if !(0. ..=100.).contains(&self.edge_trim_mm) {
            return Err(tr!("cutting_layout.edge_trim_must_be_0_to_100_mm").into());
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Piece {
    pub mark: usize,
    /// In tenths of a millimetre. X is length, Y is width.
    pub length: u64,
    pub width: u64,
    pub quantity: usize,
    pub grain_along_length: bool,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Rect {
    pub x: u64,
    pub y: u64,
    pub length: u64,
    pub width: u64,
}
#[derive(Clone, Copy, Debug)]
pub(crate) struct Placement {
    pub mark: usize,
    pub rect: Rect,
    pub rotated: bool,
    pub part_index: usize,
}
#[derive(Clone, Copy, Debug)]
struct Free {
    rect: Rect,
    /// A band from a rip that may only be crosscut, never ripped again.
    strip: bool,
}
#[derive(Debug)]
pub(crate) struct Stock {
    pub length: u64,
    pub width: u64,
    pub placements: Vec<Placement>,
    usable: Rect,
    free: Vec<Free>,
}
impl Stock {
    fn new(length: u64, width: u64, trim: u64) -> Self {
        let usable = Rect {
            x: trim,
            y: trim,
            length: length.saturating_sub(2 * trim),
            width: width.saturating_sub(2 * trim),
        };
        Self {
            length,
            width,
            placements: Vec::new(),
            usable,
            free: vec![Free { rect: usable, strip: false }],
        }
    }
}
#[derive(Debug, Default)]
pub(crate) struct Layout {
    pub stock: Vec<Stock>,
    pub oversized: Vec<Piece>,
}

pub(crate) fn tenths(mm: f64) -> u64 {
    (mm * 10.).round() as u64
}

#[derive(Clone, Copy)]
enum Order {
    Area,
    Length,
    Width,
}

#[derive(Clone, Copy)]
enum Split {
    /// The leftover strip runs the full length; the piece's band keeps its width.
    Strips,
    /// The leftover runs the full width; the piece's band keeps its length.
    Crosscut,
    /// Rips run along the stock length across the full sheet; their bands are only crosscut.
    RipFirst,
}

/// Each split is a full straight cut across the current free rectangle. Kerf is
/// reserved between retained pieces; a cut at the stock edge needs no extra gap.
pub(crate) fn sheets(pieces: &[Piece], settings: Settings) -> Layout {
    let settings = if settings.sheet_width_mm.min(settings.sheet_length_mm) < SHEET_MIN_MM {
        Settings {
            edge_trim_mm: 0.,
            ..settings
        }
    } else {
        settings
    };
    let candidates: Vec<(Order, Split)> = match settings.strategy {
        Strategy::RipFirst => vec![(Order::Width, Split::RipFirst)],
        Strategy::Fewest | Strategy::LargestOffcut => vec![
            (Order::Area, Split::Strips),
            (Order::Length, Split::Strips),
            (Order::Width, Split::Strips),
            (Order::Area, Split::Crosscut),
            (Order::Length, Split::Crosscut),
            (Order::Width, Split::Crosscut),
        ],
    };
    let kerf = tenths(settings.kerf_mm);
    let mut best: Option<((u64, u64), Layout)> = None;
    for (order, split) in candidates {
        let layout = pack(pieces, settings, order, split);
        let score = layout.score(settings.strategy, kerf);
        if best.as_ref().is_none_or(|(previous, _)| score < *previous) {
            best = Some((score, layout));
        }
    }
    best.map(|(_, layout)| layout).unwrap_or_default()
}

fn pack(pieces: &[Piece], settings: Settings, order: Order, split: Split) -> Layout {
    let length = tenths(settings.sheet_length_mm);
    let width = tenths(settings.sheet_width_mm);
    let trim = tenths(settings.edge_trim_mm);
    let kerf = tenths(settings.kerf_mm);
    let usable = (length.saturating_sub(2 * trim), width.saturating_sub(2 * trim));
    let mut pieces = pieces.to_vec();
    pieces.sort_by_key(|p| match order {
        Order::Area => (
            Reverse(p.length as u128 * p.width as u128),
            Reverse(p.length),
            p.mark,
        ),
        Order::Length => (Reverse(p.length as u128), Reverse(p.width), p.mark),
        Order::Width => (Reverse(p.width as u128), Reverse(p.length), p.mark),
    });
    let mut layout = Layout::default();
    for piece in pieces {
        let fits: Vec<_> = orientations(&piece, settings)
            .into_iter()
            .filter(|(l, w, _)| *l <= usable.0 && *w <= usable.1)
            .collect();
        if fits.is_empty() {
            layout.oversized.push(piece);
            continue;
        }
        for part_index in 0..piece.quantity {
            // Best-area-fit among the remaining guillotine rectangles.
            let mut best = None;
            for (stock_index, stock) in layout.stock.iter().enumerate() {
                for (free_index, free) in stock.free.iter().enumerate() {
                    for &(l, w, rotated) in &fits {
                        if l > free.rect.length || w > free.rect.width {
                            continue;
                        }
                        let score = (
                            free.rect.length * free.rect.width - l * w,
                            (free.rect.length - l).min(free.rect.width - w),
                        );
                        if best
                            .as_ref()
                            .is_none_or(|(_, _, _, _, _, previous)| score < *previous)
                        {
                            best = Some((stock_index, free_index, l, w, rotated, score));
                        }
                    }
                }
            }
            let (stock_index, free_index, l, w, rotated) = match best {
                Some((s, f, l, w, r, _)) => (s, f, l, w, r),
                None => {
                    let &(l, w, r) = &fits[0];
                    layout.stock.push(Stock::new(length, width, trim));
                    (layout.stock.len() - 1, 0, l, w, r)
                }
            };
            let stock = &mut layout.stock[stock_index];
            let free = stock.free.remove(free_index);
            stock.placements.push(Placement {
                mark: piece.mark,
                rect: Rect {
                    x: free.rect.x,
                    y: free.rect.y,
                    length: l,
                    width: w,
                },
                rotated,
                part_index,
            });
            stock.free.extend(leftovers(free, l, w, kerf, split));
        }
    }
    layout
}

/// Free rectangles left after cutting an l x w piece from the corner of `free`.
fn leftovers(free: Free, l: u64, w: u64, kerf: u64, split: Split) -> Vec<Free> {
    let Rect { x, y, length, width } = free.rect;
    let right_fits = length > l + kerf;
    let top_fits = width > w + kerf;
    let right = |band: u64, strip: bool| Free {
        rect: Rect {
            x: x + l + kerf,
            y,
            length: length - l - kerf,
            width: band,
        },
        strip,
    };
    let top = |run: u64, strip: bool| Free {
        rect: Rect {
            x,
            y: y + w + kerf,
            length: run,
            width: width - w - kerf,
        },
        strip,
    };
    let mut out = Vec::with_capacity(2);
    match split {
        Split::Strips => {
            if right_fits {
                out.push(right(w, false));
            }
            if top_fits {
                out.push(top(length, false));
            }
        }
        Split::Crosscut => {
            if right_fits {
                out.push(right(width, false));
            }
            if top_fits {
                out.push(top(l, false));
            }
        }
        Split::RipFirst => {
            if free.strip {
                if right_fits {
                    out.push(right(width, true));
                }
            } else {
                if right_fits {
                    out.push(right(w, true));
                }
                if top_fits {
                    out.push(top(length, false));
                }
            }
        }
    }
    out
}

/// Grained parts keep their length along the stock length when grain is respected.
fn orientations(piece: &Piece, settings: Settings) -> Vec<(u64, u64, bool)> {
    let unrotated = (piece.length, piece.width, false);
    if settings.allow_rotation && !(settings.respect_grain && piece.grain_along_length) {
        vec![unrotated, (piece.width, piece.length, true)]
    } else {
        vec![unrotated]
    }
}

/// The largest empty rectangle on a sheet, with each cut's kerf counted as lost.
/// Exhaustive over the placement grid, O(rows^2 * cols) per sheet. Fine for shop-sized layouts; a maximal-rectangle sweep would be needed if sheets get huge.
fn largest_offcut(stock: &Stock, kerf: u64) -> u64 {
    let Rect { x: x0, y: y0, length, width } = stock.usable;
    let (x1, y1) = (x0 + length, y0 + width);
    let blocks: Vec<[u64; 4]> = stock
        .placements
        .iter()
        .map(|p| {
            let r = p.rect;
            [r.x, r.y, (r.x + r.length + kerf).min(x1), (r.y + r.width + kerf).min(y1)]
        })
        .collect();
    let mut xs = vec![x0, x1];
    let mut ys = vec![y0, y1];
    for [a, b, c, d] in &blocks {
        xs.extend([*a, *c]);
        ys.extend([*b, *d]);
    }
    xs.sort_unstable();
    xs.dedup();
    ys.sort_unstable();
    ys.dedup();
    let (cols, rows) = (xs.len() - 1, ys.len() - 1);
    let mut occupied = vec![vec![false; cols]; rows];
    for [a, b, c, d] in &blocks {
        for (row, cells) in occupied.iter_mut().enumerate() {
            if ys[row] < *b || ys[row + 1] > *d {
                continue;
            }
            for (col, cell) in cells.iter_mut().enumerate() {
                if xs[col] >= *a && xs[col + 1] <= *c {
                    *cell = true;
                }
            }
        }
    }
    // Every row band from `top` to `bottom` is tested as a stack of free columns.
    let mut best = 0;
    for top in 0..rows {
        let mut free = vec![true; cols];
        for bottom in top..rows {
            for (col, cell) in free.iter_mut().enumerate() {
                *cell &= !occupied[bottom][col];
            }
            let height = ys[bottom + 1] - ys[top];
            let mut run = None;
            for col in 0..=cols {
                match (col < cols && free[col], run) {
                    (true, None) => run = Some(col),
                    (false, Some(start)) => {
                        best = best.max((xs[col] - xs[start]) * height);
                        run = None;
                    }
                    _ => {}
                }
            }
        }
    }
    best
}

impl Layout {
    pub fn oversized_marks(&self) -> BTreeMap<usize, usize> {
        self.oversized
            .iter()
            .map(|p| (p.mark, p.quantity))
            .collect()
    }

    /// Lower is better: sheet count first, then either the number of offcut
    /// fragments or, for the largest-offcut strategy, the biggest reusable offcut.
    fn score(&self, strategy: Strategy, kerf: u64) -> (u64, u64) {
        let sheets = self.stock.len() as u64;
        match strategy {
            Strategy::LargestOffcut => {
                let largest = self
                    .stock
                    .iter()
                    .map(|stock| largest_offcut(stock, kerf))
                    .max()
                    .unwrap_or(0);
                (sheets, u64::MAX - largest)
            }
            _ => (
                sheets,
                self.stock.iter().map(|stock| stock.free.len() as u64).sum(),
            ),
        }
    }
}

#[cfg(test)]
#[path = "cutting_layout_tests.rs"]
mod tests;
