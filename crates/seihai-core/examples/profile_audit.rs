//! Print the profile audit (seihai-core#7): `tsv` for
//! `documentation/profile-audit.tsv`, `summary` for the table in
//! `documentation/profile-audit.md`, `connections` for
//! `documentation/connection-capabilities.md`.

fn main() {
    use seihai_core::protocol::audit;
    match std::env::args().nth(1).as_deref() {
        Some("summary") => print!("{}", audit::render_summary()),
        Some("connections") => print!("{}", seihai_core::protocol::connect::render()),
        _ => print!("{}", audit::render_tsv()),
    }
}
