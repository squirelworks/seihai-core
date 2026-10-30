//! `levels/tiledefs/*.txt`: which tile sheet a level uses.
//!
//! One line per entry, `image,prefix,type[,x,y]` (Graal_analysis
//! `client-internals/client-assets.md`). `type` 0 is the old pics1 order and
//! 1 the new order; the 2.22 folder also holds type 4. `x` and `y` are
//! optional and default to 0.
//!
//! This is the parser only. How a client matches `prefix` against a level
//! name (whole name, prefix, or contained text) and what types other than
//! 0 and 1 mean are not settled, so nothing here picks a sheet for a level.

use std::fs;
use std::path::Path;

/// One tiledef line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TileDef {
    pub image: String,
    /// Empty for the default sheet (`pics1.png,,0,0,0`).
    pub prefix: String,
    pub kind: u32,
    pub x: i32,
    pub y: i32,
}

/// Parse a tiledefs file. Blank lines and lines that do not have at least
/// image, prefix and a numeric type are skipped.
pub fn parse_tiledefs(text: &str) -> Vec<TileDef> {
    text.lines()
        .filter_map(|l| {
            let l = l.trim();
            if l.is_empty() || l.starts_with("//") {
                return None;
            }
            let f: Vec<&str> = l.split(',').map(str::trim).collect();
            if f.len() < 3 || f[0].is_empty() {
                return None;
            }
            Some(TileDef {
                image: f[0].to_string(),
                prefix: f[1].to_string(),
                kind: f[2].parse().ok()?,
                x: f.get(3).and_then(|v| v.parse().ok()).unwrap_or(0),
                y: f.get(4).and_then(|v| v.parse().ok()).unwrap_or(0),
            })
        })
        .collect()
}

/// Read a tiledefs file (Latin-1 safe).
pub fn load_tiledefs<P: AsRef<Path>>(path: P) -> Result<Vec<TileDef>, String> {
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    Ok(parse_tiledefs(&String::from_utf8_lossy(&bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_shapes_in_the_data_folder() {
        let t = "pics1.png,,0,0,0\nalienpics.png,alien1.nw,4,1728,0\npicso.png,offlinetutorial,1,0,0\n";
        let v = parse_tiledefs(t);
        assert_eq!(v.len(), 3);
        assert_eq!(v[0], TileDef { image: "pics1.png".into(), prefix: String::new(), kind: 0, x: 0, y: 0 });
        assert_eq!((v[1].image.as_str(), v[1].prefix.as_str(), v[1].kind, v[1].x), ("alienpics.png", "alien1.nw", 4, 1728));
        assert_eq!(v[2].kind, 1);
    }

    #[test]
    fn optional_position_and_bad_lines() {
        let v = parse_tiledefs("a.png,p,1\n\n// note\nnoprefix.png\n,x,1\nb.png,q,notnum\r\nc.png,r,2,5,6\r\n");
        assert_eq!(v.len(), 2);
        assert_eq!((v[0].x, v[0].y), (0, 0));
        assert_eq!((v[1].x, v[1].y), (5, 6));
    }
}
