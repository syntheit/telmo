//! XDG base directories, with the usual fallback under `$HOME`.

use std::path::PathBuf;

/// `$var` if set and non-empty, else `$HOME/fallback`.
pub fn xdg(var: &str, fallback: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(fallback)))
}

/// `$XDG_CONFIG_HOME` or `~/.config`.
pub fn config() -> Option<PathBuf> {
    xdg("XDG_CONFIG_HOME", ".config")
}

/// `$XDG_DATA_HOME` or `~/.local/share`.
pub fn data() -> Option<PathBuf> {
    xdg("XDG_DATA_HOME", ".local/share")
}

/// `$XDG_CACHE_HOME` or `~/.cache`.
pub fn cache() -> Option<PathBuf> {
    xdg("XDG_CACHE_HOME", ".cache")
}

/// `$XDG_STATE_HOME` or `~/.local/state`.
pub fn state() -> Option<PathBuf> {
    xdg("XDG_STATE_HOME", ".local/state")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variable_wins_unless_empty() {
        // SAFETY: the variable name is private to this test.
        unsafe { std::env::set_var("TELMO_TEST_XDG", "/x") };
        assert_eq!(xdg("TELMO_TEST_XDG", "f"), Some(PathBuf::from("/x")));
        unsafe { std::env::set_var("TELMO_TEST_XDG", "") };
        let home = std::env::var_os("HOME").map(|h| PathBuf::from(h).join("f"));
        assert_eq!(xdg("TELMO_TEST_XDG", "f"), home);
    }
}
