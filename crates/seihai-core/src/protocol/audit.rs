//! Profile audit (seihai-core#7, "Next 2"): for every profile row and every
//! field the chain ever changes, which step set the value the row ends up
//! with, how that step is backed, and what the row's own step and the
//! setting step say they do not know about it.
//!
//! The chain stores deltas, so a row's value can come from many builds
//! back. A shared login token or an archive label is not evidence that two
//! builds behave alike; this table makes every carried value visible, so
//! "read in this build" and "carried from an older build" are never
//! confused. `documentation/profile-audit.tsv` is this table, and a test
//! keeps it current.

use super::chain::{changes, Change, Step, CHAIN};
use super::Basis;

/// Where one field of one row comes from.
#[derive(Debug, Clone)]
pub struct Source {
    /// Field group and name, as [`Change`] gives them.
    pub area: &'static str,
    pub field: &'static str,
    /// The step that set the value this row has; `None` for `ORIGIN`.
    pub set_by: Option<&'static Step>,
    pub evidence: Evidence,
    /// Unknown notes about this field, from the row's own step and from
    /// the step that set it.
    pub notes: Vec<&'static str>,
}

/// How a row's value for a field is backed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Evidence {
    /// Set by this row's own step, read from its binary.
    ReadHere,
    /// Set by this row's own step, which has no binary.
    InferredHere,
    /// Carried from an earlier step, and this row's step says nothing
    /// about the field: the build was read or run without a difference
    /// showing, or the field was not looked at.
    Carried,
    /// Carried from an earlier step, and this row's own step says the
    /// field was not read or not known: a Seihai fallback.
    CarriedUnknown,
    /// Carried, and this row's step has no binary at all.
    CarriedInferred,
}

impl Evidence {
    pub fn label(self) -> &'static str {
        match self {
            Evidence::ReadHere => "read here",
            Evidence::InferredHere => "inferred here",
            Evidence::Carried => "carried",
            Evidence::CarriedUnknown => "carried, flagged unknown",
            Evidence::CarriedInferred => "carried, no binary",
        }
    }

    /// Native evidence for this exact build.
    pub fn native(self) -> bool {
        self == Evidence::ReadHere
    }
}

/// Whether a step's unknown note (by its free-text `area`) is about a
/// [`Change`] field. `all` and `binary` notes cover the whole row.
fn note_matches(area: &str, ch_area: &str, field: &str) -> bool {
    match area {
        "all" | "binary" => true,
        "pli" => field.starts_with("pli"),
        "plo" => field.starts_with("plo"),
        "protocol" => field.starts_with("plo") || field.starts_with("pli"),
        "listserver" => field == "listserver",
        "login" => field.starts_with("login"),
        "framing" => field.starts_with("gen5"),
        "props" => field.ends_with("player props"),
        "other props" => field == "other-player props",
        "life" => ch_area == "life",
        "files" => ch_area == "files",
        "levels" => ch_area == "levels",
        "drops" => field == "drops",
        "showimg" => field.starts_with("showimg"),
        "tampercheck" => field.starts_with("tamper"),
        // The UI notes are about the chat face, the command count and
        // drawing tiles before the warp (the lobby).
        "ui" => matches!(field, "chat_text" | "gs1_commands" | "login lobby"),
        _ => false,
    }
}

/// Every field the chain changes at least once, in [`super::chain::diff`] order.
pub fn fields() -> Vec<(&'static str, &'static str)> {
    let mut out: Vec<(&'static str, &'static str)> = Vec::new();
    for i in 1..CHAIN.len() {
        for c in changes(i) {
            if !out.contains(&(c.area, c.field)) {
                out.push((c.area, c.field));
            }
        }
    }
    out
}

/// The audit of row `i` (the `i`th step of the chain).
pub fn audit_row(i: usize) -> Vec<Source> {
    let all: Vec<Vec<Change>> = (0..=i).map(changes).collect();
    let own = &CHAIN[i];
    fields()
        .into_iter()
        .map(|(area, field)| {
            let j = (1..=i)
                .rev()
                .find(|&j| all[j].iter().any(|c| c.field == field && c.area == area));
            let set_by = j.map(|j| &CHAIN[j]);
            let notes_of = |s: &'static Step| -> Vec<&'static str> {
                s.unknown
                    .iter()
                    .filter(|u| note_matches(u.area, area, field))
                    .map(|u| u.what)
                    .collect()
            };
            let mut notes = notes_of(own);
            let own_flags = !notes.is_empty();
            if let Some(s) = set_by.filter(|s| !std::ptr::eq(*s, own)) {
                for n in notes_of(s) {
                    if !notes.contains(&n) {
                        notes.push(n);
                    }
                }
            }
            let here = j == Some(i) || (i == 0 && j.is_none());
            let evidence = match (here, own.basis) {
                (true, Basis::Binary) => Evidence::ReadHere,
                (true, Basis::Inferred) => Evidence::InferredHere,
                (false, Basis::Inferred) => Evidence::CarriedInferred,
                (false, Basis::Binary) if own_flags => Evidence::CarriedUnknown,
                (false, Basis::Binary) => Evidence::Carried,
            };
            Source {
                area,
                field,
                set_by,
                evidence,
                notes,
            }
        })
        .collect()
}

fn one_line(s: &str) -> String {
    s.replace(['\t', '\n'], " ")
}

/// The whole audit as TSV: one line per row and field.
pub fn render_tsv() -> String {
    let mut out = String::from("row\tarea\tfield\tset_by\tset_by_basis\tevidence\tnotes\n");
    for (i, step) in CHAIN.iter().enumerate() {
        for s in audit_row(i) {
            let (by, basis) = match s.set_by {
                Some(b) => (
                    b.label,
                    match b.basis {
                        Basis::Binary => "binary",
                        Basis::Inferred => "inferred",
                    },
                ),
                None => ("ORIGIN", "origin"),
            };
            out.push_str(&format!(
                "{}\t{}\t{}\t{}\t{}\t{}\t{}\n",
                step.label,
                s.area,
                s.field,
                by,
                basis,
                s.evidence.label(),
                one_line(&s.notes.join(" | "))
            ));
        }
    }
    out
}

/// One summary line per row: how many of its fields are native evidence,
/// carried, or flagged unknown. Markdown.
pub fn render_summary() -> String {
    let mut out = String::from(
        "| Row | Basis | Read here | Inferred here | Carried | Carried, flagged unknown | Carried, no binary |\n|---|---|---|---|---|---|---|\n",
    );
    for (i, step) in CHAIN.iter().enumerate() {
        let row = audit_row(i);
        let n = |e: Evidence| row.iter().filter(|s| s.evidence == e).count();
        out.push_str(&format!(
            "| {} | {} | {} | {} | {} | {} | {} |\n",
            step.label,
            match step.basis {
                Basis::Binary => "binary",
                Basis::Inferred => "inferred",
            },
            n(Evidence::ReadHere),
            n(Evidence::InferredHere),
            n(Evidence::Carried),
            n(Evidence::CarriedUnknown),
            n(Evidence::CarriedInferred),
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::ClientVersion;

    fn row_of(v: ClientVersion) -> Vec<Source> {
        audit_row(CHAIN.iter().position(|s| s.version == v).unwrap())
    }

    fn get<'a>(row: &'a [Source], field: &str) -> &'a Source {
        row.iter().find(|s| s.field == field).unwrap()
    }

    #[test]
    fn values_are_traced_to_the_step_that_set_them() {
        // 5.12 keeps 5.00r7's PLO set and says its send ids were not read.
        let r = row_of(ClientVersion::V512);
        let plo = get(&r, "plo (server ids handled)");
        assert_eq!(plo.set_by.unwrap().label, "5.00r7");
        assert_eq!(plo.evidence, Evidence::Carried);
        let pli = get(&r, "pli (client ids sent)");
        assert_eq!(pli.evidence, Evidence::CarriedUnknown);
        assert!(pli.notes.iter().any(|n| n.contains("not read")));
        // 5.24r4 reads its own PLO set.
        let r = row_of(ClientVersion::V524R4);
        assert_eq!(
            get(&r, "plo (server ids handled)").evidence,
            Evidence::ReadHere
        );
        // 6.015 has no binary: what it sets is inferred, what it keeps is
        // carried without a binary.
        let r = row_of(ClientVersion::V6015);
        assert_eq!(
            get(&r, "plo (server ids handled)").evidence,
            Evidence::InferredHere
        );
        assert!(r.iter().any(|s| s.evidence == Evidence::CarriedInferred));
    }

    #[test]
    fn every_row_has_every_field_once() {
        let n = fields().len();
        for i in 0..CHAIN.len() {
            let r = audit_row(i);
            assert_eq!(r.len(), n);
        }
    }

    #[test]
    fn the_documented_audit_is_current() {
        let tsv = include_str!("../../../../documentation/profile-audit.tsv").replace("\r\n", "\n");
        assert!(
            tsv == render_tsv(),
            "documentation/profile-audit.tsv is stale; run \
             `cargo run -p seihai-core --example profile_audit -- tsv > documentation/profile-audit.tsv`"
        );
        let md = include_str!("../../../../documentation/profile-audit.md").replace("\r\n", "\n");
        assert!(
            md.contains(&render_summary()),
            "the summary in documentation/profile-audit.md is stale; paste \
             `cargo run -p seihai-core --example profile_audit -- summary` into it"
        );
    }
}
