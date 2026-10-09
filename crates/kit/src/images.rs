//! Picks how popups draw pictures.

use ratatui_image::picker::{Picker, ProtocolType};

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
