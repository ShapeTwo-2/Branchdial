//! Spots risky commands in a rendered recipe before it runs: pushes to
//! protected branches, force pushes, resets and other history/data-destroying
//! git commands. Also finds branch switches, for the dirty-tree warning.
//!
//! Works on the exact command text that will be typed, and follows
//! `checkout`/`switch` steps so a later bare `git push` is attributed to the
//! right branch. Git aliases can't be seen through; their steps aren't flagged.

use crate::recipes::plan::PlannedStep;
use serde::Serialize;

pub const DEFAULT_PROTECTED: [&str; 6] = ["main", "master", "prod", "production", "staging", "release/*"];

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Risk {
    /// Index into the plan's steps.
    pub step: usize,
    pub message: String,
}

/// `*` matches any run of characters (including `/`); everything else is literal.
pub fn glob_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || name.len() < first.len() + last.len() || !name.ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

pub fn is_protected(name: &str, protected: &[String]) -> bool {
    protected.iter().any(|p| glob_match(p, name))
}

/// Splits a shell command into simple commands (at `;`, `&&`, `||`, `|`, `&`,
/// parentheses), each a list of unquoted words. Good enough for the commands
/// recipes contain; it doesn't expand variables or substitutions.
pub fn simple_commands(cmd: &str) -> Vec<Vec<String>> {
    let mut out = vec![vec![]];
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();
    let flush = |word: &mut String, in_word: &mut bool, out: &mut Vec<Vec<String>>| {
        if *in_word {
            out.last_mut().unwrap().push(std::mem::take(word));
            *in_word = false;
        }
    };
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                for q in chars.by_ref() {
                    if q == '\'' {
                        break;
                    }
                    word.push(q);
                }
            }
            '"' => {
                in_word = true;
                while let Some(q) = chars.next() {
                    match q {
                        '"' => break,
                        '\\' => {
                            if let Some(n) = chars.next() {
                                word.push(n);
                            }
                        }
                        _ => word.push(q),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(n) = chars.next() {
                    word.push(n);
                }
            }
            ';' | '&' | '|' | '(' | ')' | '\n' => {
                flush(&mut word, &mut in_word, &mut out);
                if !out.last().unwrap().is_empty() {
                    out.push(vec![]);
                }
            }
            c if c.is_whitespace() => flush(&mut word, &mut in_word, &mut out),
            c => {
                in_word = true;
                word.push(c);
            }
        }
    }
    flush(&mut word, &mut in_word, &mut out);
    out.retain(|c| !c.is_empty());
    out
}

/// `(subcommand, args)` if this simple command runs git.
fn git_invocation(words: &[String]) -> Option<(&str, &[String])> {
    // Skip leading VAR=value assignments.
    let start = words.iter().position(|w| !(w.contains('=') && !w.starts_with('-') && !w.starts_with('=')))?;
    let prog = words[start].rsplit(['/', '\\']).next().unwrap_or("");
    if prog != "git" && prog != "git.exe" {
        return None;
    }
    let mut i = start + 1;
    // Global options before the subcommand.
    while i < words.len() && words[i].starts_with('-') {
        if matches!(words[i].as_str(), "-C" | "-c" | "--git-dir" | "--work-tree" | "--namespace") {
            i += 1;
        }
        i += 1;
    }
    let sub = words.get(i)?;
    Some((sub.as_str(), &words[i + 1..]))
}

/// Positional (non-option) arguments, stopping at `--`.
fn positionals<'a>(args: &'a [String], takes_value: &[&str]) -> Vec<&'a str> {
    let mut out = vec![];
    let mut skip = false;
    for a in args {
        if skip {
            skip = false;
            continue;
        }
        if a == "--" {
            break;
        }
        // A lone "-" is an argument ("the previous branch"), not an option.
        if a.starts_with('-') && a != "-" {
            skip = takes_value.contains(&a.as_str());
            continue;
        }
        out.push(a.as_str());
    }
    out
}

/// After `git checkout`/`git switch <args>`: the branch it switches to.
/// `Some(None)` = switches, but to something we can't name (`-`, a commit).
/// `None` = not a branch switch (e.g. `git checkout -- file`).
fn switch_target(sub: &str, args: &[String]) -> Option<Option<String>> {
    if sub != "checkout" && sub != "switch" {
        return None;
    }
    // New branch: -b/-B (checkout), -c/-C (switch), --orphan.
    for (i, a) in args.iter().enumerate() {
        if matches!(a.as_str(), "-b" | "-B" | "-c" | "-C" | "--create" | "--force-create" | "--orphan") {
            return Some(args.get(i + 1).cloned());
        }
    }
    if sub == "checkout" && args.iter().any(|a| a == "--") {
        return None; // restoring files
    }
    let pos = positionals(args, &[]);
    match pos.as_slice() {
        [] => None,
        [b] if *b == "-" => Some(None),
        [b] => Some(Some(b.to_string())),
        _ if sub == "checkout" => None, // checkout <tree-ish> <paths>
        [b, ..] => Some(Some(b.to_string())),
    }
}

/// Does this step switch to another existing branch (where uncommitted changes matter)?
/// Creating a branch (`-b`) carries changes along on purpose, so it doesn't count.
fn is_existing_branch_switch(sub: &str, args: &[String]) -> bool {
    let creates = args.iter().any(|a| matches!(a.as_str(), "-b" | "-B" | "-c" | "-C" | "--create" | "--force-create" | "--orphan"));
    !creates && switch_target(sub, args).is_some()
}

/// Index of the first step that switches branches without an earlier `git stash`.
pub fn first_unstashed_switch(steps: &[PlannedStep]) -> Option<usize> {
    for (i, step) in steps.iter().enumerate() {
        for words in simple_commands(&step.command) {
            if let Some((sub, args)) = git_invocation(&words) {
                if sub == "stash" {
                    return None; // the recipe takes care of it
                }
                if is_existing_branch_switch(sub, args) {
                    return Some(i);
                }
            }
        }
    }
    None
}

pub fn analyze(steps: &[PlannedStep], start_branch: Option<&str>, protected: &[String]) -> Vec<Risk> {
    let mut risks = vec![];
    let mut current = start_branch.map(String::from);
    for (i, step) in steps.iter().enumerate() {
        let mut add = |message: String| {
            if !risks.iter().any(|r: &Risk| r.step == i && r.message == message) {
                risks.push(Risk { step: i, message });
            }
        };
        for words in simple_commands(&step.command) {
            let Some((sub, args)) = git_invocation(&words) else { continue };
            if let Some(target) = switch_target(sub, args) {
                current = target;
                continue;
            }
            match sub {
                "push" => push_risks(args, current.as_deref(), protected, &mut add),
                "reset" => {
                    let mode = args.iter().find(|a| matches!(a.as_str(), "--hard" | "--soft" | "--mixed" | "--merge" | "--keep"));
                    let unstage_only = args.is_empty() || args.iter().any(|a| a == "--");
                    if mode.map(String::as_str) == Some("--hard") {
                        add("Discards all uncommitted changes (reset --hard)".into());
                    } else if mode.is_some() || !unstage_only {
                        add("Moves the branch back to another commit (reset)".into());
                    }
                }
                "branch" => {
                    let force_delete = args.iter().any(|a| a == "-D")
                        || (args.iter().any(|a| a == "-d" || a == "--delete") && args.iter().any(|a| a == "-f" || a == "--force"));
                    if force_delete {
                        add("Force-deletes a branch; commits only on it can be lost (branch -D)".into());
                    }
                }
                "clean" if args.iter().any(|a| a.starts_with('-') && !a.starts_with("--") && a.contains('f') || a == "--force") => {
                    add("Permanently deletes untracked files (clean)".into());
                }
                _ => {}
            }
        }
    }
    risks
}

fn push_risks(args: &[String], current: Option<&str>, protected: &[String], add: &mut impl FnMut(String)) {
    let mut force = false;
    let mut delete = false;
    let mut all = false;
    for a in args {
        match a.as_str() {
            "-f" | "--force" => force = true,
            s if s.starts_with("--force-with-lease") => force = true,
            "-d" | "--delete" => delete = true,
            "--all" | "--branches" => all = true,
            "--mirror" => {
                add("Mirror push makes the remote an exact copy, deleting anything else there".into());
            }
            _ => {}
        }
    }
    let pos = positionals(args, &["-o", "--push-option", "--repo", "--receive-pack", "--exec"]);
    let refspecs = pos.get(1..).unwrap_or(&[]);

    let mut targets: Vec<String> = vec![];
    for spec in refspecs {
        let spec = match spec.strip_prefix('+') {
            Some(s) => {
                force = true;
                s
            }
            None => spec,
        };
        let (src, dst) = spec.split_once(':').unwrap_or((spec, ""));
        if spec.starts_with(':') {
            delete = true;
        }
        let name = if !dst.is_empty() { dst } else { src };
        let name = if name == "HEAD" || name == "@" { current.unwrap_or("") } else { name };
        let name = name.strip_prefix("refs/heads/").unwrap_or(name);
        if !name.is_empty() && !name.starts_with("refs/tags/") {
            targets.push(name.to_string());
        }
    }
    if refspecs.is_empty() && !all {
        // Bare `git push` / `git push origin`: the current branch.
        if let Some(c) = current {
            targets.push(c.to_string());
        }
    }

    if force {
        add("Force-pushes, which can overwrite commits on the remote".into());
    }
    if all {
        add("Pushes all local branches, including protected ones".into());
    }
    for t in targets {
        if delete {
            add(format!("Deletes branch {t} on the remote"));
        } else if is_protected(&t, protected) {
            add(format!("Pushes to protected branch {t}"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(cmds: &[&str]) -> Vec<PlannedStep> {
        cmds.iter().map(|c| PlannedStep { command: c.to_string(), always: false }).collect()
    }

    fn defaults() -> Vec<String> {
        DEFAULT_PROTECTED.iter().map(|s| s.to_string()).collect()
    }

    fn messages(cmds: &[&str], start: &str) -> Vec<String> {
        analyze(&steps(cmds), Some(start), &defaults()).into_iter().map(|r| r.message).collect()
    }

    #[test]
    fn globs() {
        assert!(glob_match("release/*", "release/1.0"));
        assert!(glob_match("release/*", "release/a/b"));
        assert!(!glob_match("release/*", "releases/1"));
        assert!(glob_match("*-hotfix", "x-hotfix"));
        assert!(glob_match("main", "main") && !glob_match("main", "mainline"));
        assert!(glob_match("a*b*c", "a-b-c") && !glob_match("a*b*c", "a-c"));
    }

    #[test]
    fn splits_commands_and_quotes() {
        assert_eq!(
            simple_commands(r#"git push -u origin 'feat x' && echo "a;b" | wc"#),
            vec![
                vec!["git", "push", "-u", "origin", "feat x"],
                vec!["echo", "a;b"],
                vec!["wc"]
            ]
        );
        assert_eq!(simple_commands(r#"git commit -m "fix: "'it'\''s'"#), vec![vec!["git", "commit", "-m", "fix: it's"]]);
    }

    #[test]
    fn push_to_protected_follows_checkouts() {
        // Merge into prod: the bare `git push` happens on prod.
        let m = messages(
            &["git checkout 'prod'", "git pull --ff-only", "git merge --no-ff 'feat/a'", "git push", "git checkout 'feat/a'"],
            "feat/a",
        );
        assert_eq!(m, vec!["Pushes to protected branch prod"]);
        // ...but not on dev.
        assert!(messages(&["git checkout 'dev'", "git push"], "feat/a").is_empty());
        // bcp from main pushes the new branch, which is fine.
        assert!(messages(&["git checkout -b 'feat/b'", "git push -u origin 'feat/b'"], "main").is_empty());
        // A bare push while on main, explicit refspecs, release globs.
        assert_eq!(messages(&["git push --follow-tags"], "main"), vec!["Pushes to protected branch main"]);
        assert_eq!(messages(&["git push origin HEAD:release/2.0"], "x"), vec!["Pushes to protected branch release/2.0"]);
        assert!(messages(&["git push origin refs/tags/v1.0.0"], "x").is_empty());
    }

    #[test]
    fn force_delete_and_all() {
        assert_eq!(messages(&["git push -f"], "feat"), vec!["Force-pushes, which can overwrite commits on the remote"]);
        assert_eq!(messages(&["git push --force-with-lease origin feat"], "x").len(), 1);
        assert_eq!(messages(&["git push origin +feat"], "x").len(), 1);
        assert_eq!(messages(&["git push origin --delete old"], "x"), vec!["Deletes branch old on the remote"]);
        assert_eq!(messages(&["git push origin :old"], "x"), vec!["Deletes branch old on the remote"]);
        assert_eq!(messages(&["git push --all"], "x").len(), 1);
    }

    #[test]
    fn resets_and_other_destructive_commands() {
        assert_eq!(messages(&["git reset --soft HEAD~1"], "x"), vec!["Moves the branch back to another commit (reset)"]);
        assert_eq!(messages(&["git reset --hard origin/main"], "x"), vec!["Discards all uncommitted changes (reset --hard)"]);
        assert_eq!(messages(&["git reset HEAD~2"], "x").len(), 1);
        assert!(messages(&["git reset"], "x").is_empty(), "plain reset only unstages");
        assert!(messages(&["git reset -- file.txt"], "x").is_empty());
        assert_eq!(messages(&["git branch -D old"], "x").len(), 1);
        assert!(messages(&["git branch -d old"], "x").is_empty());
        assert_eq!(messages(&["git clean -fd"], "x").len(), 1);
        assert!(messages(&["git clean -n"], "x").is_empty());
        // Inside a pipeline, with global options and env vars.
        assert_eq!(messages(&["GIT_TRACE=1 git -C repo reset --hard"], "x").len(), 1);
        assert_eq!(messages(&["echo x | xargs -r git branch -D"], "x").len(), 0, "xargs hides the git call");
    }

    #[test]
    fn protected_list_is_configurable() {
        let custom = vec!["trunk".to_string()];
        assert!(analyze(&steps(&["git push"]), Some("main"), &custom).is_empty());
        assert_eq!(analyze(&steps(&["git push"]), Some("trunk"), &custom).len(), 1);
    }

    #[test]
    fn finds_switches_without_stash() {
        assert_eq!(first_unstashed_switch(&steps(&["git fetch", "git checkout 'dev'"])), Some(1));
        assert_eq!(first_unstashed_switch(&steps(&["git switch main"])), Some(0));
        assert_eq!(first_unstashed_switch(&steps(&["git stash push -u", "git switch main"])), None);
        assert_eq!(first_unstashed_switch(&steps(&["git checkout -b new", "git add -A"])), None);
        assert_eq!(first_unstashed_switch(&steps(&["git checkout -- file.txt"])), None);
        assert_eq!(first_unstashed_switch(&steps(&["git checkout -"])), Some(0));
    }
}
