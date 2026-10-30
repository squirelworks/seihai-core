//! GOBJSET01 object-set (stamp) parser.
//!
//! An object set is a library of named multi-tile patterns used by
//! the level editor. Each object is a width×height grid of 2-char
//! tile codes, the same encoding as a GLEVNW01 board.

use crate::assets::{tile_id, Tile};

/// One named stamp.
#[derive(Debug, Clone)]
pub struct Stamp {
    pub name: String,
    pub width: usize,
    pub height: usize,
    pub tiles: Vec<Tile>,
}

impl Stamp {
    /// Tile at (x, y), or None if out of bounds.
    pub fn tile_at(&self, x: usize, y: usize) -> Option<Tile> {
        if x >= self.width || y >= self.height {
            return None;
        }
        self.tiles.get(y * self.width + x).copied()
    }

    /// Tile ids in row-major order.
    pub fn ids(&self) -> Vec<u16> {
        self.tiles.iter().copied().map(tile_id).collect()
    }
}

/// A parsed object set.
#[derive(Debug, Clone)]
pub struct ObjectSet {
    pub stamps: Vec<Stamp>,
}

impl ObjectSet {
    pub fn parse(text: &str) -> Result<Self, String> {
        if !text.starts_with("GOBJSET01") {
            return Err("not a GOBJSET01 file".into());
        }
        let mut stamps = Vec::new();
        let mut lines = text.lines().map(|l| l.trim()).filter(|l| !l.is_empty());
        // skip header
        lines.next();

        let mut current: Option<(String, usize, usize, Vec<Tile>)> = None;
        for line in lines {
            if line.starts_with("OBJECT ") {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() < 4 {
                    return Err(format!("bad OBJECT line: {line}"));
                }
                let w: usize = parts[1].parse().map_err(|_| "bad width")?;
                let h: usize = parts[2].parse().map_err(|_| "bad height")?;
                let name = parts[3..].join(" ");
                current = Some((name, w, h, Vec::new()));
            } else if line == "OBJECTEND" {
                if let Some((name, w, h, tiles)) = current.take() {
                    if tiles.len() != w * h {
                        return Err(format!(
                            "{name}: expected {} tiles, got {}",
                            w * h,
                            tiles.len()
                        ));
                    }
                    stamps.push(Stamp {
                        name,
                        width: w,
                        height: h,
                        tiles,
                    });
                }
            } else if let Some((name, w, _, tiles)) = current.as_mut() {
                let width = *w;
                let bytes = line.as_bytes();
                if bytes.len() != width * 2 {
                    return Err(format!("row length {} != {}", bytes.len(), width * 2));
                }
                for i in 0..width {
                    tiles.push([bytes[i * 2], bytes[i * 2 + 1]]);
                }
            }
        }
        Ok(ObjectSet { stamps })
    }

    pub fn load<P: AsRef<std::path::Path>>(path: P) -> Result<Self, String> {
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        Self::parse(&text)
    }

    pub fn find(&self, name: &str) -> Option<&Stamp> {
        self.stamps.iter().find(|s| s.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_small() {
        let text = "GOBJSET01\n\nOBJECT 2 2 Bush\nACAD\nASAT\nOBJECTEND\n";
        let set = ObjectSet::parse(text).unwrap();
        assert_eq!(set.stamps.len(), 1);
        let s = &set.stamps[0];
        assert_eq!(s.name, "Bush");
        assert_eq!(s.width, 2);
        assert_eq!(s.height, 2);
        assert_eq!(s.tiles.len(), 4);
        assert_eq!(s.ids()[0], tile_id(*b"AC"));
    }
}
