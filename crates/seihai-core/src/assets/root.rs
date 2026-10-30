//! Offline asset root.
//!
//! Expected layout for a local asset tree used by the offline client:
//! ```text
//! <root>/
//!   pics1.png          # 2048×512 tile sheet
//!   tiledefs.txt       # or arrays.dat
//!   levels/            # .nw files
//!   gani/              # .gani files (the 2.22 folder has levels/ganis/)
//!   sounds/            # .wav files (levels/sounds/ is tried too)
//!   bodies/body.png    # player sheets (also found at the root or
//!   heads/head0.png    #   under levels/bodies, levels/heads)
//!   sprites.png        # shadow and effects
//! ```

use std::path::{Path, PathBuf};

/// Paths into a local asset tree.
#[derive(Debug, Clone)]
pub struct AssetRoot {
    pub root: PathBuf,
}

impl AssetRoot {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn pics1(&self) -> PathBuf {
        self.root.join("pics1.png")
    }

    pub fn tiledefs(&self) -> PathBuf {
        self.root.join("tiledefs.txt")
    }

    pub fn arrays_dat(&self) -> PathBuf {
        self.root.join("arrays.dat")
    }

    pub fn levels_dir(&self) -> PathBuf {
        self.root.join("levels")
    }

    pub fn level(&self, name: &str) -> PathBuf {
        self.levels_dir().join(name)
    }

    pub fn gani_dir(&self) -> PathBuf {
        self.root.join("gani")
    }

    pub fn gani(&self, name: &str) -> PathBuf {
        self.gani_dir().join(name)
    }

    /// Where sound effects live: `<root>/sounds`, as in every client
    /// folder, or `<root>/levels/sounds`. The first that exists wins.
    pub fn sounds_dir(&self) -> Option<PathBuf> {
        [self.root.join("sounds"), self.root.join("levels").join("sounds")]
            .into_iter()
            .find(|p| p.is_dir())
    }

    /// Find a GANI by file name in `gani/`, `levels/ganis/` (the 2.22
    /// folder) or the root.
    pub fn find_gani(&self, name: &str) -> Option<PathBuf> {
        [
            self.gani_dir().join(name),
            self.root.join("levels").join("ganis").join(name),
            self.root.join(name),
        ]
        .into_iter()
        .find(|p| p.is_file())
    }

    /// Find a player image such as `body.png` in `<dir>/`, `levels/<dir>/`
    /// or the root itself.
    pub fn find_image(&self, dir: &str, name: &str) -> Option<PathBuf> {
        [
            self.root.join(dir).join(name),
            self.root.join("levels").join(dir).join(name),
            self.root.join(name),
        ]
        .into_iter()
        .find(|p| p.is_file())
    }

    /// True if the root contains at least a `pics1.png`.
    pub fn is_valid(&self) -> bool {
        self.pics1().is_file()
    }
}

impl AsRef<Path> for AssetRoot {
    fn as_ref(&self) -> &Path {
        &self.root
    }
}
