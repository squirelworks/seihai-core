//! Gen-3 LCG insertion codec (cleaned observation of 2.05 client→server).
//!
//! After the first (login) frame, every client→server logical message
//! has one junk byte inserted before zlib compression. Server→client
//! is plain. The insertion position can be 1 (junk byte before the id).
//!
//! Algorithm (cleaned):
//! - iterator seeded `0x04A80B38`
//! - stepped `it = it.wrapping_mul(0x08088405).wrapping_add(key)`
//! - junk byte = `chr(((it >> 16) & 0x7F) + 32)`
//! - insert at 1-based position `(it & 0xFFFF) % len + 1`
//!   (len includes the trailing newline)

/// LCG multiplier used by the gen-3 insertion.
pub const GEN3_MUL: u32 = 0x08088405;
/// LCG seed used by the gen-3 insertion.
pub const GEN3_SEED: u32 = 0x04A80B38;

/// State for the gen-3 insertion iterator.
#[derive(Debug, Clone)]
pub struct Gen3 {
    it: u32,
    key: u8,
}

impl Gen3 {
    /// Create a new iterator. `key` is the login key byte (`rand & 0x7F`).
    pub fn new(key: u8) -> Self {
        Self {
            it: GEN3_SEED,
            key,
        }
    }

    /// Resume from a known iterator state (for replaying captures that
    /// start mid-session).
    pub fn with_state(it: u32, key: u8) -> Self {
        Self { it, key }
    }

    /// Current iterator state.
    pub fn state(&self) -> u32 {
        self.it
    }

    /// Step the iterator and return `(junk_byte, insert_pos)`.
    /// `len` is the message length including the trailing newline.
    /// `insert_pos` is 0-based (so 0 means "before the first byte").
    pub fn next_insertion(&mut self, len: usize) -> (u8, usize) {
        self.it = self.it.wrapping_mul(GEN3_MUL).wrapping_add(self.key as u32);
        let junk = (((self.it >> 16) & 0x7F) + 32) as u8;
        let pos = if len == 0 {
            0
        } else {
            ((self.it & 0xFFFF) as usize) % len
        };
        (junk, pos)
    }

    /// Insert the next junk byte into a logical message.
    /// `msg` should already include the trailing newline.
    pub fn encode(&mut self, msg: &[u8]) -> Vec<u8> {
        let (junk, pos) = self.next_insertion(msg.len());
        let mut out = Vec::with_capacity(msg.len() + 1);
        out.extend_from_slice(&msg[..pos]);
        out.push(junk);
        out.extend_from_slice(&msg[pos..]);
        out
    }

    /// Remove a previously inserted junk byte (for testing / decoding).
    /// `pos` is the 0-based insertion position returned by `next_insertion`.
    pub fn decode(&mut self, encoded: &[u8]) -> Option<Vec<u8>> {
        if encoded.is_empty() {
            return None;
        }
        let len = encoded.len() - 1;
        let (junk, pos) = self.next_insertion(len);
        if pos >= encoded.len() || encoded[pos] != junk {
            return None;
        }
        let mut out = Vec::with_capacity(len);
        out.extend_from_slice(&encoded[..pos]);
        out.extend_from_slice(&encoded[pos + 1..]);
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let key = 0x42;
        let mut enc = Gen3::new(key);
        let mut dec = Gen3::new(key);
        let msg = b"\"..\n";
        let encoded = enc.encode(msg);
        assert_eq!(encoded.len(), msg.len() + 1);
        let decoded = dec.decode(&encoded).unwrap();
        assert_eq!(decoded, msg);
    }

    #[test]
    fn position_can_be_zero() {
        // Just confirm the API allows a zero insertion point.
        let mut g = Gen3::new(1);
        let (junk, pos) = g.next_insertion(1);
        assert!(pos == 0);
        assert!((32..=159).contains(&junk));
    }
}
