//! Persistent settings in `$XDG_CONFIG_HOME/diskatlas/settings.conf`.
//!
//! ponytail: hand-rolled `key=value` lines (no serde/toml dependency); switch to TOML
//! once settings need nesting.
use crate::{View, look::Theme};
use std::{
    ffi::OsString,
    fs, io,
    io::Write,
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub view: View,
    pub exclusions: Vec<PathBuf>,
    /// Absolute folder of the most recent scan.
    pub last_folder: Option<String>,
    /// Inner window size in points when last closed (not maximized).
    pub window: Option<(u32, u32)>,
    pub theme: Theme,
    /// UI zoom in percent (egui zoom factor x 100).
    pub zoom: u32,
    pub reduced_motion: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            view: View::default(),
            exclusions: Vec::new(),
            last_folder: None,
            window: None,
            theme: Theme::default(),
            zoom: 100,
            reduced_motion: false,
        }
    }
}

const THEMES: [(&str, Theme); 4] = [
    ("system", Theme::System),
    ("light", Theme::Light),
    ("dark", Theme::Dark),
    ("high-contrast", Theme::HighContrast),
];

const ZOOM_RANGE: std::ops::RangeInclusive<u32> = 50..=300;

/// Sanity range for a stored window dimension; the viewport's minimum size still applies.
const WINDOW_RANGE: std::ops::RangeInclusive<u32> = 200..=16384;

const VIEWS: [(&str, View); 4] = [
    ("sunburst", View::Sunburst),
    ("treemap", View::Treemap),
    ("directory", View::Directory),
    ("largest", View::LargestFiles),
];

/// Unknown keys, unknown views and relative exclusions are ignored, so a hand-edited or
/// newer file never prevents startup.
pub fn parse(text: &str) -> Settings {
    let mut settings = Settings::default();
    for line in text.lines().map(str::trim) {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "view" => {
                if let Some((_, view)) = VIEWS.iter().find(|(name, _)| *name == value.trim()) {
                    settings.view = *view;
                }
            }
            "theme" => {
                if let Some((_, theme)) = THEMES.iter().find(|(name, _)| *name == value.trim()) {
                    settings.theme = *theme;
                }
            }
            "zoom" => {
                if let Some(zoom) = value.trim().parse().ok().filter(|z| ZOOM_RANGE.contains(z)) {
                    settings.zoom = zoom;
                }
            }
            "reduced_motion" => settings.reduced_motion = value.trim() == "true",
            "last_folder" => {
                let path = PathBuf::from(value.trim());
                if path.is_absolute() {
                    settings.last_folder = Some(value.trim().to_owned());
                }
            }
            "window" => {
                let size = value
                    .trim()
                    .split_once('x')
                    .and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?)));
                settings.window =
                    size.filter(|(w, h)| WINDOW_RANGE.contains(w) && WINDOW_RANGE.contains(h));
            }
            "exclude" => {
                let path = PathBuf::from(value.trim());
                if path.is_absolute() && !settings.exclusions.contains(&path) {
                    settings.exclusions.push(path);
                }
            }
            _ => {}
        }
    }
    settings
}

pub fn render(settings: &Settings) -> String {
    let view = VIEWS
        .iter()
        .find(|(_, v)| *v == settings.view)
        .map_or("sunburst", |(name, _)| name);
    let mut out = format!("# Disk Atlas settings\nview={view}\n");
    let theme = THEMES
        .iter()
        .find(|(_, t)| *t == settings.theme)
        .map_or("system", |(name, _)| name);
    out.push_str(&format!(
        "theme={theme}\nzoom={}\nreduced_motion={}\n",
        settings.zoom, settings.reduced_motion
    ));
    if let Some((w, h)) = settings.window {
        out.push_str(&format!("window={w}x{h}\n"));
    }
    // A path with a newline cannot be stored on one line; skip it like the exclusions below.
    if let Some(folder) = settings
        .last_folder
        .as_deref()
        .filter(|f| !f.contains('\n'))
    {
        out.push_str(&format!("last_folder={folder}\n"));
    }
    // Paths that are not UTF-8 or contain a newline cannot be stored on one line; skip them.
    for path in settings.exclusions.iter().filter_map(|p| p.to_str()) {
        if !path.contains('\n') {
            out.push_str(&format!("exclude={path}\n"));
        }
    }
    out
}

/// XDG Base Directory rule: `$XDG_CONFIG_HOME` only if absolute, else `$HOME/.config`.
fn config_dir(xdg: Option<OsString>, home: Option<OsString>) -> Option<PathBuf> {
    let xdg = xdg.map(PathBuf::from).filter(|p| p.is_absolute());
    xdg.or_else(|| {
        home.map(|h| PathBuf::from(h).join(".config"))
            .filter(|p| p.is_absolute())
    })
}

fn path() -> Option<PathBuf> {
    config_dir(
        std::env::var_os("XDG_CONFIG_HOME"),
        std::env::var_os("HOME"),
    )
    .map(|dir| dir.join("diskatlas").join("settings.conf"))
}

/// A missing or unreadable file yields the defaults.
pub fn load() -> Settings {
    path()
        .and_then(|p| fs::read(p).ok())
        .map(|bytes| parse(&String::from_utf8_lossy(&bytes)))
        .unwrap_or_default()
}

pub fn save(settings: &Settings) -> io::Result<()> {
    match path() {
        Some(path) => save_to(&path, settings),
        None => Ok(()), // no HOME: nowhere to store anything
    }
}

/// Write to a private temp file, then rename, so a crash never leaves a half-written file.
fn save_to(path: &Path, settings: &Settings) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let tmp = path.with_extension("tmp");
    fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?
        .write_all(render(settings).as_bytes())?;
    fs::rename(tmp, path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn round_trip_and_tolerant_parsing() {
        let settings = Settings {
            view: View::Treemap,
            exclusions: vec!["/usr/lib".into(), "/home/a b/cache".into()],
            last_folder: Some("/home/a b".into()),
            window: Some((1280, 800)),
            theme: Theme::HighContrast,
            zoom: 150,
            reduced_motion: true,
        };
        assert_eq!(parse(&render(&settings)), settings);
        let messy = "# comment\nnonsense\nview=bogus\nfuture=1\nexclude=relative/dir\n\
                     exclude = /a\nexclude=/a\n  view = largest  \n";
        let parsed = parse(messy);
        assert_eq!(parsed.view, View::LargestFiles);
        assert_eq!(parsed.exclusions, [PathBuf::from("/a")]);
        // Relative folder, absurd or malformed window sizes are dropped.
        for bad in [
            "theme=neon",
            "zoom=10",
            "zoom=900",
            "zoom=big",
            "reduced_motion=yes",
            "last_folder=rel/dir",
            "window=10x10",
            "window=99999x800",
            "window=800",
            "window=ax1",
        ] {
            assert_eq!(parse(bad), Settings::default(), "{bad}");
        }
        assert_eq!(parse(""), Settings::default());
    }

    #[test]
    fn xdg_config_dir_rules() {
        let s = |v: &str| Some(OsString::from(v));
        assert_eq!(config_dir(s("/x"), s("/h")), Some("/x".into()));
        // Relative XDG_CONFIG_HOME must be ignored per the spec.
        assert_eq!(config_dir(s("rel"), s("/h")), Some("/h/.config".into()));
        assert_eq!(config_dir(None, s("/h")), Some("/h/.config".into()));
        assert_eq!(config_dir(None, None), None);
    }

    #[test]
    fn save_creates_private_file_and_replaces_atomically() {
        let dir = std::env::temp_dir().join(format!("diskatlas-settings-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let file = dir.join("nested/settings.conf");
        let mut settings = Settings {
            view: View::Directory,
            exclusions: vec!["/one".into()],
            ..Settings::default()
        };
        save_to(&file, &settings).unwrap();
        settings.exclusions.push("/two".into());
        save_to(&file, &settings).unwrap();
        assert_eq!(parse(&fs::read_to_string(&file).unwrap()), settings);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(!file.with_extension("tmp").exists());
        fs::remove_dir_all(&dir).unwrap();
    }
}
