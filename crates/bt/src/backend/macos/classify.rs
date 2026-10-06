//! Class of device (Bluetooth assigned numbers, baseband section 2.8) to `Kind`.

use crate::model::Kind;

const MAJOR_COMPUTER: u32 = 0x01;
const MAJOR_PHONE: u32 = 0x02;
const MAJOR_AUDIO_VIDEO: u32 = 0x04;
const MAJOR_PERIPHERAL: u32 = 0x05;

pub fn kind(class_of_device: u32) -> Kind {
    let major = (class_of_device >> 8) & 0x1f;
    let minor = (class_of_device >> 2) & 0x3f;
    match major {
        MAJOR_COMPUTER => Kind::Computer,
        MAJOR_PHONE => Kind::Phone,
        MAJOR_AUDIO_VIDEO => audio_video(minor),
        MAJOR_PERIPHERAL => peripheral(minor),
        _ => Kind::Other,
    }
}

fn audio_video(minor: u32) -> Kind {
    match minor {
        // Wearable headset, hands-free, headphones, portable audio.
        0x01 | 0x02 | 0x06 | 0x07 => Kind::Headphones,
        // Loudspeaker, car audio, hi-fi audio.
        0x05 | 0x08 | 0x0a => Kind::Speaker,
        _ => Kind::Other,
    }
}

/// The top two bits of the minor class say keyboard or pointing device, the
/// low four bits the kind of joystick-like device.
fn peripheral(minor: u32) -> Kind {
    match (minor >> 4, minor & 0x0f) {
        (0b10, _) => Kind::Mouse,
        (0b01 | 0b11, _) => Kind::Keyboard,
        (_, 0x01 | 0x02) => Kind::Gamepad,
        _ => Kind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn class(major: u32, minor: u32) -> u32 {
        (major << 8) | (minor << 2)
    }

    #[test]
    fn maps_common_devices() {
        assert_eq!(kind(class(4, 6)), Kind::Headphones);
        assert_eq!(kind(class(4, 5)), Kind::Speaker);
        assert_eq!(kind(class(5, 0x10)), Kind::Keyboard);
        assert_eq!(kind(class(5, 0x20)), Kind::Mouse);
        assert_eq!(kind(class(5, 0x02)), Kind::Gamepad);
        assert_eq!(kind(class(2, 3)), Kind::Phone);
        assert_eq!(kind(class(1, 3)), Kind::Computer);
        assert_eq!(kind(0), Kind::Other);
    }
}
