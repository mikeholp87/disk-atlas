use diskatlas_scanner::Scan;
use std::{
    cmp::Reverse,
    path::{Path, PathBuf},
};

#[derive(Default)]
pub struct LargestFiles {
    pub apparent: Vec<usize>,
    pub allocated: Vec<usize>,
}

impl LargestFiles {
    pub fn new(scan: &Scan) -> Self {
        let mut apparent: Vec<_> = scan
            .nodes
            .iter()
            .enumerate()
            .filter(|(_, node)| node.regular_file)
            .map(|(id, _)| id)
            .collect();
        let mut allocated = apparent.clone();
        apparent.sort_unstable_by_key(|&id| (Reverse(scan.nodes[id].apparent), id));
        allocated.sort_unstable_by_key(|&id| (Reverse(scan.nodes[id].allocated), id));
        Self {
            apparent,
            allocated,
        }
    }

    pub fn filter(&self, scan: &Scan, allocated: bool, query: &str, minimum: u64) -> Vec<usize> {
        let query = query.to_lowercase();
        let order = if allocated {
            &self.allocated
        } else {
            &self.apparent
        };
        order
            .iter()
            .copied()
            .filter(|&id| {
                let node = &scan.nodes[id];
                let size = if allocated {
                    node.allocated
                } else {
                    node.apparent
                };
                size >= minimum
                    && (query.is_empty()
                        || node.name.to_string_lossy().to_lowercase().contains(&query))
            })
            .collect()
    }
}

/// Reconstruct a lossless path from the scan root and component names.
pub fn node_path(scan: &Scan, root: &Path, mut id: usize) -> PathBuf {
    let mut components = Vec::new();
    while let Some(parent) = scan.nodes[id].parent {
        components.push(&scan.nodes[id].name);
        id = parent;
    }
    let mut path = root.to_path_buf();
    for name in components.into_iter().rev() {
        path.push(name);
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;
    use diskatlas_scanner::Node;

    #[test]
    fn sizes_filter_regular_files_and_keep_stable_ties() {
        let make = |name: &str, directory, apparent, allocated| Node {
            name: name.into(),
            parent: if name == "root" { None } else { Some(0) },
            directory,
            regular_file: !directory,
            apparent,
            allocated,
        };
        let scan = Scan {
            nodes: vec![
                make("root", true, 110, 20),
                make("VIDEO.mp4", false, 100, 0),
                make("notes", false, 5, 10),
                make("notes2", false, 5, 10),
            ],
            ..Scan::default()
        };
        let index = LargestFiles::new(&scan);
        assert_eq!(index.filter(&scan, false, "", 0), vec![1, 2, 3]);
        assert_eq!(index.filter(&scan, true, "", 1), vec![2, 3]);
        assert_eq!(index.filter(&scan, false, "video", 50), vec![1]);
        assert!(index.filter(&scan, false, "video", 101).is_empty());
    }

    /// Manual benchmark: `DISKATLAS_BENCH_PATH=/usr cargo test --release -- --ignored --nocapture bench_real_tree`
    #[test]
    #[ignore]
    fn bench_real_tree() {
        use std::{sync::atomic::AtomicBool, time::Instant};
        let path = std::env::var("DISKATLAS_BENCH_PATH").expect("set DISKATLAS_BENCH_PATH");
        let t = Instant::now();
        let scan = diskatlas_scanner::scan(Path::new(&path), &AtomicBool::new(false));
        println!(
            "scan            {:>9.1?}  {} nodes",
            t.elapsed(),
            scan.nodes.len()
        );
        let t = Instant::now();
        let children = crate::chart::index(&scan);
        println!("chart::index    {:>9.1?}", t.elapsed());
        let t = Instant::now();
        let index = LargestFiles::new(&scan);
        println!(
            "LargestFiles    {:>9.1?}  {} files",
            t.elapsed(),
            index.apparent.len()
        );
        let root_children = &children[0];
        let t = Instant::now();
        let tiles = crate::chart::treemap(&scan, root_children);
        println!(
            "treemap(root)   {:>9.1?}  {} tiles",
            t.elapsed(),
            tiles.len()
        );
        for (label, allocated, query, min) in [
            ("filter empty", false, "", 0),
            ("filter min 1MB", false, "", 1 << 20),
            ("filter 'a'", false, "a", 0),
            ("filter 'zzzz'", false, "zzzz", 0),
            ("filter 'zzzz' alloc", true, "zzzz", 0),
        ] {
            let mut times = Vec::new();
            let mut hits = 0;
            for _ in 0..5 {
                let t = Instant::now();
                hits = index.filter(&scan, allocated, query, min).len();
                times.push(t.elapsed());
            }
            times.sort();
            println!("{label:<20}{:>9.1?} median  {hits} hits", times[2]);
        }
    }

    #[test]
    fn paths_preserve_non_utf8_components() {
        use std::os::unix::ffi::OsStringExt;
        let node = |name, parent| Node {
            name,
            parent,
            directory: false,
            regular_file: true,
            apparent: 0,
            allocated: 0,
        };
        let name = std::ffi::OsString::from_vec(vec![b'x', 0xff]);
        let scan = Scan {
            nodes: vec![
                node("root".into(), None),
                node("dir".into(), Some(0)),
                node(name.clone(), Some(1)),
            ],
            ..Scan::default()
        };
        assert_eq!(
            node_path(&scan, Path::new("/tmp/root"), 2),
            PathBuf::from("/tmp/root/dir").join(name)
        );
    }
}
