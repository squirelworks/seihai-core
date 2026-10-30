//! Where the settings live. The settings file itself belongs to the client
//! binaries; the shared code only needs the folder, for the buddy, ignore
//! and mute lists, the built-in asset override and the server cache.

use std::path::{Path, PathBuf};

/// The settings file, `seihai.conf`.
pub fn config_path() -> PathBuf {
    if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".config/seihai/seihai.conf")
    } else if let Ok(profile) = std::env::var("USERPROFILE") {
        PathBuf::from(profile).join(".seihai/seihai.conf")
    } else {
        PathBuf::from("seihai.conf")
    }
}

/// The settings directory: the folder `seihai.conf` is in. The buddy,
/// ignore and mute list files live here too.
pub fn config_dir() -> PathBuf {
    config_path()
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}
