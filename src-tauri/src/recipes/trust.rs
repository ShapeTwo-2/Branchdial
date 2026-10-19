//! Which repos' `.branchdial.yml` the user has reviewed and trusted.
//!
//! A cloned repo could ship a harmless-looking button that runs anything, so
//! repo recipes only load after the user has seen the file. Trust is pinned to
//! the file's SHA-256: any change asks again. Stored in ~/.branchdial/trusted.json
//! as { "<repo root>": "<sha256>" }.

use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub fn hash(content: &str) -> String {
    Sha256::digest(content.as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
}

fn key(root: &Path) -> String {
    let s = root.to_string_lossy().replace('\\', "/");
    let s = s.trim_end_matches('/');
    if cfg!(windows) {
        s.to_lowercase()
    } else {
        s.to_string()
    }
}

pub struct TrustStore {
    path: PathBuf,
}

impl TrustStore {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    /// `~/.branchdial/trusted.json`
    pub fn user() -> Option<Self> {
        crate::config::app_dir().map(|d| Self::new(d.join("trusted.json")))
    }

    fn read(&self) -> BTreeMap<String, String> {
        std::fs::read_to_string(&self.path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default()
    }

    /// The hash the user trusted for this repo, if any.
    pub fn trusted_hash(&self, root: &Path) -> Option<String> {
        self.read().remove(&key(root))
    }

    pub fn trust(&self, root: &Path, hash: &str) -> Result<(), String> {
        let mut all = self.read();
        all.insert(key(root), hash.to_string());
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let text = serde_json::to_string_pretty(&all).map_err(|e| e.to_string())?;
        std::fs::write(&self.path, text).map_err(|e| format!("Could not save {}: {e}", self.path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trust_round_trip() {
        let dir = std::env::temp_dir().join(format!("branchdial-trust-{}", std::process::id()));
        let store = TrustStore::new(dir.join("trusted.json"));
        let root = Path::new("/some/repo");
        assert_eq!(store.trusted_hash(root), None);
        store.trust(root, &hash("a")).unwrap();
        assert_eq!(store.trusted_hash(Path::new("/some/repo/")), Some(hash("a")));
        assert_ne!(hash("a"), hash("b"));
        assert_eq!(hash("").len(), 64);
        let _ = std::fs::remove_dir_all(dir);
    }
}
