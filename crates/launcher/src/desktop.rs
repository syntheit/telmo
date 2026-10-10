//! Freedesktop `.desktop` files: which are apps worth listing, their names,
//! and how to start them without a shell.
//! <https://specifications.freedesktop.org/desktop-entry/latest-single/>

use crate::model::AppEntry;

/// What decides whether an entry shows.
#[derive(Debug, Clone, Default)]
pub struct Env {
    /// `XDG_CURRENT_DESKTOP` split at colons (`Hyprland`).
    pub desktops: Vec<String>,
    /// Name-localisation keys, most specific first: `de_DE`, `de`.
    pub locales: Vec<String>,
}

impl Env {
    pub fn from_environment() -> Self {
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|d| !d.is_empty())
            .map(String::from)
            .collect();
        let locale = ["LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .find_map(|var| std::env::var(var).ok().filter(|v| !v.is_empty()))
            .unwrap_or_default();
        Self {
            desktops,
            locales: locale_keys(&locale),
        }
    }
}

/// `de_DE.UTF-8@euro` -> `de_DE`, `de`.
pub fn locale_keys(locale: &str) -> Vec<String> {
    let base = locale.split(['.', '@']).next().unwrap_or("");
    if base.is_empty() || base == "C" || base == "POSIX" {
        return Vec::new();
    }
    let mut keys = vec![base.to_string()];
    if let Some((lang, _)) = base.split_once('_') {
        keys.push(lang.to_string());
    }
    keys
}

/// `\s`, `\n`, `\t`, `\r` and `\\` in a value.
fn unescape(value: &str) -> String {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('s') => out.push(' '),
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                out.push('\\');
                out.push(other);
            }
            None => out.push('\\'),
        }
    }
    out
}

/// A `;`-separated list value; `\;` is a literal semicolon.
fn list(value: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut chars = value.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\\' if chars.peek() == Some(&';') => {
                current.push(';');
                chars.next();
            }
            ';' => items.push(std::mem::take(&mut current)),
            other => current.push(other),
        }
    }
    if !current.is_empty() {
        items.push(current);
    }
    items
        .into_iter()
        .filter(|i| !i.is_empty())
        .map(|i| unescape(&i))
        .collect()
}

/// The keys of the `[Desktop Entry]` group.
fn entry_keys(text: &str) -> Vec<(&str, &str)> {
    let mut in_entry = false;
    let mut keys = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            in_entry = line == "[Desktop Entry]";
            continue;
        }
        if in_entry && let Some((key, value)) = line.split_once('=') {
            keys.push((key.trim(), value.trim()));
        }
    }
    keys
}

/// The app a `.desktop` file describes, or `None` when it should not be
/// listed. `id` is the desktop file ID; `has_program` says whether a program
/// exists (for `TryExec`).
pub fn parse(
    id: &str,
    path: &str,
    text: &str,
    env: &Env,
    has_program: &dyn Fn(&str) -> bool,
) -> Option<AppEntry> {
    let keys = entry_keys(text);
    let get = |name: &str| keys.iter().find(|(k, _)| *k == name).map(|(_, v)| *v);
    let flag = |name: &str| get(name) == Some("true");
    if get("Type") != Some("Application") || flag("NoDisplay") || flag("Hidden") {
        return None;
    }
    if let Some(only) = get("OnlyShowIn") {
        let only = list(only);
        if !only.is_empty() && !env.desktops.iter().any(|d| only.contains(d)) {
            return None;
        }
    }
    if let Some(not) = get("NotShowIn")
        && list(not).iter().any(|n| env.desktops.contains(n))
    {
        return None;
    }
    if let Some(try_exec) = get("TryExec")
        && !has_program(&unescape(try_exec))
    {
        return None;
    }
    let exec = get("Exec").map(unescape).filter(|e| !e.trim().is_empty())?;
    let localized = |key: &str| {
        env.locales
            .iter()
            .find_map(|locale| get(&format!("{key}[{locale}]")))
            .or_else(|| get(key))
            .map(unescape)
            .filter(|n| !n.is_empty())
    };
    let name = localized("Name")?;
    let mut aliases = Vec::new();
    for extra in [localized("GenericName"), get("Name").map(unescape)]
        .into_iter()
        .flatten()
    {
        if extra != name && !aliases.contains(&extra) {
            aliases.push(extra);
        }
    }
    Some(AppEntry {
        id: id.to_string(),
        name,
        aliases,
        path: path.to_string(),
        icon: get("Icon").map(unescape).filter(|i| !i.is_empty()),
        exec: Some(exec),
        wm_class: get("StartupWMClass")
            .map(unescape)
            .filter(|c| !c.is_empty()),
        terminal: flag("Terminal"),
    })
}

/// The desktop file ID of `<dir>/applications/<rel>`: slashes become dashes.
pub fn desktop_id(relative: &str) -> String {
    relative.replace('/', "-")
}

/// The program and arguments for an `Exec` line: quoting undone, field codes
/// replaced or dropped, no shell involved.
pub fn exec_args(exec: &str, name: &str, icon: Option<&str>, path: &str) -> Vec<String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut started = false;
    let mut quoted = false;
    let mut chars = exec.chars().peekable();
    // A field code expands in place; one that is the whole argument and
    // expands to nothing removes the argument.
    let code = |c: char, current: &mut String, args: &mut Vec<String>, started: &mut bool| {
        match c {
            '%' => {
                current.push('%');
                *started = true;
            }
            'c' => {
                current.push_str(name);
                *started = true;
            }
            'k' => {
                current.push_str(path);
                *started = true;
            }
            'i' => {
                if let Some(icon) = icon {
                    if *started {
                        args.push(std::mem::take(current));
                    }
                    args.push("--icon".to_string());
                    current.push_str(icon);
                    *started = true;
                }
            }
            // f F u U d D n N v m: files, URLs and deprecated codes; we open without any.
            _ => {}
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                started = true;
            }
            '\\' if quoted => {
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            '%' => {
                if let Some(next) = chars.next() {
                    code(next, &mut current, &mut args, &mut started);
                }
            }
            c if c.is_whitespace() && !quoted => {
                if started && !current.is_empty() {
                    args.push(std::mem::take(&mut current));
                }
                current.clear();
                started = false;
            }
            c => {
                current.push(c);
                started = true;
            }
        }
    }
    if started && !current.is_empty() {
        args.push(current);
    }
    args
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env() -> Env {
        Env {
            desktops: vec!["Hyprland".into()],
            locales: locale_keys("de_DE.UTF-8"),
        }
    }

    fn parse_text(text: &str) -> Option<AppEntry> {
        parse("test.desktop", "/x/test.desktop", text, &env(), &|p| {
            p == "present"
        })
    }

    const FIREFOX: &str = "# comment
[Desktop Entry]
Type=Application
Name=Firefox
Name[de]=Feuerfuchs
GenericName=Web Browser
Exec=firefox %u
Icon=firefox
StartupWMClass=firefox
Keywords=internet;

[Desktop Action new-window]
Name=New Window
Exec=firefox --new-window
";

    #[test]
    fn a_plain_entry_is_listed_with_its_localised_name() {
        let app = parse_text(FIREFOX).expect("listed");
        assert_eq!(app.name, "Feuerfuchs");
        assert_eq!(app.aliases, ["Web Browser", "Firefox"]);
        assert_eq!(app.icon.as_deref(), Some("firefox"));
        assert_eq!(app.wm_class.as_deref(), Some("firefox"));
        assert_eq!(
            app.exec.as_deref(),
            Some("firefox %u"),
            "actions are not the entry"
        );
        assert_eq!(app.id, "test.desktop");
    }

    #[test]
    fn hidden_and_wrongly_placed_entries_are_skipped() {
        let with =
            |extra: &str| format!("[Desktop Entry]\nType=Application\nName=X\nExec=x\n{extra}\n");
        assert!(parse_text(&with("")).is_some());
        assert!(parse_text(&with("NoDisplay=true")).is_none());
        assert!(parse_text(&with("Hidden=true")).is_none());
        assert!(parse_text(&with("OnlyShowIn=GNOME;KDE;")).is_none());
        assert!(parse_text(&with("OnlyShowIn=GNOME;Hyprland;")).is_some());
        assert!(parse_text(&with("NotShowIn=Hyprland;")).is_none());
        assert!(parse_text(&with("TryExec=missing")).is_none());
        assert!(parse_text(&with("TryExec=present")).is_some());
        assert!(parse_text("[Desktop Entry]\nType=Link\nName=X\nExec=x\n").is_none());
        assert!(
            parse_text("[Desktop Entry]\nType=Application\nName=X\n").is_none(),
            "no Exec"
        );
    }

    #[test]
    fn locales_and_escapes() {
        assert_eq!(locale_keys("de_DE.UTF-8@euro"), ["de_DE", "de"]);
        assert_eq!(locale_keys("en"), ["en"]);
        assert!(locale_keys("C.UTF-8").is_empty());
        assert!(locale_keys("").is_empty());
        assert_eq!(unescape(r"a\sb\\c\n"), "a b\\c\n");
        assert_eq!(list(r"a;b\;c;"), ["a", "b;c"]);
        assert_eq!(desktop_id("kde/dolphin.desktop"), "kde-dolphin.desktop");
    }

    #[test]
    fn exec_field_codes_are_dropped_or_expanded_without_a_shell() {
        let run = |exec: &str| exec_args(exec, "App Name", Some("appicon"), "/p/app.desktop");
        assert_eq!(run("firefox %u"), ["firefox"]);
        assert_eq!(run("code --unity-launch %F"), ["code", "--unity-launch"]);
        assert_eq!(run("app --file=%f"), ["app", "--file="]);
        assert_eq!(
            run("app %i --name %c"),
            ["app", "--icon", "appicon", "--name", "App Name"]
        );
        assert_eq!(run("app 100%% %k"), ["app", "100%", "/p/app.desktop"]);
        assert_eq!(
            run(r#""/opt/my app/run" --flag "two words" %U"#),
            ["/opt/my app/run", "--flag", "two words"]
        );
        assert_eq!(
            run(r#"sh -c "echo \"hi\" \$HOME""#),
            ["sh", "-c", r#"echo "hi" $HOME"#]
        );
        assert_eq!(run("env A=1   prog"), ["env", "A=1", "prog"]);
        assert_eq!(exec_args("app %i", "n", None, "/p"), ["app"]);
    }
}
