use diskatlas_scanner::Scan;

pub struct Segment {
    /// The node, or for a group of small items, their parent folder.
    pub id: usize,
    pub depth: usize,
    pub start: f32,
    pub end: f32,
    pub size: u64,
    /// Nonzero: this segment stands for that many items too small to draw alone.
    pub hidden: usize,
}

/// Items spanning less than this (radians, about 0.8 degrees) fold into a group.
const MIN_ANGLE: f32 = 0.014;

/// A treemap tile in normalized coordinates.
#[derive(Debug)]
pub struct Tile {
    pub id: usize,
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// Split sorted sibling groups along the longest axis, balancing their byte totals.
pub fn treemap(scan: &Scan, children: &[usize]) -> Vec<Tile> {
    let mut ids: Vec<_> = children
        .iter()
        .copied()
        .filter(|&id| scan.nodes[id].apparent > 0)
        .collect();
    ids.sort_unstable_by_key(|&id| (std::cmp::Reverse(scan.nodes[id].apparent), id));
    let mut tiles = Vec::with_capacity(ids.len());
    let mut pending = vec![(ids.as_slice(), [0.0_f64; 2], [1.0_f64; 2])];
    while let Some((group, min, max)) = pending.pop() {
        if group.is_empty() {
            continue;
        }
        if group.len() == 1 {
            tiles.push(Tile {
                id: group[0],
                min,
                max,
            });
            continue;
        }
        let total: u128 = group
            .iter()
            .map(|&id| u128::from(scan.nodes[id].apparent))
            .sum();
        let mut split = 1;
        let mut left = u128::from(scan.nodes[group[0]].apparent);
        while split < group.len() - 1 && left < total / 2 {
            left += u128::from(scan.nodes[group[split]].apparent);
            split += 1;
        }
        let axis = usize::from(max[1] - min[1] > max[0] - min[0]);
        let boundary = min[axis] + (max[axis] - min[axis]) * left as f64 / total as f64;
        let mut left_max = max;
        left_max[axis] = boundary;
        let mut right_min = min;
        right_min[axis] = boundary;
        pending.push((&group[split..], right_min, max));
        pending.push((&group[..split], min, left_max));
    }
    tiles
}

/// Lay out visible descendants in proportion to their parent's apparent size.
pub fn index(scan: &Scan) -> Vec<Vec<usize>> {
    let mut children = vec![Vec::new(); scan.nodes.len()];
    for (id, node) in scan.nodes.iter().enumerate() {
        if let Some(parent) = node.parent {
            children[parent].push(id);
        }
    }
    children
}

/// Lay out descendants as rings, largest first, in proportion to their parent's apparent size.
pub fn layout(scan: &Scan, children: &[Vec<usize>], root: usize, max_depth: usize) -> Vec<Segment> {
    let mut result = Vec::new();
    let mut pending = vec![(root, 0, 0.0_f32, std::f32::consts::TAU)];
    while let Some((parent, depth, start, end)) = pending.pop() {
        if depth >= max_depth {
            continue;
        }
        let total = scan.nodes[parent].apparent;
        if total == 0 {
            continue;
        }
        let mut kids: Vec<usize> = children[parent]
            .iter()
            .copied()
            .filter(|&id| scan.nodes[id].apparent > 0)
            .collect();
        kids.sort_unstable_by_key(|&id| (std::cmp::Reverse(scan.nodes[id].apparent), id));
        let span = |size: u64| (end - start) * (size as f64 / total as f64) as f32;
        let mut angle = start;
        let (mut small, mut small_size) = (0, 0_u64);
        for id in kids {
            let size = scan.nodes[id].apparent;
            if span(size) < MIN_ANGLE {
                small += 1;
                small_size = small_size.saturating_add(size);
                continue;
            }
            let next = angle + span(size);
            result.push(Segment {
                id,
                depth,
                start: angle,
                end: next,
                size,
                hidden: 0,
            });
            if scan.nodes[id].directory {
                pending.push((id, depth + 1, angle, next));
            }
            angle = next;
        }
        if small > 0 && span(small_size) >= MIN_ANGLE / 4.0 {
            let end = angle + span(small_size);
            result.push(Segment {
                id: parent,
                depth,
                start: angle,
                end,
                size: small_size,
                hidden: small,
            });
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use diskatlas_scanner::Node;
    #[test]
    fn treemap_preserves_area_and_never_overlaps() {
        let scan = Scan {
            nodes: [0, 3, 1, 8, 2, u64::MAX]
                .into_iter()
                .map(|apparent| Node {
                    name: "x".into(),
                    parent: None,
                    directory: false,
                    regular_file: true,
                    apparent,
                    allocated: 0,
                })
                .collect(),
            ..Scan::default()
        };
        let tiles = treemap(&scan, &[0, 1, 2, 3, 4]);
        assert_eq!(tiles.len(), 4);
        for a in &tiles {
            let area = (a.max[0] - a.min[0]) * (a.max[1] - a.min[1]);
            assert!((area - scan.nodes[a.id].apparent as f64 / 14.0).abs() < 1e-12);
            assert!(
                a.min
                    .iter()
                    .chain(a.max.iter())
                    .all(|v| v.is_finite() && (0.0..=1.0).contains(v))
            );
            for b in &tiles {
                if a.id != b.id {
                    assert!(
                        a.max[0] <= b.min[0]
                            || b.max[0] <= a.min[0]
                            || a.max[1] <= b.min[1]
                            || b.max[1] <= a.min[1]
                    );
                }
            }
        }
        assert!(treemap(&scan, &[0]).is_empty());
        assert_eq!(treemap(&scan, &[5, 5]).len(), 2);
    }
    #[test]
    fn zero_sizes_and_proportions() {
        let node = |parent, apparent| Node {
            name: "x".into(),
            parent,
            directory: true,
            regular_file: false,
            apparent,
            allocated: 0,
        };
        let scan = Scan {
            nodes: vec![
                node(None, 4),
                node(Some(0), 3),
                node(Some(0), 1),
                node(Some(0), 0),
            ],
            ..Scan::default()
        };
        let segments = layout(&scan, &index(&scan), 0, 2);
        assert_eq!(segments.len(), 2);
        assert!((segments[0].end / std::f32::consts::TAU - 0.75).abs() < 0.0001);
        assert!((segments[1].end - std::f32::consts::TAU).abs() < 0.0001);
    }
    #[test]
    fn tiny_items_fold_into_one_group() {
        let node = |parent, apparent| Node {
            name: "x".into(),
            parent,
            directory: false,
            regular_file: true,
            apparent,
            allocated: 0,
        };
        let mut nodes = vec![node(None, 10_000), node(Some(0), 9_000)];
        nodes.extend((0..100).map(|_| node(Some(0), 10)));
        let scan = Scan {
            nodes,
            ..Scan::default()
        };
        let segments = layout(&scan, &index(&scan), 0, 1);
        assert_eq!(segments.len(), 2);
        assert_eq!((segments[1].hidden, segments[1].size), (100, 1_000));
        assert!(segments[0].end <= segments[1].start + 1e-6);
    }
}
