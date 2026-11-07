//! Read-only filesystem traversal.
use std::{
    collections::HashSet,
    ffi::OsString,
    os::unix::fs::MetadataExt,
    path::Path,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use walkdir::WalkDir;

/// A filesystem entry. Parent indices refer to earlier entries in the scan.
#[derive(Debug)]
pub struct Node {
    pub name: OsString,
    pub parent: Option<usize>,
    pub directory: bool,
    pub regular_file: bool,
    pub apparent: u64,
    pub allocated: u64,
}

/// A partial result is returned on cancellation or individual filesystem errors.
#[derive(Debug, Default)]
pub struct Scan {
    pub nodes: Vec<Node>,
    pub errors: Vec<String>,
    pub cancelled: bool,
    /// Extra hard links to an already-counted file; they count 0 bytes (like `du`).
    pub hard_links_skipped: usize,
}

/// Scan without following symbolic links or crossing device boundaries.
pub fn scan(path: &Path, cancel: &AtomicBool) -> Scan {
    scan_with_progress(path, cancel, &AtomicUsize::new(0))
}

/// Like [`scan`], and publishes the number of entries found so far to `progress`.
pub fn scan_with_progress(path: &Path, cancel: &AtomicBool, progress: &AtomicUsize) -> Scan {
    let mut result = Scan::default();
    let mut parents = Vec::new();
    // ponytail: in-memory set of multi-link inodes; fine until millions of linked files.
    let mut seen_links = HashSet::new();
    for item in WalkDir::new(path)
        .follow_links(false)
        // A symlinked root is deliberately followed so `/var/foo -> /data` scans /data;
        // every symlink below the root is recorded but never entered.
        .follow_root_links(true)
        .same_file_system(true)
        .max_open(32)
    {
        if cancel.load(Ordering::Relaxed) {
            result.cancelled = true;
            break;
        }
        let entry = match item {
            Ok(e) => e,
            Err(e) => {
                result.errors.push(e.to_string());
                continue;
            }
        };
        // Keep the node even if metadata fails: skipping a directory would leave
        // `parents` short and attach its children to the wrong ancestor.
        // The root is stat'ed through symlinks (walkdir descends into it but lstat's it).
        let metadata = if entry.depth() == 0 {
            std::fs::metadata(entry.path())
        } else {
            entry.metadata().map_err(Into::into)
        }
        .map_err(|e| result.errors.push(e.to_string()))
        .ok();
        // Prefer metadata's type: for a followed root symlink it describes the target.
        let file_type = metadata
            .as_ref()
            .map_or(entry.file_type(), |m| m.file_type());
        let depth = entry.depth();
        parents.truncate(depth);
        debug_assert_eq!(parents.len(), depth, "every traversed ancestor is indexed");
        let parent = parents.last().copied();
        // Count each multi-linked file once: the first path seen owns the bytes.
        let duplicate_link = metadata.as_ref().is_some_and(|m| {
            file_type.is_file() && m.nlink() > 1 && !seen_links.insert((m.dev(), m.ino()))
        });
        if duplicate_link {
            result.hard_links_skipped += 1;
        }
        let id = result.nodes.len();
        result.nodes.push(Node {
            name: entry.file_name().to_os_string(),
            parent,
            directory: file_type.is_dir(),
            regular_file: file_type.is_file(),
            apparent: match &metadata {
                Some(m) if file_type.is_file() && !duplicate_link => m.len(),
                _ => 0,
            },
            allocated: match &metadata {
                Some(m) if !duplicate_link => m.blocks().saturating_mul(512),
                _ => 0,
            },
        });
        progress.store(id + 1, Ordering::Relaxed);
        if file_type.is_dir() {
            parents.push(id);
        }
    }
    for id in (0..result.nodes.len()).rev() {
        if let Some(parent) = result.nodes[id].parent {
            result.nodes[parent].apparent = result.nodes[parent]
                .apparent
                .saturating_add(result.nodes[id].apparent);
            result.nodes[parent].allocated = result.nodes[parent]
                .allocated
                .saturating_add(result.nodes[id].allocated);
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, os::unix::fs::symlink, path::PathBuf};

    struct Fixture(PathBuf);
    impl Fixture {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("diskatlas-{name}-{}", std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn find(scan: &Scan, name: &str) -> usize {
        scan.nodes.iter().position(|n| n.name == name).unwrap()
    }

    #[test]
    fn missing_root_reports_error() {
        let result = scan(
            Path::new("/definitely-not-a-diskatlas-test-path"),
            &AtomicBool::new(false),
        );
        assert!(!result.errors.is_empty());
    }
    #[test]
    fn cancellation_is_recorded() {
        let result = scan(Path::new("."), &AtomicBool::new(true));
        assert!(result.cancelled);
    }

    #[test]
    fn hierarchy_and_sizes() {
        let f = Fixture::new("tree");
        fs::create_dir_all(f.0.join("a/b")).unwrap();
        fs::create_dir(f.0.join("c")).unwrap();
        fs::write(f.0.join("a/b/deep.bin"), [0u8; 100]).unwrap();
        fs::write(f.0.join("a/top.bin"), [0u8; 10]).unwrap();
        fs::write(f.0.join("c/other.bin"), [0u8; 1]).unwrap();
        let s = scan(&f.0, &AtomicBool::new(false));
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        assert_eq!(s.nodes[0].parent, None);
        assert_eq!(s.nodes[0].apparent, 111);
        assert_eq!(s.nodes[find(&s, "a")].apparent, 110);
        assert_eq!(s.nodes[find(&s, "b")].parent, Some(find(&s, "a")));
        assert_eq!(s.nodes[find(&s, "deep.bin")].parent, Some(find(&s, "b")));
        assert_eq!(s.nodes[find(&s, "other.bin")].parent, Some(find(&s, "c")));
    }

    #[test]
    fn inner_symlinks_not_followed_root_symlink_is() {
        let f = Fixture::new("links");
        fs::create_dir(f.0.join("real")).unwrap();
        fs::write(f.0.join("real/file"), [0u8; 50]).unwrap();
        symlink(f.0.join("real"), f.0.join("real/loop")).unwrap();
        symlink(f.0.join("real"), f.0.join("root-link")).unwrap();
        let s = scan(&f.0.join("real"), &AtomicBool::new(false));
        let link = &s.nodes[find(&s, "loop")];
        assert!(!link.directory && !link.regular_file);
        assert_eq!(s.nodes.len(), 3, "loop must not be entered");
        // symlinked root is followed
        let s = scan(&f.0.join("root-link"), &AtomicBool::new(false));
        assert!(s.nodes[0].directory);
        assert_eq!(s.nodes.len(), 3);
    }

    #[test]
    fn special_files_and_non_utf8_names() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt, process::Command};
        let f = Fixture::new("special");
        let odd = OsStr::from_bytes(b"bad-\xff\xfe-name");
        fs::write(f.0.join(odd), [0u8; 9]).unwrap();
        let fifo = Command::new("mkfifo").arg(f.0.join("pipe")).status();
        let s = scan(&f.0, &AtomicBool::new(false));
        assert!(s.errors.is_empty(), "{:?}", s.errors);
        let n = s
            .nodes
            .iter()
            .find(|n| n.name == odd)
            .expect("odd name kept");
        assert!(n.regular_file && n.apparent == 9 && n.parent == Some(0));
        assert_eq!(s.nodes[0].apparent, 9);
        if fifo.is_ok_and(|st| st.success()) {
            // FIFO is listed but is neither file nor directory and adds no size.
            let p = &s.nodes[find(&s, "pipe")];
            assert!(!p.directory && !p.regular_file && p.apparent == 0);
        }
    }

    #[test]
    fn progress_reports_entry_count() {
        let f = Fixture::new("progress");
        fs::create_dir(f.0.join("d")).unwrap();
        fs::write(f.0.join("d/x"), [0u8; 1]).unwrap();
        let progress = AtomicUsize::new(0);
        let s = scan_with_progress(&f.0, &AtomicBool::new(false), &progress);
        assert_eq!(progress.load(Ordering::Relaxed), s.nodes.len());
        assert_eq!(s.nodes.len(), 3);
    }

    #[test]
    fn hard_links_counted_once() {
        let f = Fixture::new("hardlink");
        fs::create_dir(f.0.join("d")).unwrap();
        fs::write(f.0.join("d/a"), [0u8; 100]).unwrap();
        fs::hard_link(f.0.join("d/a"), f.0.join("d/b")).unwrap();
        fs::hard_link(f.0.join("d/a"), f.0.join("c")).unwrap();
        let s = scan(&f.0, &AtomicBool::new(false));
        assert_eq!(s.nodes[0].apparent, 100);
        assert_eq!(s.hard_links_skipped, 2);
        let owned = s
            .nodes
            .iter()
            .filter(|n| n.apparent == 100 && n.regular_file)
            .count();
        assert_eq!(owned, 1);
    }

    #[test]
    fn mount_boundary_is_not_crossed() {
        use std::os::unix::fs::MetadataExt;
        // /dev/shm is a tmpfs mounted inside devtmpfs on typical Linux; skip elsewhere.
        let (parent, mount) = (Path::new("/dev"), Path::new("/dev/shm"));
        let differs = matches!((parent.metadata(), mount.metadata()),
            (Ok(a), Ok(b)) if a.dev() != b.dev());
        if !differs {
            return;
        }
        let s = scan(parent, &AtomicBool::new(false));
        // The mount point may be listed as a leaf, but never entered.
        let shm = s.nodes.iter().position(|n| n.name == "shm");
        assert!(
            !s.nodes
                .iter()
                .any(|n| n.parent.is_some() && n.parent == shm),
            "contents of another device must be excluded"
        );
    }

    #[test]
    fn unreadable_directory_keeps_tree_consistent() {
        use std::os::unix::fs::PermissionsExt;
        let f = Fixture::new("perm");
        fs::create_dir(f.0.join("locked")).unwrap();
        fs::create_dir(f.0.join("open")).unwrap();
        fs::write(f.0.join("locked/hidden"), [0u8; 5]).unwrap();
        fs::write(f.0.join("open/seen"), [0u8; 7]).unwrap();
        fs::set_permissions(f.0.join("locked"), fs::Permissions::from_mode(0o000)).unwrap();
        let s = scan(&f.0, &AtomicBool::new(false));
        fs::set_permissions(f.0.join("locked"), fs::Permissions::from_mode(0o755)).unwrap();
        if s.errors.is_empty() {
            return; // running as root: permissions not enforced
        }
        assert_eq!(s.nodes[find(&s, "seen")].parent, Some(find(&s, "open")));
        assert!(s.nodes[find(&s, "locked")].directory);
        assert_eq!(s.nodes[0].apparent, 7);
    }
}
