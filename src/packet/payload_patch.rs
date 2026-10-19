use std::error::Error;
use std::fmt;

/// Fixed-capacity byte replacement patch applied during RTP payload serialization.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PayloadPatch {
    len: u8,
    offsets: [u16; Self::MAX],
    bytes: [u8; Self::MAX],
}

/// Errors returned when adding a byte replacement to [`PayloadPatch`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PayloadPatchError {
    /// All [`PayloadPatch::MAX`] slots are already occupied.
    Full,
    /// The byte offset does not fit in a `u16`.
    OffsetTooLarge,
}

impl fmt::Display for PayloadPatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Full => write!(f, "PayloadPatch is full (max {} bytes)", PayloadPatch::MAX),
            Self::OffsetTooLarge => write!(f, "PayloadPatch offset exceeds u16::MAX"),
        }
    }
}

impl Error for PayloadPatchError {}

impl PayloadPatch {
    /// Maximum number of byte replacements in a single patch.
    pub const MAX: usize = 4;

    /// Create an empty payload patch.
    pub fn new() -> Self {
        Self::default()
    }

    /// Record a byte replacement at `offset`.
    pub fn push(&mut self, offset: usize, byte: u8) -> Result<(), PayloadPatchError> {
        let index = usize::from(self.len);
        if index >= Self::MAX {
            return Err(PayloadPatchError::Full);
        }
        let Ok(offset_u16) = u16::try_from(offset) else {
            return Err(PayloadPatchError::OffsetTooLarge);
        };
        self.offsets[index] = offset_u16;
        self.bytes[index] = byte;
        self.len += 1;
        Ok(())
    }

    /// Return `true` if no replacements have been recorded.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// Apply recorded byte replacements to `dst` in-place.
    ///
    /// Offsets past `dst.len()` are ignored and logged at `debug`.
    pub(crate) fn apply(&self, dst: &mut [u8]) {
        for i in 0..usize::from(self.len) {
            let offset = usize::from(self.offsets[i]);
            if let Some(slot) = dst.get_mut(offset) {
                *slot = self.bytes[i];
            } else {
                debug!(
                    "PayloadPatch offset {} exceeds payload length {}; ignoring",
                    offset,
                    dst.len()
                );
            }
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn push_rejects_a_fifth_replacement() {
        let mut p = PayloadPatch::new();
        assert!(p.is_empty());
        for i in 0..PayloadPatch::MAX {
            p.push(i, i as u8).expect("first 4 succeed");
        }
        assert!(!p.is_empty());
        assert_eq!(p.push(4, 0xaa), Err(PayloadPatchError::Full));
    }

    #[test]
    fn push_rejects_offset_over_u16() {
        let mut p = PayloadPatch::new();
        assert_eq!(
            p.push(usize::from(u16::MAX) + 1, 0x11),
            Err(PayloadPatchError::OffsetTooLarge)
        );
        assert!(p.is_empty());
    }

    #[test]
    fn apply_replaces_only_listed_bytes() {
        let mut p = PayloadPatch::new();
        p.push(1, 0xaa).unwrap();
        p.push(3, 0xbb).unwrap();
        let mut buf = [0x10, 0x20, 0x30, 0x40, 0x50];
        p.apply(&mut buf);
        assert_eq!(buf, [0x10, 0xaa, 0x30, 0xbb, 0x50]);
    }

    #[test]
    fn apply_ignores_offsets_past_the_end() {
        let mut p = PayloadPatch::new();
        p.push(0, 0x99).unwrap();
        p.push(10, 0xff).unwrap();
        let mut buf = [0x01, 0x02, 0x03];
        p.apply(&mut buf);
        assert_eq!(buf, [0x99, 0x02, 0x03]);
    }
}
