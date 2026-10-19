//! User settings from `~/.branchdial/config.yml`. Every field is optional.
//!
//! ```yaml
//! shell:
//!   path: C:\Program Files\Git\bin\bash.exe
//!   args: [--login, -i]
//! startDir: ~/code
//! protectedBranches: [main, master, prod, production, staging, "release/*"]
//! checkForUpdates: false
//! ```

use serde::Deserialize;
use std::path::PathBuf;

pub const APP_DIR_NAME: &str = ".branchdial";

#[derive(Debug, Default, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    pub shell: Option<ShellConfig>,
    pub start_dir: Option<String>,
    /// Replaces the default protected branches (a repo's .branchdial.yml can override again).
    pub protected_branches: Option<Vec<String>>,
    /// Look for a new version on startup (default true).
    pub check_for_updates: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellConfig {
    pub path: String,
    /// When omitted, sensible defaults are picked from the shell's name.
    pub args: Option<Vec<String>>,
}

/// `~/.branchdial`
pub fn app_dir() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(APP_DIR_NAME))
}

/// Expands a leading `~` to the home directory.
pub fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix('~') {
        if rest.is_empty() || rest.starts_with('/') || rest.starts_with('\\') {
            if let Some(home) = dirs::home_dir() {
                return home.join(rest.trim_start_matches(['/', '\\']));
            }
        }
    }
    PathBuf::from(path)
}

/// Loads the config. A missing file is not an error; a broken one returns the
/// defaults plus a warning to show the user.
pub fn load() -> (Config, Option<String>) {
    let Some(path) = app_dir().map(|d| d.join("config.yml")) else {
        return (Config::default(), None);
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Config::default(), None),
        Err(e) => return (Config::default(), Some(format!("Could not read {}: {e}", path.display()))),
    };
    if text.trim().is_empty() {
        return (Config::default(), None);
    }
    match serde_yaml::from_str::<Config>(&text) {
        Ok(c) => (c, None),
        Err(e) => (
            Config::default(),
            Some(format!("Ignoring {} (invalid YAML: {e})", path.display())),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_config() {
        let c: Config = serde_yaml::from_str(
            "shell:\n  path: /bin/zsh\n  args: [-l]\nstartDir: ~/code\ncheckForUpdates: false\n",
        )
        .unwrap();
        let shell = c.shell.unwrap();
        assert_eq!(shell.path, "/bin/zsh");
        assert_eq!(shell.args.unwrap(), vec!["-l"]);
        assert_eq!(c.start_dir.as_deref(), Some("~/code"));
        assert_eq!(c.check_for_updates, Some(false));
    }

    #[test]
    fn rejects_unknown_keys() {
        assert!(serde_yaml::from_str::<Config>("shel: {path: x}").is_err());
    }

    #[test]
    fn expands_tilde() {
        let home = dirs::home_dir().unwrap();
        assert_eq!(expand_home("~"), home);
        assert_eq!(expand_home("~/x"), home.join("x"));
        assert_eq!(expand_home("/abs"), PathBuf::from("/abs"));
    }
}
