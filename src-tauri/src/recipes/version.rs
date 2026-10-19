//! Version bumping for the release recipe: read the current version from the
//! project's manifests (or the latest `vX.Y.Z` tag), compute the next one,
//! and generate the visible shell commands that update each manifest.

use super::template::sh_quote;
use crate::git;
use std::fmt;
use std::path::Path;

/// Checked in this order when a recipe doesn't list its own files.
pub const DEFAULT_FILES: [&str; 5] =
    ["package.json", "Cargo.toml", "src-tauri/Cargo.toml", "tauri.conf.json", "src-tauri/tauri.conf.json"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Semver(pub u64, pub u64, pub u64);

impl Semver {
    pub fn parse(s: &str) -> Option<Semver> {
        let s = s.strip_prefix('v').unwrap_or(s);
        let mut it = s.split('.');
        let v = Semver(it.next()?.parse().ok()?, it.next()?.parse().ok()?, it.next()?.parse().ok()?);
        it.next().is_none().then_some(v)
    }

    /// `patch` / `minor` / `major`, or an explicit `X.Y.Z` that must be newer.
    pub fn bump(self, how: &str) -> Result<Semver, String> {
        let Semver(ma, mi, pa) = self;
        match how {
            "patch" => Ok(Semver(ma, mi, pa + 1)),
            "minor" => Ok(Semver(ma, mi + 1, 0)),
            "major" => Ok(Semver(ma + 1, 0, 0)),
            other => match Semver::parse(other) {
                Some(v) if v > self => Ok(v),
                Some(v) => Err(format!("{v} is not newer than {self}")),
                None => Err(format!("{other:?} is not patch, minor, major or a version like 1.2.3")),
            },
        }
    }
}

impl fmt::Display for Semver {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}.{}.{}", self.0, self.1, self.2)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    PackageJson,
    CargoToml,
    /// Any other JSON file with a top-level "version" (tauri.conf.json, ...).
    Json,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    /// Relative to the repo root, with forward slashes.
    pub rel: String,
    pub kind: Kind,
    pub version: String,
    /// Cargo package name, used to update Cargo.lock.
    pub crate_name: Option<String>,
}

fn kind_of(rel: &str) -> Option<Kind> {
    let name = rel.rsplit('/').next().unwrap_or(rel);
    if name == "package.json" {
        Some(Kind::PackageJson)
    } else if name == "Cargo.toml" {
        Some(Kind::CargoToml)
    } else if name.ends_with(".json") {
        Some(Kind::Json)
    } else {
        None
    }
}

/// Top-level `version` of a JSON document.
fn json_version(text: &str) -> Option<String> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    v.get("version")?.as_str().map(String::from)
}

/// `(name, version)` from the `[package]` table. `version.workspace = true` yields no version.
fn cargo_package(text: &str) -> (Option<String>, Option<String>) {
    let (mut name, mut version) = (None, None);
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim();
        let unquoted = value.strip_prefix('"').and_then(|v| v.split('"').next()).map(String::from);
        match key.trim() {
            "name" => name = unquoted,
            "version" => version = unquoted,
            _ => {}
        }
    }
    (name, version)
}

/// Manifests from `files` that exist and carry a version.
pub fn scan(root: &Path, files: &[String]) -> Result<Vec<Manifest>, String> {
    let mut out = Vec::new();
    for rel in files {
        let rel = rel.replace('\\', "/");
        let kind = kind_of(&rel).ok_or_else(|| format!("{rel}: only package.json, Cargo.toml and .json files are supported"))?;
        let Ok(text) = std::fs::read_to_string(root.join(&rel)) else { continue };
        let (version, crate_name) = match kind {
            Kind::PackageJson | Kind::Json => (json_version(&text), None),
            Kind::CargoToml => {
                let (name, version) = cargo_package(&text);
                (version, name)
            }
        };
        if let Some(version) = version {
            out.push(Manifest { rel, kind, version, crate_name });
        }
    }
    Ok(out)
}

/// The highest `vX.Y.Z` tag, for repos without manifests.
pub fn latest_tag(root: &Path) -> Option<Semver> {
    git::git(root, &["tag", "--list", "v*"])?.lines().filter_map(Semver::parse).max()
}

#[derive(Debug)]
pub struct Bump {
    pub current: Semver,
    pub new: Semver,
    /// Commands that edit the manifests, in order.
    pub commands: Vec<String>,
    /// Files those commands change (manifests plus lock files), for git add/commit.
    pub files: Vec<String>,
    /// Non-fatal notes for the preview.
    pub warnings: Vec<String>,
}

/// Reads the current version, checks every manifest agrees, and builds the edit commands.
pub fn plan_bump(root: &Path, files: &[String], how: &str) -> Result<Bump, String> {
    let manifests = scan(root, files)?;
    let current = match manifests.first() {
        Some(m) => {
            let v = Semver::parse(&m.version)
                .ok_or_else(|| format!("{} has version {:?}; only X.Y.Z is supported", m.rel, m.version))?;
            let mismatched: Vec<_> = manifests.iter().filter(|o| Semver::parse(&o.version) != Some(v)).collect();
            if !mismatched.is_empty() {
                let all: Vec<_> = manifests.iter().map(|m| format!("{} {}", m.rel, m.version)).collect();
                return Err(format!("Versions don't match: {}. Make them equal first.", all.join(", ")));
            }
            v
        }
        None => latest_tag(root).ok_or("No version found: none of the version files exist and there is no vX.Y.Z tag.")?,
    };
    let new = current.bump(how)?;
    let new_s = new.to_string();

    let mut commands = Vec::new();
    let mut changed: Vec<String> = Vec::new();
    for m in &manifests {
        let (dir, _) = m.rel.rsplit_once('/').unwrap_or(("", &m.rel));
        let sibling = |name: &str| if dir.is_empty() { name.to_string() } else { format!("{dir}/{name}") };
        match m.kind {
            Kind::PackageJson => {
                // npm also updates package-lock.json / npm-shrinkwrap.json.
                let npm = format!("npm version {} --no-git-tag-version --allow-same-version", sh_quote(&new_s));
                commands.push(if dir.is_empty() { npm } else { format!("(cd {} && {npm})", sh_quote(dir)) });
                changed.push(m.rel.clone());
                for lock in ["package-lock.json", "npm-shrinkwrap.json"] {
                    if root.join(sibling(lock)).is_file() {
                        changed.push(sibling(lock));
                    }
                }
            }
            Kind::CargoToml => {
                // `new_s` is digits and dots only, so it is safe inside the perl program.
                commands.push(format!(
                    r#"perl -0pi -e 's/^(version\s*=\s*")[^"]*"/${{1}}{new_s}"/m' {}"#,
                    sh_quote(&m.rel)
                ));
                changed.push(m.rel.clone());
                let lock = [sibling("Cargo.lock"), "Cargo.lock".to_string()]
                    .into_iter()
                    .find(|l| root.join(l).is_file());
                let name_ok = |n: &&String| n.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
                if let (Some(lock), Some(name)) = (lock, m.crate_name.as_ref().filter(name_ok)) {
                    commands.push(format!(
                        r#"perl -0pi -e 's/(\[\[package\]\]\r?\nname = "{name}"\r?\nversion = ")[^"]*"/${{1}}{new_s}"/' {}"#,
                        sh_quote(&lock)
                    ));
                    if !changed.contains(&lock) {
                        changed.push(lock);
                    }
                }
            }
            Kind::Json => {
                commands.push(format!(
                    r#"perl -0pi -e 's/("version"\s*:\s*")[^"]*"/${{1}}{new_s}"/' {}"#,
                    sh_quote(&m.rel)
                ));
                changed.push(m.rel.clone());
            }
        }
    }

    let mut warnings = Vec::new();
    if git::git(root, &["rev-parse", "-q", "--verify", &format!("refs/tags/v{new_s}")]).is_some() {
        return Err(format!("Tag v{new_s} already exists."));
    }
    if !changed.is_empty() {
        let mut args = vec!["status", "--porcelain", "--"];
        args.extend(changed.iter().map(String::as_str));
        if let Some(dirty) = git::git(root, &args).filter(|s| !s.trim().is_empty()) {
            let files: Vec<_> = dirty.lines().map(|l| l.get(3..).unwrap_or(l)).collect();
            warnings.push(format!(
                "Uncommitted changes in {} will be included in the release commit.",
                files.join(", ")
            ));
        }
    }
    Ok(Bump { current, new, commands, files: changed, warnings })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::TempRepo;

    #[test]
    fn semver() {
        let v = Semver::parse("v1.2.3").unwrap();
        assert_eq!(v.bump("patch").unwrap().to_string(), "1.2.4");
        assert_eq!(v.bump("minor").unwrap().to_string(), "1.3.0");
        assert_eq!(v.bump("major").unwrap().to_string(), "2.0.0");
        assert_eq!(v.bump("1.5.0").unwrap().to_string(), "1.5.0");
        assert!(v.bump("1.2.3").is_err());
        assert!(v.bump("huge").is_err());
        assert!(Semver::parse("1.2").is_none());
        assert!(Semver::parse("1.2.3-beta").is_none());
    }

    #[test]
    fn cargo_parsing() {
        let t = "[package]\r\nname = \"my-app\"\r\nversion = \"0.3.1\" # c\r\n[dependencies]\r\nx = { version = \"9\" }\r\n";
        assert_eq!(cargo_package(t), (Some("my-app".into()), Some("0.3.1".into())));
        assert_eq!(cargo_package("[package]\nname = \"a\"\nversion.workspace = true\n").1, None);
    }

    fn write(r: &TempRepo, rel: &str, body: &str) {
        let p = r.work.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }

    fn files() -> Vec<String> {
        DEFAULT_FILES.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plans_commands_for_existing_manifests() {
        let r = TempRepo::new("ver");
        write(&r, "package.json", "{\n  \"name\": \"x\",\n  \"version\": \"0.1.0\"\n}\n");
        write(&r, "package-lock.json", "{}");
        write(&r, "src-tauri/Cargo.toml", "[package]\nname = \"x-app\"\nversion = \"0.1.0\"\n");
        write(&r, "src-tauri/Cargo.lock", "");
        write(&r, "src-tauri/tauri.conf.json", "{ \"version\": \"0.1.0\" }");
        r.git(&["add", "-A"]);
        r.git(&["commit", "-q", "-m", "manifests"]);

        let b = plan_bump(&r.work, &files(), "minor").unwrap();
        assert_eq!(b.new.to_string(), "0.2.0");
        assert_eq!(b.commands.len(), 4);
        assert_eq!(b.commands[0], "npm version '0.2.0' --no-git-tag-version --allow-same-version");
        assert_eq!(
            b.files,
            vec!["package.json", "package-lock.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/tauri.conf.json"]
        );
        assert!(b.warnings.is_empty());

        write(&r, "src-tauri/tauri.conf.json", "{ \"version\": \"0.0.9\" }");
        assert!(plan_bump(&r.work, &files(), "patch").unwrap_err().contains("don't match"));
    }

    #[test]
    fn falls_back_to_tags_and_refuses_existing_tag() {
        let r = TempRepo::new("vertag");
        assert!(plan_bump(&r.work, &files(), "patch").is_err());
        r.git(&["tag", "v1.0.0"]);
        r.git(&["tag", "v1.0.1"]);
        let b = plan_bump(&r.work, &files(), "patch").unwrap();
        assert_eq!(b.new.to_string(), "1.0.2");
        assert!(b.commands.is_empty() && b.files.is_empty());
        assert!(plan_bump(&r.work, &files(), "1.0.1").unwrap_err().contains("not newer"));
        r.git(&["tag", "v1.1.0"]);
        assert!(plan_bump(&r.work, &files(), "1.1.0").is_err());
    }

    /// The generated perl edits really work (bash + perl from Git for Windows on Windows).
    #[test]
    fn edit_commands_apply() {
        let r = TempRepo::new("veredit");
        write(&r, "Cargo.toml", "[package]\r\nname = \"demo\"\r\nversion = \"1.0.0\"\r\n\r\n[dependencies]\r\nserde = { version = \"1\" }\r\n");
        write(&r, "Cargo.lock", "[[package]]\nname = \"other\"\nversion = \"1.0.0\"\n\n[[package]]\nname = \"demo\"\nversion = \"1.0.0\"\n");
        write(&r, "tauri.conf.json", "{\n  \"productName\": \"D\",\n  \"version\": \"1.0.0\",\n  \"x\": { \"version\": \"7\" }\n}\n");
        let b = plan_bump(&r.work, &files(), "patch").unwrap();
        #[cfg(windows)]
        let bash = crate::shell::find_git_bash().unwrap();
        #[cfg(not(windows))]
        let bash = std::path::PathBuf::from("/bin/bash");
        for c in &b.commands {
            let ok = std::process::Command::new(&bash).args(["--norc", "-c", c]).current_dir(&r.work).status().unwrap();
            assert!(ok.success(), "{c}");
        }
        let read = |f: &str| std::fs::read_to_string(r.work.join(f)).unwrap();
        assert_eq!(cargo_package(&read("Cargo.toml")).1.as_deref(), Some("1.0.1"));
        assert!(read("Cargo.toml").contains("serde = { version = \"1\" }\r\n"));
        assert!(read("Cargo.lock").contains("name = \"other\"\nversion = \"1.0.0\""));
        assert!(read("Cargo.lock").contains("name = \"demo\"\nversion = \"1.0.1\""));
        assert_eq!(json_version(&read("tauri.conf.json")).as_deref(), Some("1.0.1"));
        assert!(read("tauri.conf.json").contains("\"version\": \"7\""));
    }
}
