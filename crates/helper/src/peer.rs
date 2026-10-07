//! Who is on the other end of a connection: same user, and a binary signed by
//! our Developer ID team. The check uses the peer's audit token rather than its
//! pid, so a recycled pid can't impersonate the app.

use std::os::unix::net::UnixStream;

/// Team IDs are ten letters/digits; anything else could alter the requirement.
pub fn valid_team_id(team: &str) -> bool {
    team.len() == 10
        && team
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

pub fn requirement(team: &str) -> String {
    format!(r#"anchor apple generic and certificate leaf[subject.OU] = "{team}""#)
}

#[cfg(target_os = "macos")]
pub use mac::verify;

#[cfg(not(target_os = "macos"))]
pub fn verify(_: &UnixStream, _: u32, _: &str) -> Result<(), String> {
    Err("Peer verification only exists on macOS.".to_string())
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{UnixStream, requirement};
    use std::ffi::{CString, c_char, c_int, c_void};
    use std::os::fd::AsRawFd;

    type Ref = *const c_void;
    const UTF8: u32 = 0x0800_0100;
    const SOL_LOCAL: c_int = 0;
    const LOCAL_PEERTOKEN: c_int = 6;

    #[link(name = "System")]
    unsafe extern "C" {
        fn getpeereid(fd: c_int, uid: *mut u32, gid: *mut u32) -> c_int;
        fn getsockopt(
            fd: c_int,
            level: c_int,
            name: c_int,
            value: *mut c_void,
            len: *mut u32,
        ) -> c_int;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFRelease(object: Ref);
        fn CFDataCreate(allocator: Ref, bytes: *const u8, len: isize) -> Ref;
        fn CFStringCreateWithCString(allocator: Ref, text: *const c_char, encoding: u32) -> Ref;
        fn CFDictionaryCreate(
            allocator: Ref,
            keys: *const Ref,
            values: *const Ref,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> Ref;
    }

    #[link(name = "Security", kind = "framework")]
    unsafe extern "C" {
        static kSecGuestAttributeAudit: Ref;
        fn SecCodeCopyGuestWithAttributes(
            host: Ref,
            attributes: Ref,
            flags: u32,
            guest: *mut Ref,
        ) -> i32;
        fn SecRequirementCreateWithString(text: Ref, flags: u32, requirement: *mut Ref) -> i32;
        fn SecCodeCheckValidity(code: Ref, flags: u32, requirement: Ref) -> i32;
    }

    /// Releases a Core Foundation object on drop.
    struct Owned(Ref);

    impl Owned {
        fn new(object: Ref, what: &str) -> Result<Owned, String> {
            if object.is_null() {
                Err(format!("Couldn't create {what}."))
            } else {
                Ok(Owned(object))
            }
        }
    }

    impl Drop for Owned {
        fn drop(&mut self) {
            // SAFETY: we own one reference to a non-null object.
            unsafe { CFRelease(self.0) }
        }
    }

    pub fn verify(stream: &UnixStream, uid: u32, team: &str) -> Result<(), String> {
        let fd = stream.as_raw_fd();
        if peer_uid(fd)? != uid {
            return Err("Peer is a different user.".to_string());
        }
        check_signature(&peer_token(fd)?, &requirement(team))
    }

    fn peer_uid(fd: c_int) -> Result<u32, String> {
        let (mut uid, mut gid) = (u32::MAX, u32::MAX);
        // SAFETY: valid fd and out pointers.
        if unsafe { getpeereid(fd, &mut uid, &mut gid) } != 0 {
            return Err("Couldn't read the peer's user.".to_string());
        }
        Ok(uid)
    }

    fn peer_token(fd: c_int) -> Result<[u8; 32], String> {
        let mut token = [0u8; 32];
        let mut len = token.len() as u32;
        // SAFETY: the buffer is exactly the audit token's 32 bytes.
        let rc = unsafe {
            getsockopt(
                fd,
                SOL_LOCAL,
                LOCAL_PEERTOKEN,
                token.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if rc != 0 || len != 32 {
            return Err("Couldn't read the peer's identity.".to_string());
        }
        Ok(token)
    }

    fn check_signature(token: &[u8; 32], requirement: &str) -> Result<(), String> {
        let text = CString::new(requirement).map_err(|_| "Bad requirement.".to_string())?;
        // SAFETY: every object is checked for null and released by `Owned`;
        // the dictionary retains its key and value, so drop order is safe.
        unsafe {
            let data = Owned::new(
                CFDataCreate(std::ptr::null(), token.as_ptr(), 32),
                "token data",
            )?;
            let keys = [kSecGuestAttributeAudit];
            let values = [data.0];
            let attributes = Owned::new(
                CFDictionaryCreate(
                    std::ptr::null(),
                    keys.as_ptr(),
                    values.as_ptr(),
                    1,
                    &raw const kCFTypeDictionaryKeyCallBacks,
                    &raw const kCFTypeDictionaryValueCallBacks,
                ),
                "attributes",
            )?;
            let mut guest = std::ptr::null();
            if SecCodeCopyGuestWithAttributes(std::ptr::null(), attributes.0, 0, &mut guest) != 0 {
                return Err("Couldn't find the peer's code.".to_string());
            }
            let guest = Owned::new(guest, "guest code")?;
            let text = Owned::new(
                CFStringCreateWithCString(std::ptr::null(), text.as_ptr(), UTF8),
                "requirement text",
            )?;
            let mut required = std::ptr::null();
            if SecRequirementCreateWithString(text.0, 0, &mut required) != 0 {
                return Err("The code requirement is invalid.".to_string());
            }
            let required = Owned::new(required, "requirement")?;
            if SecCodeCheckValidity(guest.0, 0, required.0) != 0 {
                return Err("Peer isn't signed by the expected team.".to_string());
            }
        }
        Ok(())
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn an_unsigned_peer_is_refused() {
            let (a, _b) = UnixStream::pair().unwrap();
            // SAFETY: getuid has no preconditions.
            let uid = unsafe { getuid() };
            assert!(verify(&a, uid, "6NHZWHQX37").is_err());
            assert!(verify(&a, uid.wrapping_add(1), "6NHZWHQX37").is_err());
        }

        unsafe extern "C" {
            fn getuid() -> u32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn team_ids_are_strict() {
        assert!(valid_team_id("6NHZWHQX37"));
        for bad in ["", "6NHZWHQX3", "6nhzwhqx37", "6NHZWHQX37\"", "6NHZWHQX3 "] {
            assert!(!valid_team_id(bad), "{bad:?}");
        }
    }

    #[test]
    fn requirement_names_the_team() {
        assert_eq!(
            requirement("6NHZWHQX37"),
            r#"anchor apple generic and certificate leaf[subject.OU] = "6NHZWHQX37""#
        );
    }
}
