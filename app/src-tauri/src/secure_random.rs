//! Secrets and unguessable names from the operating system's CSPRNG.

/// `bytes` random bytes from the OS, as lower-case hex.
///
/// Panics if the OS random source fails, which on macOS and Windows only
/// happens when the system itself is broken; there is no safe fallback for
/// a secret.
pub fn hex(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("the OS random source failed");
    buf.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    #[test]
    fn hex_has_the_asked_length_and_differs() {
        let (a, b) = (super::hex(16), super::hex(16));
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
