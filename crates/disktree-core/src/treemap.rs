//! Squarified treemap layout.
//!
//! Bruls, Huizing and van Wijk's squarified algorithm: grow a row of tiles
//! while the worst aspect ratio inside it keeps improving, then start a new row
//! in the remaining space. That is what produces the legible mosaic of
//! KDirStat-style explorers instead of the slivers a naive slice-and-dice
//! gives.
//!
//! Layout runs in pixels, in the coordinate space of an unzoomed viewport. The
//! view applies its own pan and zoom when painting, so re-layout is only needed
//! when the tree, the viewport size, or the requested depth changes.

use crate::filter::{Keep, Matches};
use crate::tree::{Metric, Node};

/// An axis-aligned rectangle in viewport pixels.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn right(&self) -> f32 {
        self.x + self.w
    }

    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }

    pub fn area(&self) -> f32 {
        (self.w.max(0.0)) * (self.h.max(0.0))
    }

    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }

    /// Shrink on every side, never past empty.
    #[must_use]
    pub fn inset(&self, padding: f32) -> Self {
        let w = padding.mul_add(-2.0, self.w).max(0.0);
        let h = padding.mul_add(-2.0, self.h).max(0.0);
        Self::new(self.x + padding, self.y + padding, w, h)
    }

    #[must_use]
    pub fn scaled(&self, scale: f32) -> Self {
        Self::new(
            self.x * scale,
            self.y * scale,
            self.w * scale,
            self.h * scale,
        )
    }

    #[must_use]
    pub fn translated(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }
}

/// What a tile stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TileKind {
    /// A real node at `crumbs`, which are child indices from the scanned root.
    Node { crumbs: Vec<usize> },
    /// The tail of a long child list, merged so its area is still accounted
    /// for instead of silently dropped.
    Others { crumbs: Vec<usize>, count: usize },
}

/// One rectangle of the mosaic.
#[derive(Clone, Debug)]
pub struct Tile {
    pub kind: TileKind,
    pub rect: Rect,
    /// Nesting level: 0 tiles are children of the current root.
    pub depth: u32,
    /// The band this directory reserved for its own name, when it is
    /// subdivided. Its children are laid out below it, so a parent's name
    /// never sits on top of a child — and the band is part of the parent for
    /// hit-testing, which makes a parent reachable without aiming at its
    /// border.
    pub header: Option<Rect>,
}

impl Tile {
    pub fn crumbs(&self) -> &[usize] {
        match &self.kind {
            TileKind::Node { crumbs } | TileKind::Others { crumbs, .. } => {
                crumbs
            }
        }
    }
}

/// How much of the tree to draw, and how finely.
#[derive(Clone, Debug, PartialEq)]
pub struct LayoutOptions {
    /// Nesting levels drawn at once; 1 draws only the root's children.
    pub max_depth: u32,
    /// Gap between siblings inside a directory, and inset into it.
    pub padding: f32,
    /// Gap between the top-level directories: wider than `padding`, so the
    /// first level of structure reads before the detail inside it.
    pub padding_outer: f32,
    /// Tiles below this many pixels are dropped; they cannot be read or hit.
    pub min_tile: f32,
    /// Children kept per directory. The tail merges into one `Others` tile.
    pub max_children: usize,
    /// Height of the band a top-level directory keeps for its name.
    pub header: f32,
    /// Height of the slimmer band a deeper directory keeps when it is drawn
    /// open. A directory drawn closed has no band: its label sits in its
    /// corner, over nothing but its own fill.
    pub header_inner: f32,
    /// Weight bytes by what each node holds alone
    /// ([`Node::private_bytes`]) instead of its whole allocation, so
    /// APFS clones that share their blocks shrink to what removing them
    /// would hand back. Only [`Metric::Bytes`] has a private size; a
    /// file count ignores this.
    pub private: bool,
}

impl Default for LayoutOptions {
    fn default() -> Self {
        Self {
            max_depth: 3,
            padding: 1.0,
            min_tile: 5.0,
            max_children: 96,
            padding_outer: 3.0,
            header: 20.0,
            header_inner: 15.0,
            private: false,
        }
    }
}

/// Lay out everything beneath `root` inside `area`.
///
/// `root_crumbs` is where `root` sits in the scanned tree. Every tile's
/// crumbs extend it, so they always address the scanned tree, never the node
/// being drawn: a caller that resolves a tile from the scanned root gets that
/// tile, at any depth.
pub fn layout(
    root: Node<'_>,
    root_crumbs: &[usize],
    area: Rect,
    metric: Metric,
    options: &LayoutOptions,
) -> Vec<Tile> {
    layout_filtered(root, root_crumbs, area, metric, options, None)
}

/// [`layout`], showing only what `filter` keeps: its matches, at the size
/// of what matched, inside ancestors sized the same way.
pub fn layout_filtered(
    root: Node<'_>,
    root_crumbs: &[usize],
    area: Rect,
    metric: Metric,
    options: &LayoutOptions,
    filter: Option<&Matches>,
) -> Vec<Tile> {
    let mut tiles = Vec::new();
    let mut crumbs = root_crumbs.to_vec();
    let place = Placement {
        metric,
        options,
        filter,
    };
    place_children(root, area, &place, 0, &mut crumbs, &mut tiles);
    tiles
}

/// What stays the same for every level of one layout.
struct Placement<'a> {
    metric: Metric,
    options: &'a LayoutOptions,
    /// Set while the level being placed is inside a filtered region; a
    /// match clears it for everything beneath.
    filter: Option<&'a Matches>,
}

fn place_children(
    node: Node<'_>,
    area: Rect,
    place: &Placement<'_>,
    depth: u32,
    crumbs: &mut Vec<usize>,
    out: &mut Vec<Tile>,
) {
    let (metric, options) = (place.metric, place.options);
    let private = options.private && metric == Metric::Bytes;
    if node.children().is_empty() || area.w <= 0.0 || area.h <= 0.0 {
        return;
    }

    // Rank by importance ourselves: the tree is already sorted, but a metric
    // switch or a hand-built tree must not produce a bad layout.
    let mut ranked: Vec<(usize, f64)> = node
        .children()
        .iter()
        .enumerate()
        .filter_map(|(index, child)| {
            let value = match place.filter {
                None if private => child.private_bytes(),
                None => child.value(metric),
                Some(filter) => {
                    crumbs.push(index);
                    let keep = filter.keep_at(crumbs, child);
                    crumbs.pop();
                    let kept = Matches::value(keep?, child, metric);
                    if private {
                        private_share(kept, child)
                    } else {
                        kept
                    }
                }
            };
            Some((index, value as f64))
        })
        .filter(|(_, value)| *value > 0.0)
        .collect();
    if ranked.is_empty() {
        return;
    }
    ranked.sort_by(|left, right| right.1.total_cmp(&left.1));

    let kept = ranked.len().min(options.max_children);
    let mut values: Vec<f64> =
        ranked[..kept].iter().map(|(_, value)| *value).collect();
    let mut sources: Vec<Option<usize>> = ranked[..kept]
        .iter()
        .map(|(index, _)| Some(*index))
        .collect();
    let tail_count = ranked.len() - kept;
    if tail_count > 0 {
        values.push(ranked[kept..].iter().map(|(_, value)| *value).sum());
        sources.push(None);
    }

    for (slot, raw) in squarify(&values, area).iter().enumerate() {
        let rect = raw.inset(if depth == 0 {
            options.padding_outer
        } else {
            options.padding
        });
        if rect.w < options.min_tile || rect.h < options.min_tile {
            continue;
        }
        let Some(index) = sources[slot] else {
            out.push(Tile {
                kind: TileKind::Others {
                    crumbs: crumbs.clone(),
                    count: tail_count,
                },
                rect,
                depth,
                header: None,
            });
            continue;
        };

        let child = node.children().at(index);
        let subdividable = child.is_dir() && depth + 1 < options.max_depth;
        // A directory that is about to be subdivided claims a header band for
        // its own name. When there is no room for one it stays whole: a name
        // drawn over its own children is worse than one level less of detail,
        // and zooming in gives it the room back.
        let header = subdividable
            .then(|| header_band(rect, options, depth))
            .flatten();
        crumbs.push(index);
        out.push(Tile {
            kind: TileKind::Node {
                crumbs: crumbs.clone(),
            },
            rect,
            depth,
            header,
        });
        if let Some(header) = header {
            let body = Rect::new(
                rect.x,
                header.bottom(),
                rect.w,
                rect.bottom() - header.bottom(),
            );
            // Beneath a match everything is shown; above one, only matches.
            let inner = Placement {
                filter: place
                    .filter
                    .filter(|filter| filter.keep(crumbs) != Some(Keep::Whole)),
                ..*place
            };
            place_children(child, body, &inner, depth + 1, crumbs, out);
        }
        crumbs.pop();
    }
}

/// `node`'s children by index, largest private size first, or `None`
/// when the tree's own order already is that rank.
///
/// Lists and menus rank by the weights the mosaic is drawn with, so under
/// private weights the first entry is the largest tile. The tree is not
/// reordered for it: crumbs, marks, history and a linked agent's view all
/// read the tree's order, and moving between On disk and Private must not
/// renumber them. Ties keep the tree's order, which is the on-disk rank,
/// so equal private sizes still read largest on disk first. `None` lets
/// a caller keep its index-is-position fast path, which is every
/// directory without clones beneath it.
pub fn private_rank(node: Node<'_>) -> Option<Vec<usize>> {
    let weights: Vec<u64> =
        node.children().iter().map(Node::private_bytes).collect();
    if weights.is_sorted_by(|left, right| left >= right) {
        return None;
    }
    let mut order: Vec<usize> = (0..weights.len()).collect();
    // Stable, so ties keep the tree's order.
    order.sort_by_key(|&index| std::cmp::Reverse(weights[index]));
    Some(order)
}

/// The private part of `kept` of `node`'s bytes.
///
/// A filter records only the bytes it kept, not which files, so they are
/// taken to be as private as the node is on the whole: exact for a node
/// kept whole, and never more than either figure.
pub fn private_share(kept: u64, node: Node<'_>) -> u64 {
    let (bytes, private) = (node.bytes, node.private_bytes());
    if kept >= bytes || bytes == 0 {
        return private.min(kept);
    }
    // In u128: two byte counts of a large volume overflow u64 multiplied.
    let share = u128::from(kept) * u128::from(private) / u128::from(bytes);
    u64::try_from(share).unwrap_or(u64::MAX)
}

/// The band a directory keeps for its name, or `None` when the tile is too
/// small to leave its children a usable area below it.
///
/// The top level gets the full header; deeper directories a slimmer band.
fn header_band(
    rect: Rect,
    options: &LayoutOptions,
    depth: u32,
) -> Option<Rect> {
    let height = if depth == 0 {
        options.header
    } else {
        options.header_inner
    };
    let body = rect.h - height;
    if rect.w < 44.0 || body < options.min_tile * 3.0 {
        return None;
    }
    Some(Rect::new(rect.x, rect.y, rect.w, height))
}

/// Split `area` into one rectangle per value, proportional to it.
///
/// Returned in the order of `values`, not in layout order.
pub fn squarify(values: &[f64], area: Rect) -> Vec<Rect> {
    let mut rects = vec![Rect::default(); values.len()];
    let total: f64 = values.iter().filter(|value| **value > 0.0).sum();
    if total <= 0.0 || area.w <= 0.0 || area.h <= 0.0 {
        return rects;
    }

    let mut order: Vec<usize> =
        (0..values.len()).filter(|i| values[*i] > 0.0).collect();
    order.sort_by(|left, right| values[*right].total_cmp(&values[*left]));

    let scale = f64::from(area.w) * f64::from(area.h) / total;
    let areas: Vec<f64> =
        order.iter().map(|index| values[*index] * scale).collect();

    let mut free = area;
    let mut start = 0;
    while start < areas.len() {
        let side = f64::from(free.w.min(free.h));
        let mut end = start + 1;
        let mut row_sum = areas[start];
        let mut row_worst = worst_ratio(&areas[start..end], row_sum, side);

        while end < areas.len() {
            let candidate_sum = row_sum + areas[end];
            let candidate_worst =
                worst_ratio(&areas[start..=end], candidate_sum, side);
            if candidate_worst > row_worst {
                break;
            }
            row_sum = candidate_sum;
            row_worst = candidate_worst;
            end += 1;
        }

        if free.w >= free.h {
            // A vertical strip on the left; tiles stack top to bottom.
            let strip_w = (row_sum / f64::from(free.h)) as f32;
            let strip_w = strip_w.min(free.w);
            let mut y = free.y;
            for index in start..end {
                let height = if strip_w > 0.0 {
                    (areas[index] / f64::from(strip_w)) as f32
                } else {
                    0.0
                };
                let height = height.min(free.bottom() - y).max(0.0);
                rects[order[index]] = Rect::new(free.x, y, strip_w, height);
                y += height;
            }
            free.x += strip_w;
            free.w -= strip_w;
        } else {
            // A horizontal strip along the top; tiles run left to right.
            let strip_h = (row_sum / f64::from(free.w)) as f32;
            let strip_h = strip_h.min(free.h);
            let mut x = free.x;
            for index in start..end {
                let width = if strip_h > 0.0 {
                    (areas[index] / f64::from(strip_h)) as f32
                } else {
                    0.0
                };
                let width = width.min(free.right() - x).max(0.0);
                rects[order[index]] = Rect::new(x, free.y, width, strip_h);
                x += width;
            }
            free.y += strip_h;
            free.h -= strip_h;
        }
        start = end;
    }
    rects
}

/// Worst (largest) aspect ratio in a row of `areas` laid along `side`.
fn worst_ratio(areas: &[f64], row_sum: f64, side: f64) -> f64 {
    if row_sum <= 0.0 || side <= 0.0 {
        return f64::INFINITY;
    }
    let thickness = row_sum / side;
    areas.iter().fold(0.0_f64, |worst, area| {
        if *area <= 0.0 || thickness <= 0.0 {
            return worst;
        }
        let other = area / thickness;
        let ratio = (thickness / other).max(other / thickness);
        worst.max(ratio)
    })
}

/// The deepest tile containing a point.
///
/// Children are emitted after their parent and are inset inside it, so the last
/// match in reverse order is the most specific one.
pub fn hit(tiles: &[Tile], x: f32, y: f32) -> Option<&Tile> {
    tiles.iter().rev().find(|tile| tile.rect.contains(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tree::NodeKind;
    use crate::tree::{Draft, Tree};

    fn file(name: &str, bytes: u64) -> Draft {
        Draft::entry(name, NodeKind::File, bytes)
    }

    fn dir(name: &str, children: Vec<Draft>) -> Draft {
        let mut node = Draft::directory(name);
        node.children = children;
        node
    }

    /// A tree whose root holds the given children, aggregated so totals are
    /// consistent before layout runs.
    fn tree(name: &str, children: Vec<Draft>) -> Tree {
        let mut node = Tree::from(dir(name, children));
        crate::tree::aggregate(&mut node, Metric::Bytes);
        node
    }

    fn area() -> Rect {
        Rect::new(0.0, 0.0, 800.0, 500.0)
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            cases: 256,
            failure_persistence: None,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// Private weights give each top-level tile its share of what the
        /// level holds alone: a tile's area, padding aside, is its share
        /// of the private total. A node holding nothing alone, however
        /// large on disk, gets no tile.
        #[test]
        fn private_weights_size_tiles_by_what_they_hold_alone(
            spec in crate::test_clones::dir(),
        ) {
            let tree = spec.tree();
            let root = tree.root();
            let options = LayoutOptions {
                max_depth: 1,
                padding_outer: 0.0,
                min_tile: 0.0,
                private: true,
                ..LayoutOptions::default()
            };
            let tiles = layout(root, &[], area(), Metric::Bytes, &options);
            let total: u64 =
                root.children().iter().map(Node::private_bytes).sum();
            let kept = root
                .children()
                .iter()
                .filter(|child| child.private_bytes() > 0)
                .count();
            proptest::prop_assert_eq!(tiles.len(), kept.min(options.max_children));
            for tile in &tiles {
                let TileKind::Node { crumbs } = &tile.kind else {
                    continue;
                };
                let node = root.resolve(crumbs).expect("tile resolves");
                let share = node.private_bytes() as f32 / total as f32;
                let drawn = tile.rect.area() / area().area();
                proptest::prop_assert!(
                    (drawn - share).abs() < 1e-3,
                    "{} drawn {drawn}, holds {share}",
                    node.name()
                );
            }
        }

        /// The private rank is a permutation of the children, largest
        /// private size first, ties in the tree's order; and `None` only
        /// where the tree's order already is exactly that. Checked at
        /// every directory, against a sort of (private size, index)
        /// pairs written out here as the oracle.
        #[test]
        fn the_private_rank_orders_by_what_each_holds_alone(
            spec in crate::test_clones::dir(),
        ) {
            let tree = spec.tree();
            let mut stack = vec![tree.root()];
            while let Some(node) = stack.pop() {
                let children = node.children();
                let mut oracle: Vec<(std::cmp::Reverse<u64>, usize)> =
                    children
                        .iter()
                        .enumerate()
                        .map(|(index, child)| {
                            (std::cmp::Reverse(child.private_bytes()), index)
                        })
                        .collect();
                oracle.sort_unstable();
                let expected: Vec<usize> =
                    oracle.into_iter().map(|(_, index)| index).collect();
                let identity: Vec<usize> = (0..children.len()).collect();
                match private_rank(node) {
                    None => proptest::prop_assert_eq!(&identity, &expected),
                    Some(order) => {
                        proptest::prop_assert_ne!(&identity, &expected);
                        proptest::prop_assert_eq!(order, expected);
                    }
                }
                stack.extend(children.iter().filter(|child| child.is_dir()));
            }
        }

        /// A filter's kept bytes are counted as private as their node is,
        /// never more than either figure, and exactly the node's private
        /// size when it is kept whole.
        #[test]
        fn a_filtered_share_of_private_never_exceeds_either_figure(
            spec in crate::test_clones::dir(),
            fraction in 0.0_f64..=1.5,
        ) {
            let tree = spec.tree();
            let root = tree.root();
            #[expect(
                clippy::cast_possible_truncation,
                clippy::cast_sign_loss,
                clippy::cast_precision_loss,
                reason = "a test's fraction of a small byte count"
            )]
            let kept = (root.bytes as f64 * fraction) as u64;
            let share = private_share(kept, root);
            proptest::prop_assert!(share <= kept);
            proptest::prop_assert!(share <= root.private_bytes());
            if kept >= root.bytes {
                proptest::prop_assert_eq!(share, root.private_bytes());
            }
        }
    }

    #[test]
    fn on_disk_weights_ignore_private_sizes() {
        use crate::sharing::{Allocation, CloneInfo};
        let mut shared = file("shared", 900);
        shared.allocation = Some(Box::new(Allocation {
            bytes: 900,
            identity: None,
            links: 1,
            followed: false,
            clone: Some(CloneInfo {
                device: 1,
                id: 7,
                references: 2,
                private_bytes: Some(0),
                data_bytes: 900,
            }),
        }));
        let root = tree("root", vec![shared, file("plain", 100)]);
        let options = LayoutOptions {
            max_depth: 1,
            ..LayoutOptions::default()
        };
        let on_disk = layout(root.root(), &[], area(), Metric::Bytes, &options);
        assert_eq!(on_disk.len(), 2, "both take space on disk");
        let private = layout(
            root.root(),
            &[],
            area(),
            Metric::Bytes,
            &LayoutOptions {
                private: true,
                ..options
            },
        );
        assert_eq!(private.len(), 1, "the full clone holds nothing alone");
        // A file count has no private size: the option changes nothing.
        let files = |private| {
            layout(
                root.root(),
                &[],
                area(),
                Metric::Files,
                &LayoutOptions { private, ..options },
            )
            .len()
        };
        assert_eq!(files(true), files(false));
    }

    #[test]
    fn squarify_fills_the_area() {
        let values = [40.0, 30.0, 20.0, 5.0, 3.0, 2.0];
        let rects = squarify(&values, area());
        let covered: f32 = rects.iter().map(Rect::area).sum();
        assert!(
            (covered - area().area()).abs() < 1.0,
            "covered {covered} of {}",
            area().area()
        );
        for rect in &rects {
            assert!(rect.x >= -0.01 && rect.y >= -0.01);
            assert!(rect.right() <= area().right() + 0.01);
            assert!(rect.bottom() <= area().bottom() + 0.01);
        }
    }

    #[test]
    fn squarify_keeps_aspect_ratios_reasonable() {
        // Same distribution as the canonical squarify example: 6, 6, 4, 3, 2, 2,
        // 1 on a 6x4 canvas.
        let values = [6.0, 6.0, 4.0, 3.0, 2.0, 2.0, 1.0];
        let rects = squarify(&values, Rect::new(0.0, 0.0, 600.0, 400.0));
        for rect in &rects {
            let ratio = (rect.w / rect.h).max(rect.h / rect.w);
            assert!(ratio <= 4.0, "aspect ratio {ratio} in {rect:?}");
        }
    }

    #[test]
    fn squarify_survives_degenerate_input() {
        assert_eq!(squarify(&[], area()).len(), 0);
        assert!(
            squarify(&[0.0, 0.0], area())
                .iter()
                .all(|r| r.area().abs() < f32::EPSILON)
        );
        assert!(
            squarify(&[1.0], Rect::new(0.0, 0.0, 0.0, 10.0))[0]
                .area()
                .abs()
                < f32::EPSILON
        );
    }

    #[test]
    fn layout_nests_children_inside_their_parent() {
        // Depth first, parent before its own children: that order is what lets
        // the view paint in sequence and hit-test in reverse.
        let root = tree(
            "root",
            vec![
                dir("big", vec![file("inside", 100), file("also", 50)]),
                file("small", 10),
            ],
        );

        let tiles = layout(
            root.root(),
            &[],
            area(),
            Metric::Bytes,
            &LayoutOptions::default(),
        );
        let crumbs: Vec<&[usize]> = tiles.iter().map(Tile::crumbs).collect();
        assert_eq!(crumbs, vec![&[0][..], &[0, 0], &[0, 1], &[1]]);

        let parent = &tiles[0];
        let child = &tiles[1];
        assert_eq!(child.depth, 1);
        assert!(parent.rect.contains(child.rect.x, child.rect.y));
        assert!(
            parent
                .rect
                .contains(child.rect.right() - 0.5, child.rect.bottom() - 0.5)
        );
    }

    #[test]
    fn layout_depth_one_stops_at_direct_children() {
        let root = tree("root", vec![dir("big", vec![file("inside", 100)])]);

        let options = LayoutOptions {
            max_depth: 1,
            ..LayoutOptions::default()
        };
        let tiles = layout(root.root(), &[], area(), Metric::Bytes, &options);
        assert_eq!(tiles.len(), 1);
        assert_eq!(tiles[0].crumbs(), &[0]);
    }

    #[test]
    fn a_subdivided_directory_keeps_a_header_for_its_own_name() {
        let root = tree(
            "root",
            vec![dir("big", vec![file("inside", 100), file("also", 50)])],
        );
        let tiles = layout(
            root.root(),
            &[],
            area(),
            Metric::Bytes,
            &LayoutOptions::default(),
        );

        let parent = tiles
            .iter()
            .find(|tile| tile.crumbs() == [0])
            .expect("the parent tile");
        let header = parent.header.expect("a parent keeps a header");
        assert!(header.w > 0.0 && header.h > 0.0);
        assert!(
            (header.y - parent.rect.y).abs() < f32::EPSILON,
            "the band sits at the top of its tile"
        );
        assert!(header.bottom() <= parent.rect.bottom());

        // Every descendant starts below the band, so no child is drawn under
        // the parent's name.
        for tile in tiles.iter().filter(|tile| tile.crumbs().len() > 1) {
            assert!(
                tile.rect.y >= header.bottom() - f32::EPSILON,
                "{:?} overlaps the header",
                tile.crumbs()
            );
        }

        // And the band belongs to the parent for hit-testing.
        let centre = (header.x + header.w / 2.0, header.y + header.h / 2.0);
        let hit_tile = hit(&tiles, centre.0, centre.1).expect("a hit");
        assert_eq!(hit_tile.crumbs(), &[0]);
    }

    #[test]
    fn a_tile_without_room_for_a_header_stays_whole() {
        let root = tree("root", vec![dir("big", vec![file("inside", 100)])]);
        let options = LayoutOptions {
            // A band this tall leaves no usable body under it.
            header: 400.0,
            min_tile: 150.0,
            ..LayoutOptions::default()
        };
        let tiles = layout(root.root(), &[], area(), Metric::Bytes, &options);
        assert_eq!(tiles.len(), 1, "the child was not subdivided");
        assert!(tiles[0].header.is_none(), "and it has no band of its own");
    }

    #[test]
    fn tile_crumbs_address_the_scanned_tree_not_the_drawn_node() {
        // Draw the node at [3, 1] of some larger tree: its tiles must be
        // [3, 1, …], or anything resolving them from the scanned root would
        // find a stranger.
        let node = tree("inner", vec![file("a", 10), file("b", 5)]);
        let tiles = layout(
            node.root(),
            &[3, 1],
            area(),
            Metric::Bytes,
            &LayoutOptions::default(),
        );
        assert!(!tiles.is_empty());
        for tile in &tiles {
            assert!(tile.crumbs().starts_with(&[3, 1]), "{:?}", tile.crumbs());
            assert_eq!(tile.crumbs().len(), 3);
        }
    }

    #[test]
    fn a_leaf_never_claims_a_header() {
        let root = tree("root", vec![file("solo", 10)]);
        let tiles = layout(
            root.root(),
            &[],
            area(),
            Metric::Bytes,
            &LayoutOptions::default(),
        );
        assert!(tiles.iter().all(|tile| tile.header.is_none()));
    }

    #[test]
    fn layout_merges_the_child_tail_into_one_tile() {
        let children: Vec<Draft> = (0..10)
            .map(|index| file(&format!("f{index}"), 10 - index))
            .collect();
        let root = tree("root", children);
        let options = LayoutOptions {
            max_children: 4,
            ..LayoutOptions::default()
        };
        let tiles = layout(root.root(), &[], area(), Metric::Bytes, &options);
        let others: Vec<&Tile> = tiles
            .iter()
            .filter(|tile| matches!(tile.kind, TileKind::Others { .. }))
            .collect();
        assert_eq!(others.len(), 1);
        assert!(matches!(others[0].kind, TileKind::Others { count: 6, .. }));
    }

    #[test]
    fn hit_returns_the_deepest_tile() {
        let root = tree(
            "root",
            vec![dir("big", vec![file("inside", 100)]), file("small", 1)],
        );

        let tiles = layout(
            root.root(),
            &[],
            area(),
            Metric::Bytes,
            &LayoutOptions::default(),
        );
        let child = tiles
            .iter()
            .find(|tile| tile.crumbs() == [0, 0])
            .expect("child tile");
        let centre = (
            child.rect.x + child.rect.w / 2.0,
            child.rect.y + child.rect.h / 2.0,
        );
        let found = hit(&tiles, centre.0, centre.1).expect("a hit");
        assert_eq!(found.crumbs(), &[0, 0]);
        assert!(hit(&tiles, -50.0, -50.0).is_none());
    }

    #[test]
    fn a_filtered_layout_shows_only_the_matches_at_their_size() {
        use crate::filter::filter;
        use crate::tree::{NodeKind, aggregate};

        let file =
            |name: &str, bytes| Draft::entry(name, NodeKind::File, bytes);
        let mut src = Draft::directory("src");
        src.children = vec![file("big_test.rs", 300), file("other.rs", 700)];
        let mut root = Draft::directory("root");
        root.children =
            vec![src, file("unit_test.txt", 100), file("huge.iso", 5000)];
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        let root = root.root();

        let matches = filter(root, &[], "test").expect("needle");
        let area = Rect::new(0.0, 0.0, 400.0, 400.0);
        let options = LayoutOptions {
            padding: 0.0,
            padding_outer: 0.0,
            ..LayoutOptions::default()
        };
        let tiles = layout_filtered(
            root,
            &[],
            area,
            Metric::Bytes,
            &options,
            Some(&matches),
        );
        let names: Vec<String> = tiles
            .iter()
            .filter_map(|tile| root.resolve(tile.crumbs()))
            .map(|node| node.name().to_string())
            .collect();
        assert!(names.contains(&"big_test.rs".to_string()));
        assert!(names.contains(&"unit_test.txt".to_string()));
        assert!(
            !names
                .iter()
                .any(|name| name == "huge.iso" || name == "other.rs")
        );

        // src is sized by its 300 matched bytes, not its 1000: three times
        // the 100-byte match beside it.
        let area_of = |name: &str| {
            tiles
                .iter()
                .find(|tile| {
                    root.resolve(tile.crumbs())
                        .is_some_and(|node| node.name() == name)
                })
                .map(|tile| tile.rect.w * tile.rect.h)
                .expect("drawn")
        };
        let ratio = area_of("src") / area_of("unit_test.txt");
        assert!((ratio - 3.0).abs() < 0.05, "ratio {ratio}");
    }

    /// A rule's files are not recorded in the matches, so the layout must
    /// judge them itself: a partly kept folder still shows its passing
    /// files, and only those.
    #[test]
    fn a_rule_lays_out_the_passing_files_of_a_partial_folder() {
        use crate::classify::Category;
        use crate::filter::{Rule, filter_with};
        use crate::tree::{NodeKind, aggregate};

        let file = |name: &str, bytes, category| {
            let mut node = Draft::entry(name, NodeKind::File, bytes);
            node.category = category;
            node
        };
        let mut mixed = Draft::directory("mixed");
        mixed.children = vec![
            file("lib.rs", 300, Category::Code),
            file("cat.jpg", 700, Category::Media),
        ];
        let mut root = Draft::directory("root");
        root.children = vec![mixed, file("main.rs", 100, Category::Code)];
        let mut root = Tree::from(root);
        aggregate(&mut root, Metric::Bytes);
        let root = root.root();

        let code = Rule {
            only: Rule::bit(Category::Code),
            ..Rule::default()
        };
        let matches = filter_with(root, &[], "", Some(&code)).expect("rule");
        let options = LayoutOptions {
            padding: 0.0,
            padding_outer: 0.0,
            ..LayoutOptions::default()
        };
        let tiles = layout_filtered(
            root,
            &[],
            Rect::new(0.0, 0.0, 400.0, 400.0),
            Metric::Bytes,
            &options,
            Some(&matches),
        );
        let names: Vec<&str> = tiles
            .iter()
            .filter_map(|tile| root.resolve(tile.crumbs()))
            .map(crate::tree::Node::name)
            .collect();
        assert_eq!(names.len(), 3, "{names:?}");
        assert!(names.contains(&"lib.rs") && names.contains(&"main.rs"));
        assert!(!names.contains(&"cat.jpg"));
    }
}
