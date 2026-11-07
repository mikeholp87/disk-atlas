//! Linux mount table (`/proc/self/mountinfo`).
use std::{
    ffi::OsString,
    io,
    os::unix::ffi::OsStringExt,
    path::{Path, PathBuf},
};

#[derive(Debug, PartialEq, Eq)]
pub struct Mount {
    pub id: u32,
    pub parent_id: u32,
    /// `major:minor` of the backing device.
    pub device: String,
    /// Subtree of the filesystem that is mounted here: `/` unless this is a bind mount or subvolume.
    pub root: PathBuf,
    pub mount_point: PathBuf,
    pub fs_type: String,
    pub source: OsString,
}

/// Decode the kernel's `\040`-style octal escapes, keeping non-UTF-8 bytes intact.
fn unescape(field: &[u8]) -> OsString {
    let mut out = Vec::with_capacity(field.len());
    let mut i = 0;
    while i < field.len() {
        let octal = field
            .get(i + 1..i + 4)
            .filter(|d| field[i] == b'\\' && d.iter().all(|b| (b'0'..=b'7').contains(b)))
            .map(|d| d.iter().fold(0u32, |n, b| n * 8 + u32::from(b - b'0')));
        match octal {
            Some(n) => {
                out.push(n as u8); // kernel escapes single bytes only
                i += 4;
            }
            None => {
                out.push(field[i]);
                i += 1;
            }
        }
    }
    OsString::from_vec(out)
}

fn parse_line(line: &[u8]) -> Option<Mount> {
    let f: Vec<&[u8]> = line.split(|&b| b == b' ').collect();
    // Fields 6.. are optional tags, terminated by a lone "-".
    let sep = 6 + f.get(6..)?.iter().position(|x| *x == b"-")?;
    if f.len() < sep + 3 {
        return None;
    }
    let num = |i: usize| -> Option<u32> { std::str::from_utf8(f[i]).ok()?.parse().ok() };
    Some(Mount {
        id: num(0)?,
        parent_id: num(1)?,
        device: String::from_utf8_lossy(f[2]).into_owned(),
        root: unescape(f[3]).into(),
        mount_point: unescape(f[4]).into(),
        fs_type: String::from_utf8_lossy(f[sep + 1]).into_owned(),
        source: unescape(f[sep + 2]),
    })
}

/// Parse mountinfo text; malformed lines are skipped.
pub fn parse(text: &[u8]) -> Vec<Mount> {
    text.split(|&b| b == b'\n').filter_map(parse_line).collect()
}

pub fn read() -> io::Result<Vec<Mount>> {
    Ok(parse(&std::fs::read("/proc/self/mountinfo")?))
}

/// The mount that holds `path`: the deepest mount point that is a path prefix (by component,
/// so `/mnt2` is not under `/mnt`). If mounts are stacked on one point, the last listed wins,
/// which is the one visible. `path` must be canonical.
pub fn mount_for<'a>(mounts: &'a [Mount], path: &Path) -> Option<&'a Mount> {
    mounts
        .iter()
        .filter(|m| path.starts_with(&m.mount_point))
        .max_by_key(|m| m.mount_point.components().count())
}

/// Filesystem space in bytes, as reported by `statvfs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Capacity {
    pub total: u64,
    /// Free blocks, including those reserved for root.
    pub free: u64,
    /// Free space available to unprivileged users (what `df` calls Avail).
    pub available: u64,
}

impl Capacity {
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.free)
    }
}

/// May block indefinitely on an unresponsive network mount; call off the UI thread.
pub fn capacity(path: &Path) -> io::Result<Capacity> {
    let s = rustix::fs::statvfs(path)?;
    let bytes = |blocks: u64| blocks.saturating_mul(s.f_frsize);
    Ok(Capacity {
        total: bytes(s.f_blocks),
        free: bytes(s.f_bfree),
        available: bytes(s.f_bavail),
    })
}

#[derive(Debug)]
pub struct Volume {
    pub mount: Mount,
    pub capacity: Capacity,
}

// ponytail: fixed denylist of kernel/virtual filesystem types; revisit if real volumes get hidden.
const HIDDEN_FS: &[&str] = &[
    "proc",
    "sysfs",
    "cgroup",
    "cgroup2",
    "devpts",
    "devtmpfs",
    "securityfs",
    "pstore",
    "bpf",
    "tracefs",
    "debugfs",
    "mqueue",
    "hugetlbfs",
    "configfs",
    "fusectl",
    "binfmt_misc",
    "autofs",
    "efivarfs",
    "nsfs",
    "squashfs",
];

/// Mounted filesystems with real capacity, sorted by mount point. Unreadable mounts are skipped.
pub fn volumes() -> Vec<Volume> {
    let mut volumes: Vec<_> = read()
        .unwrap_or_default()
        .into_iter()
        .filter(|m| !HIDDEN_FS.contains(&m.fs_type.as_str()))
        .filter_map(|mount| {
            let capacity = capacity(&mount.mount_point).ok()?;
            (capacity.total > 0).then_some(Volume { mount, capacity })
        })
        .collect();
    volumes.sort_by(|a, b| a.mount.mount_point.cmp(&b.mount.mount_point));
    volumes
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn parses_bind_mount_escapes_and_optional_fields() {
        let text = b"36 35 98:0 /mnt1 /mnt2 rw,noatime master:1 - ext3 /dev/root rw\n\
            77 1 0:30 / /mnt/my\\040disk\\134x rw shared:5 shared:6 - ext4 /dev/sdb1 rw\n\
            garbage line\n\
            90 1 0:5 / /proc rw - proc proc rw\n";
        let m = parse(text);
        assert_eq!(m.len(), 3, "malformed line skipped: {m:?}");
        assert_eq!(m[0].root, Path::new("/mnt1"));
        assert_eq!(m[0].mount_point, Path::new("/mnt2"));
        assert_eq!(
            (m[0].device.as_str(), m[0].fs_type.as_str()),
            ("98:0", "ext3")
        );
        assert_eq!(m[1].mount_point, Path::new("/mnt/my disk\\x"));
        assert_eq!(m[1].source, "/dev/sdb1");
        assert_eq!(m[2].fs_type, "proc");
    }

    #[test]
    fn volumes_include_root_and_hide_pseudo_filesystems() {
        let v = volumes();
        let root = v
            .iter()
            .find(|v| v.mount.mount_point == Path::new("/"))
            .unwrap();
        let c = root.capacity;
        assert!(c.total > 0 && c.free <= c.total && c.available <= c.free);
        assert_eq!(c.used(), c.total - c.free);
        assert!(
            v.iter()
                .all(|v| !HIDDEN_FS.contains(&v.mount.fs_type.as_str()))
        );
        assert!(
            v.windows(2)
                .all(|w| w[0].mount.mount_point <= w[1].mount.mount_point)
        );
    }

    #[test]
    fn mount_for_picks_deepest_then_last_and_respects_components() {
        let text = b"1 0 8:1 / / rw - ext4 /dev/a rw\n\
            2 1 8:2 / /mnt rw - xfs /dev/b rw\n\
            3 1 8:3 / /mnt2 rw - vfat /dev/c rw\n\
            4 2 8:4 / /mnt/data rw - btrfs /dev/d rw\n\
            5 2 8:5 / /mnt/data rw - tmpfs tmpfs rw\n";
        let m = parse(text);
        let fs = |p: &str| mount_for(&m, Path::new(p)).map(|m| m.fs_type.as_str());
        assert_eq!(fs("/home/x"), Some("ext4"));
        assert_eq!(fs("/mnt/file"), Some("xfs"));
        assert_eq!(fs("/mnt2/file"), Some("vfat"), "/mnt2 is not under /mnt");
        assert_eq!(fs("/mnt/data/deep/er"), Some("tmpfs"), "stacked: last wins");
        assert_eq!(mount_for(&[], Path::new("/")), None);
    }

    #[test]
    fn real_mount_table_has_root() {
        let m = read().unwrap();
        assert!(m.iter().any(|m| m.mount_point == Path::new("/")));
    }
}
