//! Where the `.desktop` files and icons are, and reading them. Pure file
//! work, so it is tested on any system.

use crate::desktop::{self, Env, desktop_id};
use crate::model::AppEntry;
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};

fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `$XDG_DATA_HOME` and each `$XDG_DATA_DIRS` entry, plus the places a NixOS
/// session often leaves out of them. Earlier ones win.
pub fn data_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME").filter(|d| !d.is_empty()) {
        Some(dir) => dirs.push(PathBuf::from(dir)),
        None => dirs.extend(home().map(|h| h.join(".local/share"))),
    }
    let system = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let system = if system.is_empty() {
        "/usr/local/share:/usr/share".to_string()
    } else {
        system
    };
    dirs.extend(
        system
            .split(':')
            .filter(|d| !d.is_empty())
            .map(PathBuf::from),
    );
    dirs.extend(home().map(|h| h.join(".nix-profile/share")));
    if let Ok(user) = std::env::var("USER") {
        dirs.push(PathBuf::from(format!(
            "/etc/profiles/per-user/{user}/share"
        )));
    }
    dirs.push(PathBuf::from("/run/current-system/sw/share"));
    let mut seen = HashSet::new();
    dirs.retain(|d| seen.insert(d.clone()));
    dirs
}

fn has_program(program: &str) -> bool {
    if program.contains('/') {
        return Path::new(program).is_file();
    }
    std::env::var_os("PATH")
        .is_some_and(|path| std::env::split_paths(&path).any(|dir| dir.join(program).is_file()))
}

fn collect(dir: &Path, base: &Path, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            collect(&path, base, out);
        } else if path.extension().is_some_and(|e| e == "desktop")
            && let Ok(relative) = path.strip_prefix(base)
        {
            out.push((desktop_id(&relative.to_string_lossy()), path));
        }
    }
}

/// Every listed app; where two data dirs have the same file ID the first wins.
pub fn scan_dirs(dirs: &[PathBuf], env: &Env) -> Vec<AppEntry> {
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in dirs {
        let base = dir.join("applications");
        let mut files = Vec::new();
        collect(&base, &base, &mut files);
        for (id, path) in files {
            if !seen.insert(id.clone()) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if let Some(app) =
                desktop::parse(&id, &path.to_string_lossy(), &text, env, &has_program)
            {
                apps.push(app);
            }
        }
    }
    apps.sort_by_cached_key(|a| a.name.to_lowercase());
    apps
}

const SIZES: [u32; 6] = [48, 64, 32, 128, 256, 24];

/// A PNG for an `Icon=` value: the file itself, or a theme icon found under
/// the data dirs (hicolor first, then any other theme). SVG is skipped.
pub fn icon_path(icon: &str, dirs: &[PathBuf]) -> Option<PathBuf> {
    if icon.starts_with('/') {
        return (Path::new(icon).extension().is_some_and(|e| e == "png")
            && Path::new(icon).is_file())
        .then(|| PathBuf::from(icon));
    }
    let name = icon.strip_suffix(".png").unwrap_or(icon);
    for dir in dirs {
        let icons = dir.join("icons");
        let mut themes: Vec<PathBuf> = vec![icons.join("hicolor")];
        if let Ok(entries) = std::fs::read_dir(&icons) {
            let mut others: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path())
                .filter(|p| p.is_dir() && !p.ends_with("hicolor"))
                .collect();
            others.sort();
            themes.extend(others);
        }
        for theme in themes {
            for size in SIZES {
                for layout in [
                    theme.join(format!("{size}x{size}/apps")),
                    theme.join(format!("apps/{size}")),
                ] {
                    let candidate = layout.join(format!("{name}.png"));
                    if candidate.is_file() {
                        return Some(candidate);
                    }
                }
            }
        }
        let pixmap = dir.join(format!("pixmaps/{name}.png"));
        if pixmap.is_file() {
            return Some(pixmap);
        }
    }
    let pixmap = PathBuf::from(format!("/usr/share/pixmaps/{name}.png"));
    pixmap.is_file().then_some(pixmap)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("telmo-launcher-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn the_first_data_dir_wins_and_subfolders_become_dashes() {
        let root = scratch("desktop");
        let (a, b) = (root.join("a"), root.join("b"));
        let entry =
            |name: &str| format!("[Desktop Entry]\nType=Application\nName={name}\nExec=x\n");
        write(&a.join("applications/app.desktop"), &entry("From A"));
        write(&b.join("applications/app.desktop"), &entry("From B"));
        write(&b.join("applications/kde/tool.desktop"), &entry("Tool"));
        let env = Env::default();
        let apps = scan_dirs(&[a, b], &env);
        let named: Vec<_> = apps
            .iter()
            .map(|a| (a.id.as_str(), a.name.as_str()))
            .collect();
        assert_eq!(
            named,
            [("app.desktop", "From A"), ("kde-tool.desktop", "Tool")]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn icons_resolve_to_a_png_and_skip_svg() {
        let root = scratch("icons");
        write(&root.join("icons/hicolor/48x48/apps/fox.png"), "png");
        write(&root.join("icons/hicolor/scalable/apps/vec.svg"), "svg");
        write(&root.join("icons/Papirus/64x64/apps/papi.png"), "png");
        write(&root.join("pixmaps/old.png"), "png");
        let dirs = vec![root.clone()];
        assert!(
            icon_path("fox", &dirs)
                .unwrap()
                .ends_with("hicolor/48x48/apps/fox.png")
        );
        assert!(
            icon_path("papi", &dirs)
                .unwrap()
                .ends_with("Papirus/64x64/apps/papi.png")
        );
        assert!(
            icon_path("old", &dirs)
                .unwrap()
                .ends_with("pixmaps/old.png")
        );
        assert!(icon_path("vec", &dirs).is_none(), "SVG has no icon");
        assert!(icon_path("missing", &dirs).is_none());
        let absolute = root.join("pixmaps/old.png");
        assert_eq!(
            icon_path(&absolute.to_string_lossy(), &dirs),
            Some(absolute)
        );
        assert!(icon_path("/nope/x.png", &dirs).is_none());
        let _ = std::fs::remove_dir_all(root);
    }
}
