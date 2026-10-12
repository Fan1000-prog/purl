//! Secrets and unguessable names from the operating system's CSPRNG.
//!
//! There is no fallback: if the OS source fails, callers fail closed (Smart
//! cleanup reports failed and Fast is used) rather than use weaker bytes.

/// The OS random source failed.
#[derive(Debug, PartialEq, thiserror::Error)]
#[error("the OS random source failed")]
pub struct RandomUnavailable;

/// `bytes` random bytes from the OS, as lower-case hex.
pub fn random_hex(bytes: usize) -> Result<String, RandomUnavailable> {
    hex_with(bytes, getrandom::fill)
}

fn hex_with(
    bytes: usize,
    fill: impl FnOnce(&mut [u8]) -> Result<(), getrandom::Error>,
) -> Result<String, RandomUnavailable> {
    let mut buf = vec![0u8; bytes];
    fill(&mut buf).map_err(|_| RandomUnavailable)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failing_os_source_is_an_error_not_a_weaker_value() {
        let r = hex_with(8, |_| Err(getrandom::Error::UNSUPPORTED));
        assert!(matches!(r, Err(RandomUnavailable)));
    }

    #[test]
    fn hex_has_the_asked_length_and_differs() {
        let (a, b) = (random_hex(16).unwrap(), random_hex(16).unwrap());
        assert_eq!(a.len(), 32);
        assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }
}
