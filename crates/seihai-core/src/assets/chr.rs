//! Classic character files (`.chr`, `.chi`): one line, `CHR:1.00`, the
//! name, `=`, the head picture, the sex and the five body colours by name,
//! as Graal Character Designer 1.2.7 and GChar 1.0 (1999) write them:
//!
//! ```text
//! CHR:1.00Blade=head189.gif,male,black,black,gray,gray,red
//! ```
//!
//! The colours are in prop 13's order (skin, coat, sleeves, shoes, belt;
//! [`crate::looks`]). The file does not label them; the order is the
//! client's. Graal_analysis `tools/classic-tools/character-designer.md`.

use crate::looks::{color_by_name, Looks};
use crate::scripting::engine::vocab::COLORS;

/// The magic at the start of the line.
pub const CHR_MAGIC: &str = "CHR:1.00";

/// One character.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Character {
    pub name: String,
    /// The head picture (`head189.gif`).
    pub head: String,
    pub female: bool,
    /// Skin, coat, sleeves, shoes, belt, as colour-list indexes.
    pub colors: [u8; 5],
}

impl Character {
    /// Read a `.chr` / `.chi` file's text (its first line).
    pub fn parse(text: &str) -> Result<Self, String> {
        let line = text
            .trim_start_matches('\u{feff}')
            .lines()
            .next()
            .unwrap_or("")
            .trim_end();
        let rest = line
            .strip_prefix(CHR_MAGIC)
            .ok_or_else(|| format!("not a character file (no {CHR_MAGIC})"))?;
        let (name, fields) = rest
            .split_once('=')
            .ok_or("no `=` after the character's name")?;
        let fields: Vec<&str> = fields.split(',').map(str::trim).collect();
        let [head, sex, c0, c1, c2, c3, c4] = fields.as_slice() else {
            return Err(format!(
                "want head,sex and five colours after `=`, found {} fields",
                fields.len()
            ));
        };
        let female = match sex.to_ascii_lowercase().as_str() {
            "male" => false,
            "female" => true,
            other => return Err(format!("sex {other}: want male or female")),
        };
        let mut colors = [0u8; 5];
        for (slot, name) in colors.iter_mut().zip([c0, c1, c2, c3, c4]) {
            *slot = color_by_name(name).ok_or_else(|| format!("{name} is not a colour"))?;
        }
        Ok(Self {
            name: name.to_string(),
            head: head.to_string(),
            female,
            colors,
        })
    }

    /// The file's line (no line break; the samples have none).
    pub fn write(&self) -> String {
        let colors: Vec<&str> = self
            .colors
            .iter()
            .map(|&c| COLORS.get(usize::from(c)).copied().unwrap_or("black"))
            .collect();
        format!(
            "{CHR_MAGIC}{}={},{},{}",
            self.name,
            self.head,
            if self.female { "female" } else { "male" },
            colors.join(",")
        )
    }

    /// The character's looks: its head and colours on the default body.
    pub fn looks(&self) -> Looks {
        Looks {
            head: Some(self.head.clone()),
            colors: self.colors,
            ..Looks::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_writes_the_designer_line() {
        let c =
            Character::parse("CHR:1.00Blade=head189.gif,male,black,black,gray,gray,red").unwrap();
        assert_eq!(c.name, "Blade");
        assert_eq!(c.head, "head189.gif");
        assert!(!c.female);
        assert_eq!(c.colors, [18, 18, 17, 17, 4]);
        assert_eq!(
            c.write(),
            "CHR:1.00Blade=head189.gif,male,black,black,gray,gray,red"
        );
        assert_eq!(c.looks().head_name(), "head189.gif");
        assert_eq!(c.looks().colors, c.colors);
        let anna = Character::parse(
            "CHR:1.00Anna=head5.gif,female,cynober,pink,white,brown,darkpurple\r\n",
        )
        .unwrap();
        assert!(anna.female);
        assert_eq!(anna.colors, [13, 3, 0, 12, 15]);
    }

    #[test]
    fn refuses_what_the_designer_would_not_write() {
        assert!(Character::parse("Blade=head.gif,male,a,b,c,d,e").is_err());
        assert!(Character::parse("CHR:1.00Blade").is_err());
        assert!(Character::parse("CHR:1.00B=h.gif,male,black").is_err());
        assert!(Character::parse("CHR:1.00B=h.gif,robot,black,black,black,black,black").is_err());
        assert!(
            Character::parse("CHR:1.00B=h.gif,male,black,black,black,black,mauve")
                .unwrap_err()
                .contains("mauve")
        );
    }
}
