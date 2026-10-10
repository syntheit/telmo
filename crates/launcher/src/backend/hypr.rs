//! Hyprland's window list: which apps are running and how to focus one.

use crate::desktop::exec_args;
use crate::model::AppEntry;
use std::process::{Command, Stdio};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Client {
    pub class: String,
    pub initial_class: String,
    pub address: String,
}

pub fn parse_clients(json: &str) -> Vec<Client> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(json) else {
        return Vec::new();
    };
    value
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|c| {
            let text = |key: &str| c[key].as_str().unwrap_or("").to_string();
            let address = text("address");
            (!address.is_empty()).then(|| Client {
                class: text("class"),
                initial_class: text("initialClass"),
                address,
            })
        })
        .collect()
}

/// The windows now open; empty when Hyprland isn't there.
pub fn clients() -> Vec<Client> {
    let output = Command::new("hyprctl")
        .args(["-j", "clients"])
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output();
    match output {
        Ok(out) if out.status.success() => parse_clients(&String::from_utf8_lossy(&out.stdout)),
        _ => Vec::new(),
    }
}

/// The program an `Exec` line starts, skipping `env A=b` prefixes.
fn program_name(exec: &str) -> Option<String> {
    let args = exec_args(exec, "", None, "");
    for arg in &args {
        if arg == "env" || arg.contains('=') && !arg.starts_with('/') {
            continue;
        }
        return arg.rsplit('/').next().map(String::from);
    }
    None
}

/// Whether `client` is a window of `app`: its class is the app's
/// `StartupWMClass`, its desktop ID, or the program it starts.
pub fn matches(app: &AppEntry, client: &Client) -> bool {
    let id = app.id.strip_suffix(".desktop").unwrap_or(&app.id);
    let exec = app.exec.as_deref().and_then(program_name);
    [app.wm_class.as_deref(), Some(id), exec.as_deref()]
        .into_iter()
        .flatten()
        .any(|name| {
            !name.is_empty()
                && (name.eq_ignore_ascii_case(&client.class)
                    || name.eq_ignore_ascii_case(&client.initial_class))
        })
}

pub fn window_of<'a>(app: &AppEntry, clients: &'a [Client]) -> Option<&'a Client> {
    clients.iter().find(|c| matches(app, c))
}

pub fn focus(address: &str) -> Result<(), String> {
    let status = Command::new("hyprctl")
        .args(["dispatch", "focuswindow", &format!("address:{address}")])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| format!("Couldn't run hyprctl: {e}."))?;
    if status.success() {
        Ok(())
    } else {
        Err("hyprctl couldn't focus the window.".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clients() -> Vec<Client> {
        parse_clients(
            r#"[
              {"address":"0xaa","class":"firefox","initialClass":"firefox","title":"x"},
              {"address":"0xbb","class":"com.mitchellh.ghostty","initialClass":"com.mitchellh.ghostty"},
              {"address":"0xcc","class":"telmo.net","initialClass":"telmo.net"},
              {"address":"","class":"junk"}
            ]"#,
        )
    }

    fn app(id: &str, exec: &str, wm_class: Option<&str>) -> AppEntry {
        let mut app = AppEntry::new(id, "App", "/p");
        app.exec = Some(exec.into());
        app.wm_class = wm_class.map(String::from);
        app
    }

    #[test]
    fn the_window_list_parses() {
        let list = clients();
        assert_eq!(list.len(), 3);
        assert_eq!(list[1].address, "0xbb");
        assert!(parse_clients("not json").is_empty());
    }

    #[test]
    fn a_window_is_found_by_class_id_or_program() {
        let list = clients();
        let by_class = app("x.desktop", "something", Some("Firefox"));
        assert_eq!(window_of(&by_class, &list).unwrap().address, "0xaa");
        let by_id = app("com.mitchellh.ghostty.desktop", "ghostty", None);
        assert_eq!(window_of(&by_id, &list).unwrap().address, "0xbb");
        let by_exec = app("web.desktop", "env MOZ=1 /usr/bin/firefox %u", None);
        assert_eq!(window_of(&by_exec, &list).unwrap().address, "0xaa");
        let none = app("gimp.desktop", "gimp %U", None);
        assert!(window_of(&none, &list).is_none());
    }
}
