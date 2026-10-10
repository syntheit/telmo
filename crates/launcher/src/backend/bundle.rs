//! macOS app bundles: finding them without Spotlight and reading their names.

use crate::model::AppEntry;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The host app; it must not offer to launch itself.
const HOST_BUNDLE: &str = "io.github.syntheit.telmo";

/// Where apps live. Nix puts its apps in the last three.
pub fn search_dirs(home: &Path) -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Applications"),
        PathBuf::from("/System/Applications"),
        PathBuf::from("/System/Applications/Utilities"),
        home.join("Applications"),
        home.join("Applications/Home Manager Apps"),
        PathBuf::from("/Applications/Nix Apps"),
    ]
}

/// Apps that live outside those folders.
pub fn extra_bundles() -> Vec<PathBuf> {
    vec![PathBuf::from("/System/Library/CoreServices/Finder.app")]
}

/// The name to show, and the other names the app answers to. The plist's
/// display name wins, then its bundle name, then the folder's name; except
/// that a folder name which spells the plist name out ("Visual Studio Code"
/// for "Code") is the one people know.
pub fn names(stem: &str, display: Option<&str>, name: Option<&str>) -> (String, Vec<String>) {
    let clean = |s: Option<&str>| s.map(str::trim).filter(|s| !s.is_empty()).map(String::from);
    let (display, name) = (clean(display), clean(name));
    let stem = stem.trim().to_string();
    let from_plist = display.clone().or_else(|| name.clone());
    let primary = match from_plist {
        Some(p) if stem.len() > p.len() && stem.to_lowercase().contains(&p.to_lowercase()) => {
            stem.clone()
        }
        Some(p) => p,
        None => stem.clone(),
    };
    let mut aliases = Vec::new();
    for other in [Some(stem), display, name].into_iter().flatten() {
        if other != primary && !aliases.contains(&other) {
            aliases.push(other);
        }
    }
    (primary, aliases)
}

/// Reads `Contents/Info.plist`. `None` for anything that isn't a plain app
/// (no plist, a background agent, another package type).
pub fn read(bundle: &Path) -> Option<AppEntry> {
    let value = plist::Value::from_file(bundle.join("Contents/Info.plist")).ok()?;
    let info = value.as_dictionary()?;
    let text = |key: &str| info.get(key).and_then(|v| v.as_string());
    let truthy = |key: &str| {
        info.get(key).is_some_and(|v| {
            v.as_boolean() == Some(true)
                || v.as_string()
                    .is_some_and(|s| s.eq_ignore_ascii_case("yes") || s == "1")
        })
    };
    if text("CFBundlePackageType").is_some_and(|t| t != "APPL")
        || truthy("LSUIElement")
        || truthy("LSBackgroundOnly")
    {
        return None;
    }
    let path = bundle.to_string_lossy().into_owned();
    let id = text("CFBundleIdentifier").map_or_else(|| path.clone(), String::from);
    if id == HOST_BUNDLE {
        return None;
    }
    let stem = bundle.file_stem()?.to_string_lossy();
    let (name, aliases) = names(&stem, text("CFBundleDisplayName"), text("CFBundleName"));
    let mut app = AppEntry::new(&id, &name, &path);
    app.aliases = aliases;
    Some(app)
}

fn bundles_in(dir: &Path, depth: u8, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.extension().is_some_and(|e| e == "app") {
            out.push(path);
        } else if depth == 0 && path.is_dir() {
            // One folder deeper: /Applications/Adobe Creative Cloud/….app
            bundles_in(&path, 1, out);
        }
    }
}

/// Every app in `dirs` (one folder level down too) and in `extra`. The first
/// of two copies wins, by resolved path and by identifier.
pub fn scan(dirs: &[PathBuf], extra: &[PathBuf]) -> Vec<AppEntry> {
    let mut found = Vec::new();
    for dir in dirs {
        bundles_in(dir, 0, &mut found);
    }
    found.extend(extra.iter().filter(|p| p.is_dir()).cloned());
    let mut seen_paths = HashSet::new();
    let mut seen_ids = HashSet::new();
    let mut apps = Vec::new();
    for bundle in found {
        // Nix apps are symlinks into the store: judge copies by where they point.
        let resolved = std::fs::canonicalize(&bundle).unwrap_or_else(|_| bundle.clone());
        if !seen_paths.insert(resolved) {
            continue;
        }
        let Some(app) = read(&bundle) else { continue };
        if seen_ids.insert(app.id.clone()) {
            apps.push(app);
        }
    }
    apps.sort_by_cached_key(|a| a.name.to_lowercase());
    apps
}

#[cfg(test)]
mod tests {
    use super::*;
    use plist::Dictionary;

    #[test]
    fn the_plist_name_wins_unless_the_folder_spells_it_out() {
        assert_eq!(names("Zen", None, Some("Zen")), ("Zen".into(), vec![]));
        assert_eq!(
            names("Visual Studio Code", None, Some("Code")),
            ("Visual Studio Code".into(), vec!["Code".into()])
        );
        assert_eq!(
            names("wifi-panel", Some("Wi-Fi Panel"), Some("wifi")),
            (
                "Wi-Fi Panel".into(),
                vec!["wifi-panel".into(), "wifi".into()]
            )
        );
        assert_eq!(
            names("Calculator", Some(" "), None),
            ("Calculator".into(), vec![])
        );
        assert_eq!(
            names("Safari", Some("Safari"), Some("Safari")).1,
            Vec::<String>::new()
        );
    }

    fn write_app(dir: &Path, stem: &str, info: &[(&str, plist::Value)]) -> PathBuf {
        let bundle = dir.join(format!("{stem}.app"));
        std::fs::create_dir_all(bundle.join("Contents")).unwrap();
        let mut dict = Dictionary::new();
        for (key, value) in info {
            dict.insert((*key).into(), value.clone());
        }
        plist::Value::Dictionary(dict)
            .to_file_xml(bundle.join("Contents/Info.plist"))
            .unwrap();
        bundle
    }

    fn s(text: &str) -> plist::Value {
        plist::Value::String(text.into())
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("telmo-launcher-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn bundles_are_read_deduplicated_and_filtered() {
        let root = scratch("bundles");
        let apps = root.join("Applications");
        let nix = root.join("Nix Apps");
        std::fs::create_dir_all(&nix).unwrap();
        write_app(
            &apps,
            "Zen",
            &[
                ("CFBundleIdentifier", s("app.zen")),
                ("CFBundleName", s("Zen")),
            ],
        );
        write_app(
            &apps,
            "Visual Studio Code",
            &[
                ("CFBundleIdentifier", s("com.vsc")),
                ("CFBundleName", s("Code")),
                ("CFBundlePackageType", s("APPL")),
            ],
        );
        write_app(
            &apps,
            "Agent",
            &[
                ("CFBundleIdentifier", s("x.agent")),
                ("LSUIElement", plist::Value::Boolean(true)),
            ],
        );
        write_app(&apps, "Host", &[("CFBundleIdentifier", s(HOST_BUNDLE))]);
        write_app(&apps, "NoId", &[]);
        write_app(
            &apps.join("Suite"),
            "Inner",
            &[("CFBundleIdentifier", s("x.inner"))],
        );
        std::fs::create_dir_all(apps.join("Broken.app")).unwrap();
        // A Nix app: the same bundle seen through a symlink, and a second copy with the same id.
        std::os::unix::fs::symlink(apps.join("Zen.app"), nix.join("Zen.app")).unwrap();
        write_app(&nix, "Zen Copy", &[("CFBundleIdentifier", s("app.zen"))]);

        let found = scan(&[apps.clone(), nix], &[]);
        let names: Vec<_> = found.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Inner", "NoId", "Visual Studio Code", "Zen"]);
        let code = found.iter().find(|a| a.id == "com.vsc").unwrap();
        assert_eq!(code.aliases, ["Code"]);
        assert_eq!(
            code.path,
            apps.join("Visual Studio Code.app").to_string_lossy()
        );
        let no_id = found.iter().find(|a| a.name == "NoId").unwrap();
        assert_eq!(no_id.id, no_id.path, "no identifier: the path stands in");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_missing_folder_is_just_empty() {
        assert!(scan(&[PathBuf::from("/definitely/not/here")], &[]).is_empty());
    }

    #[test]
    fn the_search_list_covers_the_documented_places() {
        let dirs = search_dirs(Path::new("/Users/me"));
        assert!(dirs.contains(&PathBuf::from("/Users/me/Applications/Home Manager Apps")));
        assert!(dirs.contains(&PathBuf::from("/Applications/Nix Apps")));
        assert!(dirs.contains(&PathBuf::from("/System/Applications/Utilities")));
    }
}
