//! Picks how popups draw pictures.

use ratatui_image::{
    FontSize,
    picker::{Picker, ProtocolType},
};

/// Asks the terminal what it supports, falling back to half blocks.
/// `TELMO_IMAGE_PROTOCOL` wins when the host knows better: SwiftTerm answers
/// the kitty query but doesn't draw ratatui-image's kitty placements, so
/// Telmo.app sets `iterm2`.
pub fn picker() -> Picker {
    let mut picker = Picker::from_query_stdio().unwrap_or_else(|_| Picker::halfblocks());
    if let Some(protocol) = protocol_override(std::env::var("TELMO_IMAGE_PROTOCOL").ok().as_deref())
    {
        picker.set_protocol_type(protocol);
    }
    picker
}

/// Like `picker`, but never asks the terminal: the answer would come back
/// through the same input the user is typing into, and keys typed before the
/// first frame (the launcher is summoned by a hotkey, then typed at) would be
/// eaten. The protocol comes from `TELMO_IMAGE_PROTOCOL` or the terminal's
/// environment, the cell size from the window's pixel size.
#[allow(deprecated)] // from_fontsize is the constructor that doesn't query
pub fn picker_without_query() -> Picker {
    let mut picker = Picker::from_fontsize({
        let (w, h) = cell_size().unwrap_or((8, 16));
        FontSize::new(w, h)
    });
    let env = |name: &str| std::env::var(name).unwrap_or_default();
    let kitty_like = !env("KITTY_WINDOW_ID").is_empty()
        || ["kitty", "ghostty"].iter().any(|t| env("TERM").contains(t))
        || ["ghostty", "kitty"].contains(&env("TERM_PROGRAM").as_str());
    if kitty_like {
        picker.set_protocol_type(ProtocolType::Kitty);
    }
    if let Some(protocol) = protocol_override(std::env::var("TELMO_IMAGE_PROTOCOL").ok().as_deref())
    {
        picker.set_protocol_type(protocol);
    }
    picker
}

/// Pixels per cell, when the terminal reports its pixel size.
fn cell_size() -> Option<(u16, u16)> {
    let mut size = libc::winsize {
        ws_row: 0,
        ws_col: 0,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    // SAFETY: TIOCGWINSZ fills the winsize we point at.
    let ok = unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut size) } == 0;
    (ok && size.ws_col > 0 && size.ws_row > 0 && size.ws_xpixel > 0 && size.ws_ypixel > 0)
        .then(|| (size.ws_xpixel / size.ws_col, size.ws_ypixel / size.ws_row))
}

fn protocol_override(value: Option<&str>) -> Option<ProtocolType> {
    match value? {
        "iterm2" => Some(ProtocolType::Iterm2),
        "sixel" => Some(ProtocolType::Sixel),
        "kitty" => Some(ProtocolType::Kitty),
        "halfblocks" => Some(ProtocolType::Halfblocks),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::protocol_override;
    use ratatui_image::picker::ProtocolType;

    #[test]
    fn the_host_can_name_the_protocol() {
        assert_eq!(
            protocol_override(Some("iterm2")),
            Some(ProtocolType::Iterm2)
        );
        assert_eq!(protocol_override(Some("sixel")), Some(ProtocolType::Sixel));
        assert_eq!(protocol_override(Some("kitty")), Some(ProtocolType::Kitty));
        assert_eq!(
            protocol_override(Some("halfblocks")),
            Some(ProtocolType::Halfblocks)
        );
    }

    #[test]
    fn anything_else_keeps_the_terminal_answer() {
        assert_eq!(protocol_override(None), None);
        assert_eq!(protocol_override(Some("")), None);
        assert_eq!(protocol_override(Some("bogus")), None);
    }
}
