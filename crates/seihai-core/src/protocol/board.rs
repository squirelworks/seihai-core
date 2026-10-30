//! Board modify packet helpers.

/// A rectangular tile update (PLO_BOARDMODIFY / PLO_LEVELBOARD payload).
#[derive(Debug, Clone)]
pub struct BoardModify {
    pub x: u8,
    pub y: u8,
    pub width: u8,
    pub height: u8,
    pub tiles: Vec<u8>, // raw 2-byte tile codes
}

impl BoardModify {
    /// Encode to the on-wire form used by `getBoardStr()`.
    pub fn encode(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(4 + self.tiles.len());
        out.push(self.x);
        out.push(self.y);
        out.push(self.width);
        out.push(self.height);
        out.extend_from_slice(&self.tiles);
        out
    }

    /// Decode a single modify record.
    pub fn decode(data: &[u8]) -> Option<Self> {
        if data.len() < 4 {
            return None;
        }
        Some(Self {
            x: data[0],
            y: data[1],
            width: data[2],
            height: data[3],
            tiles: data[4..].to_vec(),
        })
    }
}
