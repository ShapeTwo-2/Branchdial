//! The repo status shown at the top of the sidebar: branch, ahead/behind,
//! changed files and any merge/rebase in progress.
//!
//! Computed with one `git status --porcelain=v2 --branch -z` call. Refreshes
//! after every shell prompt and on demand; requests that arrive while one is
//! running are merged into a single follow-up, so slow repos don't pile up.

use crate::git::{self, Operation};
use crate::pty::PtyState;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, State};

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RepoStatus {
    /// The terminal's directory this status is for.
    pub cwd: Option<String>,
    /// None when the cwd is not inside a repository.
    pub root: Option<String>,
    /// None on a detached HEAD.
    pub branch: Option<String>,
    /// Short commit id, for a detached HEAD.
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub modified: u32,
    pub untracked: u32,
    pub conflicted: u32,
    pub operation: Option<Operation>,
}

/// Parses `git status --porcelain=v2 --branch -z` output.
fn parse(out: &[u8], status: &mut RepoStatus) {
    let mut fields = out.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).into_owned());
    while let Some(line) = fields.next() {
        if let Some(h) = line.strip_prefix("# branch.") {
            let (key, value) = h.split_once(' ').unwrap_or((h, ""));
            match key {
                "oid" if value != "(initial)" => status.head = Some(value.chars().take(7).collect()),
                "head" if value != "(detached)" => status.branch = Some(value.to_string()),
                "upstream" => status.upstream = Some(value.to_string()),
                "ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            status.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            status.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let kind = line.chars().next();
        match kind {
            Some('1') | Some('2') => {
                let xy: Vec<char> = line.chars().skip(2).take(2).collect();
                if xy.first().is_some_and(|c| *c != '.') {
                    status.staged += 1;
                }
                if xy.get(1).is_some_and(|c| *c != '.') {
                    status.modified += 1;
                }
                if kind == Some('2') {
                    fields.next(); // a rename/copy is followed by its original path
                }
            }
            Some('u') => status.conflicted += 1,
            Some('?') => status.untracked += 1,
            _ => {}
        }
    }
}

pub fn read(cwd: &Path) -> RepoStatus {
    let mut status = RepoStatus { cwd: Some(cwd.display().to_string()), ..Default::default() };
    let Some(root) = git::repo_root(cwd) else { return status };
    // --no-optional-locks: never take index.lock in the background, so the
    // user's own git commands can't collide with a status refresh.
    if let Some(out) = git::git_raw(&root, &["--no-optional-locks", "status", "--porcelain=v2", "--branch", "-z"]) {
        parse(&out, &mut status);
    }
    if status.branch.is_some() {
        status.head = None;
    }
    status.operation = git::operation_in_progress(&root);
    status.root = Some(root.display().to_string());
    status
}

#[derive(Default)]
struct Worker {
    running: bool,
    /// Latest cwd requested while a refresh was running.
    pending: Option<PathBuf>,
}

/// Managed state that coalesces refresh requests.
#[derive(Default, Clone)]
pub struct StatusWatcher {
    worker: Arc<Mutex<Worker>>,
}

impl StatusWatcher {
    pub fn request(&self, app: &AppHandle, cwd: PathBuf) {
        {
            let mut w = self.worker.lock().unwrap();
            if w.running {
                w.pending = Some(cwd);
                return;
            }
            w.running = true;
        }
        let worker = self.worker.clone();
        let app = app.clone();
        std::thread::spawn(move || {
            let mut cwd = cwd;
            loop {
                let status = read(&cwd);
                let _ = app.emit("git://status", status);
                let mut w = worker.lock().unwrap();
                match w.pending.take() {
                    Some(next) => cwd = next,
                    None => {
                        w.running = false;
                        return;
                    }
                }
            }
        });
    }
}

/// Asks for a fresh status (window focus, refresh button). The result arrives as `git://status`.
#[tauri::command]
pub async fn status_refresh(app: AppHandle, pty: State<'_, PtyState>, watcher: State<'_, StatusWatcher>) -> Result<(), String> {
    if let Some(cwd) = pty.cwd() {
        watcher.request(&app, cwd);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git::testutil::TempRepo;

    #[test]
    fn parses_porcelain_v2() {
        let out = b"# branch.oid 1234567890abcdef\0# branch.head feat/x\0# branch.upstream origin/feat/x\0# branch.ab +2 -1\0\
1 .M N... 100644 100644 100644 a b src/a.rs\0\
1 M. N... 100644 100644 100644 a b src/b.rs\0\
1 MM N... 100644 100644 100644 a b src/c.rs\0\
2 R. N... 100644 100644 100644 a b R100 new name.rs\0old name.rs\0\
u UU N... 100644 100644 100644 100644 a b c conflict.rs\0\
? notes.txt\0";
        let mut s = RepoStatus::default();
        parse(out, &mut s);
        assert_eq!(s.branch.as_deref(), Some("feat/x"));
        assert_eq!(s.upstream.as_deref(), Some("origin/feat/x"));
        assert_eq!((s.ahead, s.behind), (2, 1));
        assert_eq!((s.staged, s.modified, s.untracked, s.conflicted), (3, 2, 1, 1));
    }

    #[test]
    fn detached_and_initial() {
        let mut s = RepoStatus::default();
        parse(b"# branch.oid abcdef1234\0# branch.head (detached)\0", &mut s);
        assert_eq!((s.branch.as_deref(), s.head.as_deref()), (None, Some("abcdef1")));
        let mut s = RepoStatus::default();
        parse(b"# branch.oid (initial)\0# branch.head main\0", &mut s);
        assert_eq!((s.branch.as_deref(), s.head.as_deref()), (Some("main"), None));
    }

    #[test]
    fn reads_a_real_repo() {
        let r = TempRepo::new("status");
        let s = read(&r.work);
        assert_eq!(s.branch.as_deref(), Some("main"));
        assert_eq!(s.upstream.as_deref(), Some("origin/main"));
        assert_eq!((s.ahead, s.behind, s.staged, s.modified, s.untracked), (0, 0, 0, 0, 0));

        std::fs::write(r.work.join("README.md"), "changed\n").unwrap();
        std::fs::write(r.work.join("new file.txt"), "x").unwrap();
        r.git(&["commit", "-q", "-am", "local"]);
        std::fs::write(r.work.join("README.md"), "changed again\n").unwrap();
        let s = read(&r.work);
        assert_eq!((s.ahead, s.modified, s.untracked), (1, 1, 1));

        // From a subfolder, and with a merge conflict.
        std::fs::create_dir_all(r.work.join("sub")).unwrap();
        r.git(&["checkout", "-q", "README.md"]);
        r.git(&["checkout", "-q", "-b", "other", "HEAD~1"]);
        std::fs::write(r.work.join("README.md"), "other\n").unwrap();
        r.git(&["commit", "-q", "-am", "other"]);
        let _ = std::process::Command::new("git").args(["merge", "main"]).current_dir(&r.work).output();
        let s = read(&r.work.join("sub"));
        assert_eq!(s.operation, Some(Operation::Merge));
        assert_eq!(s.conflicted, 1);
        assert!(s.root.is_some());
    }

    #[test]
    fn outside_a_repo() {
        let dir = std::env::temp_dir();
        let s = read(&dir);
        assert!(s.root.is_none() && s.branch.is_none());
    }

    #[test]
    fn git_is_found() {
        let g = git::install().expect("git must be installed to run the tests");
        assert!(g.version.chars().next().unwrap().is_ascii_digit(), "{}", g.version);
    }
}
