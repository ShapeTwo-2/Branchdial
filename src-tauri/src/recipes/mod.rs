//! Recipes: YAML workflow definitions, rendered into shell commands and run
//! step by step in the terminal.

pub mod loader;
pub mod model;
pub mod plan;
pub mod template;
pub mod trust;
pub mod version;

use crate::git;
use crate::pty::PtyState;
use crate::runner::RunMeta;
use model::{InputDef, Recipe, Source};
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, State};

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeView {
    id: String,
    name: String,
    description: Option<String>,
    source: Source,
    confirm: bool,
    inputs: Vec<InputDef>,
}

impl From<&Recipe> for RecipeView {
    fn from(r: &Recipe) -> Self {
        Self {
            id: r.id.clone(),
            name: r.name.clone(),
            description: r.description.clone(),
            source: r.source,
            confirm: r.confirm,
            inputs: r.inputs.clone(),
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeList {
    cwd: Option<PathBuf>,
    repo_root: Option<PathBuf>,
    recipes: Vec<RecipeView>,
    errors: Vec<loader::LoadError>,
    global_file: Option<PathBuf>,
    /// The repo's .branchdial.yml, if it hasn't been trusted in its current form.
    pending_trust: Option<loader::PendingTrust>,
}

fn find(cwd: Option<&Path>, id: &str) -> Result<(Recipe, git::RepoContext), String> {
    if git::install().is_none() {
        return Err(format!("Git was not found. {}", git::install_hint()));
    }
    let mut ctx = cwd.map(git::context).unwrap_or_default();
    let loaded = loader::load(ctx.root.as_deref());
    ctx.protected = loaded.protected.unwrap_or_default();
    let recipe = loaded
        .recipes
        .into_iter()
        .find(|r| r.id == id)
        .ok_or_else(|| format!("Recipe {id:?} no longer exists. Reload the list."))?;
    Ok((recipe, ctx))
}

/// The recipes that apply to the terminal's current directory.
#[tauri::command]
pub async fn recipes_list(state: State<'_, PtyState>) -> Result<RecipeList, String> {
    let cwd = state.cwd();
    let repo_root = cwd.as_deref().and_then(git::repo_root);
    let loaded = loader::load(repo_root.as_deref());
    Ok(RecipeList {
        cwd,
        repo_root,
        recipes: loaded.recipes.iter().map(RecipeView::from).collect(),
        errors: loaded.errors,
        global_file: loader::global_file(),
        pending_trust: loaded.pending_trust,
    })
}

/// Trusts the repo's .branchdial.yml, but only if it still has the content
/// (hash) the user just reviewed.
#[tauri::command]
pub async fn repo_trust(state: State<'_, PtyState>, hash: String) -> Result<(), String> {
    let root = state.cwd().as_deref().and_then(git::repo_root).ok_or("Not inside a git repository.")?;
    let text = std::fs::read_to_string(root.join(loader::REPO_FILE)).map_err(|e| e.to_string())?;
    if trust::hash(&text) != hash {
        return Err(format!("{} changed while you were reviewing it. Review it again.", loader::REPO_FILE));
    }
    trust::TrustStore::user().ok_or("No home directory to save trust in.")?.trust(&root, &hash)
}

/// Renders the commands for a preview. Always succeeds unless the recipe is gone;
/// problems are reported in `errors`.
#[tauri::command]
pub async fn recipe_plan(
    state: State<'_, PtyState>,
    id: String,
    values: HashMap<String, serde_json::Value>,
    staged: Option<Vec<String>>,
    stash_first: Option<bool>,
) -> Result<plan::Plan, String> {
    let (recipe, ctx) = find(state.cwd().as_deref(), &id)?;
    let extras = plan::Extras { staged, stash_first: stash_first.unwrap_or(false) };
    Ok(plan::plan_with(&recipe, &values, &ctx, &extras))
}

/// Branch names for autocomplete in branch inputs.
#[tauri::command]
pub async fn git_branches(state: State<'_, PtyState>) -> Result<Vec<String>, String> {
    let root = state.cwd().as_deref().and_then(git::repo_root);
    Ok(root.map(|r| git::branch_names(&r)).unwrap_or_default())
}

/// The user's answers in the dialog, checked again here so the frontend
/// can't skip a confirmation.
#[derive(serde::Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct RunChoices {
    staged: Option<Vec<String>>,
    #[serde(default)]
    stash_first: bool,
    /// "Continue anyway" for switching branches with uncommitted changes.
    #[serde(default)]
    allow_dirty_switch: bool,
    /// Confirmed risky steps / a `confirm: true` recipe.
    #[serde(default)]
    confirmed: bool,
}

/// Re-renders the plan and runs it, but only if it still matches what the
/// user saw in the preview (`expected`). Never runs anything unreviewed.
#[tauri::command]
pub async fn recipe_run(
    app: AppHandle,
    state: State<'_, PtyState>,
    id: String,
    values: HashMap<String, serde_json::Value>,
    expected: Vec<String>,
    choices: Option<RunChoices>,
) -> Result<(), String> {
    let choices = choices.unwrap_or_default();
    let (recipe, ctx) = find(state.cwd().as_deref(), &id)?;
    let extras = plan::Extras { staged: choices.staged, stash_first: choices.stash_first };
    let p = plan::plan_with(&recipe, &values, &ctx, &extras);
    if let Some(e) = p.errors.first() {
        return Err(e.message.clone());
    }
    let commands: Vec<&str> = p.steps.iter().map(|s| s.command.as_str()).collect();
    if commands != expected.iter().map(String::as_str).collect::<Vec<_>>() {
        return Err("The commands changed since the preview (branch, files or recipe changed). Review them again.".into());
    }
    if p.dirty_switch.is_some() && !choices.stash_first && !choices.allow_dirty_switch {
        return Err("You have uncommitted changes. Choose \"Stash first\" or \"Continue anyway\".".into());
    }
    if (recipe.confirm || !p.risks.is_empty()) && !choices.confirmed {
        return Err("This recipe needs confirmation before it runs.".into());
    }
    if let Some((path, contents)) = &p.pathspec_file {
        std::fs::write(path, contents).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    }
    let meta = RunMeta { name: recipe.name.clone(), start_branch: ctx.current_branch.clone() };
    crate::runner::start_in_app(&app, &state, meta, p.steps)
}

/// Runs every built-in recipe's real commands with bash in throwaway repos
/// whose "origin" is a local bare repo.
#[cfg(test)]
mod builtin_tests {
    use super::*;
    use crate::git::testutil::TempRepo;
    use serde_json::json;

    fn bash() -> PathBuf {
        #[cfg(windows)]
        return crate::shell::find_git_bash().unwrap();
        #[cfg(not(windows))]
        return PathBuf::from("/bin/bash");
    }

    /// Runs the recipe like the runner does: stop at the first failure, then only `always` steps.
    fn run(repo: &TempRepo, id: &str, values: &[(&str, serde_json::Value)]) -> Vec<(String, i32)> {
        let ctx = git::context(&repo.work);
        let loaded = loader::load(None);
        let recipe = loaded.recipes.iter().find(|r| r.id == id).unwrap();
        let values = values.iter().map(|(k, v)| (k.to_string(), v.clone())).collect();
        let p = plan::plan(recipe, &values, &ctx);
        assert!(p.errors.is_empty(), "{id}: {:?}", p.errors);
        let mut failed = false;
        let mut out = vec![];
        for s in p.steps {
            if failed && (!s.always || git::operation_in_progress(&repo.work).is_some()) {
                continue;
            }
            let o = std::process::Command::new(bash())
                .args(["--norc", "--noprofile", "-c", &s.command])
                .current_dir(&repo.work)
                .output()
                .unwrap();
            let code = o.status.code().unwrap_or(-1);
            if code != 0 {
                eprintln!("{id}: `{}` -> {code}\n{}", s.command, String::from_utf8_lossy(&o.stderr));
                failed = true;
            }
            out.push((s.command, code));
        }
        out
    }

    fn all_ok(results: &[(String, i32)]) -> bool {
        results.iter().all(|(_, c)| *c == 0)
    }

    fn head(repo: &TempRepo) -> String {
        git::git(&repo.work, &["symbolic-ref", "--short", "HEAD"]).unwrap()
    }

    fn commit_file(repo: &TempRepo, name: &str, body: &str) {
        std::fs::write(repo.work.join(name), body).unwrap();
        repo.git(&["add", "-A"]);
        repo.git(&["commit", "-q", "-m", name]);
    }

    #[test]
    fn bcp() {
        let r = TempRepo::new("bcp");
        std::fs::write(r.work.join("a.txt"), "a").unwrap();
        let res = run(&r, "bcp", &[("branch", json!("feat/a")), ("msg", json!("feat: it's \"done\" $(x)"))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "feat/a");
        let msg = git::git(&r.origin, &["log", "-1", "--format=%s", "feat/a"]).unwrap();
        assert_eq!(msg, "feat: it's \"done\" $(x)");
    }

    #[test]
    fn merge_into_returns_even_when_push_fails() {
        let r = TempRepo::new("merge");
        r.git(&["checkout", "-q", "-b", "dev"]);
        r.git(&["push", "-q", "-u", "origin", "dev"]);
        r.git(&["checkout", "-q", "-b", "feat/m"]);
        commit_file(&r, "m.txt", "m");
        let res = run(&r, "merge-into", &[("target", json!("dev"))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "feat/m");
        let parents = git::git(&r.origin, &["log", "-1", "--format=%p", "dev"]).unwrap();
        assert_eq!(parents.split(' ').count(), 2, "--no-ff should create a merge commit");

        // Make the push fail: the always-step must still bring us back.
        commit_file(&r, "m2.txt", "m2");
        std::fs::write(r.origin.join("hooks").join("pre-receive"), "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(r.origin.join("hooks/pre-receive"), std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let res = run(&r, "merge-into", &[("target", json!("dev")), ("no_ff", json!(false))]);
        assert!(!all_ok(&res), "push should have failed: {res:?}");
        assert_eq!(res.last().unwrap().0, "git checkout 'feat/m'");
        assert_eq!(head(&r), "feat/m");
    }

    #[test]
    fn sync_main_rebase_and_merge() {
        let r = TempRepo::new("sync");
        r.git(&["checkout", "-q", "-b", "feat/s"]);
        commit_file(&r, "s.txt", "s");
        r.git(&["checkout", "-q", "main"]);
        commit_file(&r, "main2.txt", "x");
        r.git(&["push", "-q"]);
        r.git(&["checkout", "-q", "feat/s"]);
        assert!(all_ok(&run(&r, "sync-main", &[])));
        assert!(r.work.join("main2.txt").exists());

        r.git(&["checkout", "-q", "main"]);
        commit_file(&r, "main3.txt", "y");
        r.git(&["push", "-q"]);
        r.git(&["checkout", "-q", "feat/s"]);
        assert!(all_ok(&run(&r, "sync-main", &[("strategy", json!("merge"))])));
        assert!(r.work.join("main3.txt").exists());
    }

    #[test]
    fn stash_switch_pops_only_its_own_stash() {
        let r = TempRepo::new("stash");
        r.git(&["branch", "other"]);
        // An older, unrelated stash that must survive.
        std::fs::write(r.work.join("old.txt"), "old").unwrap();
        r.git(&["stash", "push", "-q", "-u", "-m", "older work"]);

        // Clean tree + pop: nothing stashed, so nothing popped.
        let res = run(&r, "stash-switch", &[("branch", json!("other")), ("pop", json!(true))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "other");
        assert!(!r.work.join("old.txt").exists());
        assert_eq!(git::git(&r.work, &["stash", "list"]).unwrap().lines().count(), 1);

        // Dirty tree + pop: changes come along, older stash still there.
        std::fs::write(r.work.join("wip.txt"), "wip").unwrap();
        let res = run(&r, "stash-switch", &[("branch", json!("main")), ("pop", json!(true))]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(head(&r), "main");
        assert!(r.work.join("wip.txt").exists());
        assert_eq!(git::git(&r.work, &["stash", "list"]).unwrap().lines().count(), 1);

        // Without pop the changes stay stashed.
        let res = run(&r, "stash-switch", &[("branch", json!("other"))]);
        assert!(all_ok(&res), "{res:?}");
        assert!(!r.work.join("wip.txt").exists());
        assert_eq!(git::git(&r.work, &["stash", "list"]).unwrap().lines().count(), 2);
    }

    #[test]
    fn version_bump_updates_files_commits_tags_and_pushes() {
        let r = TempRepo::new("bump");
        let write = |rel: &str, body: &str| {
            let p = r.work.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        write("package.json", "{\n  \"name\": \"demo\",\n  \"version\": \"0.1.0\"\n}\n");
        write("src-tauri/Cargo.toml", "[package]\nname = \"demo\"\nversion = \"0.1.0\"\n");
        write("src-tauri/Cargo.lock", "[[package]]\nname = \"demo\"\nversion = \"0.1.0\"\n");
        write("src-tauri/tauri.conf.json", "{\n  \"version\": \"0.1.0\"\n}\n");
        write("unrelated.txt", "committed");
        r.git(&["add", "-A"]);
        r.git(&["commit", "-q", "-m", "setup"]);
        write("unrelated.txt", "dirty, must not be in the release commit");

        let res = run(&r, "version-bump", &[("bump", json!("minor"))]);
        assert!(all_ok(&res), "{res:?}");

        let show = |args: &[&str]| git::git(&r.work, args).unwrap();
        assert_eq!(show(&["log", "-1", "--format=%s"]), "chore: release v0.2.0");
        let mut changed: Vec<_> = show(&["show", "--name-only", "--format="]).lines().map(String::from).collect();
        changed.sort();
        assert_eq!(changed, ["package.json", "src-tauri/Cargo.lock", "src-tauri/Cargo.toml", "src-tauri/tauri.conf.json"]);
        assert_eq!(show(&["cat-file", "-t", "v0.2.0"]), "tag", "annotated tag");
        assert!(git::git(&r.origin, &["rev-parse", "-q", "--verify", "refs/tags/v0.2.0"]).is_some(), "tag pushed");
        assert_eq!(git::git(&r.origin, &["log", "-1", "--format=%s", "main"]).unwrap(), "chore: release v0.2.0");
        for f in ["package.json", "src-tauri/Cargo.toml", "src-tauri/Cargo.lock", "src-tauri/tauri.conf.json"] {
            let body = std::fs::read_to_string(r.work.join(f)).unwrap();
            assert!(body.contains("0.2.0") && !body.contains("0.1.0"), "{f}: {body}");
        }
        assert_eq!(show(&["status", "--porcelain"]), " M unrelated.txt");
    }

    #[test]
    fn abort_go_back_after_failed_merge() {
        let r = TempRepo::new("abort");
        r.git(&["checkout", "-q", "-b", "dev"]);
        commit_file(&r, "README.md", "dev version\n");
        r.git(&["push", "-q", "-u", "origin", "dev"]);
        r.git(&["checkout", "-q", "main"]);
        r.git(&["checkout", "-q", "-b", "feat/c"]);
        commit_file(&r, "README.md", "feature version\n");

        // Nothing in progress yet: the recipe refuses to plan.
        let ctx = git::context(&r.work);
        let recipe = loader::load(None).recipes.into_iter().find(|x| x.id == "abort-go-back").unwrap();
        assert!(plan::plan(&recipe, &HashMap::new(), &ctx).errors[0].message.contains("Nothing to abort"));

        let res = run(&r, "merge-into", &[("target", json!("dev"))]);
        assert!(!all_ok(&res), "merge should conflict: {res:?}");
        assert_eq!(git::operation_in_progress(&r.work), Some(git::Operation::Merge));
        assert_eq!(head(&r), "dev", "always-step is skipped mid-merge");

        let res = run(&r, "abort-go-back", &[]);
        assert!(all_ok(&res), "{res:?}");
        assert_eq!(git::operation_in_progress(&r.work), None);
        assert_eq!(head(&r), "feat/c");
    }

    /// Resolve → Continue: finish the merge with the runner's continue command,
    /// then the rest of Merge into… (push, go back) completes.
    #[test]
    fn resolve_and_continue_after_conflict() {
        let r = TempRepo::new("continue");
        r.git(&["checkout", "-q", "-b", "dev"]);
        commit_file(&r, "README.md", "dev version\n");
        r.git(&["push", "-q", "-u", "origin", "dev"]);
        r.git(&["checkout", "-q", "main"]);
        r.git(&["checkout", "-q", "-b", "feat/r"]);
        commit_file(&r, "README.md", "feature version\n");

        let ctx = git::context(&r.work);
        let recipe = loader::load(None).recipes.into_iter().find(|x| x.id == "merge-into").unwrap();
        let steps = plan::plan(&recipe, &HashMap::from([("target".to_string(), json!("dev"))]), &ctx).steps;
        let res = run(&r, "merge-into", &[("target", json!("dev"))]);
        let failed = res.iter().position(|(_, c)| *c != 0).unwrap();
        assert_eq!(git::operation_in_progress(&r.work), Some(git::Operation::Merge));
        assert_eq!(git::conflicted_files(&r.work), vec!["README.md"]);

        // The user resolves and stages.
        std::fs::write(r.work.join("README.md"), "both\n").unwrap();
        r.git(&["add", "README.md"]);
        assert!(git::conflicted_files(&r.work).is_empty());

        let mut cont = vec![crate::runner::continue_command(git::Operation::Merge).to_string()];
        cont.extend(steps[failed + 1..].iter().map(|s| s.command.clone()));
        for c in &cont {
            let ok = std::process::Command::new(bash()).args(["--norc", "-c", c]).current_dir(&r.work).status().unwrap();
            assert!(ok.success(), "{c}");
        }
        assert_eq!(git::operation_in_progress(&r.work), None);
        assert_eq!(head(&r), "feat/r");
        assert_eq!(git::git(&r.origin, &["show", "dev:README.md"]).unwrap(), "both");
    }

    #[test]
    fn undo_commit_keeps_changes_staged() {
        let r = TempRepo::new("undo");
        commit_file(&r, "u.txt", "u");
        assert!(all_ok(&run(&r, "undo-commit", &[])));
        assert_eq!(git::git(&r.work, &["log", "-1", "--format=%s"]).unwrap(), "init");
        assert_eq!(git::git(&r.work, &["diff", "--cached", "--name-only"]).unwrap(), "u.txt");
    }

    #[test]
    fn clean_merged_keeps_protected_and_unmerged() {
        let r = TempRepo::new("clean");
        for b in ["merged-1", "dev", "release/1.0"] {
            r.git(&["branch", b]);
        }
        r.git(&["checkout", "-q", "-b", "unmerged"]);
        commit_file(&r, "x.txt", "x");
        r.git(&["checkout", "-q", "main"]);
        let res = run(&r, "clean-merged", &[]);
        assert!(all_ok(&res), "{res:?}");
        let branches = git::git(&r.work, &["branch", "--format=%(refname:short)"]).unwrap();
        let mut names: Vec<_> = branches.lines().collect();
        names.sort();
        assert_eq!(names, vec!["dev", "main", "release/1.0", "unmerged"]);

        // Nothing left to delete is still a success.
        assert!(all_ok(&run(&r, "clean-merged", &[])));
    }
}
