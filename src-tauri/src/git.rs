//! Read-only git queries via the git CLI (never libgit2), so the user's
//! config, aliases and safe.directory rules apply.

use serde::Serialize;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The git executable Branchdial uses for its own (read-only) queries.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInstall {
    pub program: PathBuf,
    /// e.g. "2.52.0.windows.1"
    pub version: String,
}

fn command(program: &Path) -> Command {
    let mut cmd = Command::new(program);
    cmd.env("GIT_TERMINAL_PROMPT", "0");
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    cmd
}

fn probe(program: &Path) -> Option<GitInstall> {
    let out = command(program).arg("--version").output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    // On macOS without the Command Line Tools, /usr/bin/git is a stub that fails here.
    let version = text.trim().strip_prefix("git version ")?.to_string();
    out.status.success().then(|| GitInstall { program: program.to_path_buf(), version })
}

/// Finds git once per run: PATH first, then (on Windows) the Git for Windows
/// install folder, for installs that put only Git Bash on PATH.
pub fn install() -> Option<&'static GitInstall> {
    static GIT: OnceLock<Option<GitInstall>> = OnceLock::new();
    GIT.get_or_init(|| {
        if let Some(found) = probe(Path::new("git")) {
            return Some(found);
        }
        #[cfg(windows)]
        {
            let mut candidates: Vec<PathBuf> = Vec::new();
            if let Some(bash) = crate::shell::find_git_bash() {
                // <root>\bin\bash.exe -> <root>\cmd\git.exe
                if let Some(root) = bash.parent().and_then(Path::parent) {
                    candidates.push(root.join("cmd").join("git.exe"));
                }
            }
            for var in ["ProgramW6432", "ProgramFiles", "LOCALAPPDATA"] {
                if let Ok(p) = std::env::var(var) {
                    let base = PathBuf::from(p);
                    candidates.push(base.join("Git").join("cmd").join("git.exe"));
                    candidates.push(base.join("Programs").join("Git").join("cmd").join("git.exe"));
                }
            }
            if let Some(found) = candidates.iter().filter(|c| c.is_file()).find_map(|c| probe(c)) {
                return Some(found);
            }
        }
        None
    })
    .as_ref()
}

/// Runs `git <args>` in `cwd`; returns trimmed stdout on exit code 0.
/// None if git is missing or the command fails.
pub fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let out = git_raw(cwd, args)?;
    Some(String::from_utf8_lossy(&out).trim_end_matches(['\r', '\n']).to_string())
}

/// Like [`git`] but returns raw stdout (for `-z` output).
pub fn git_raw(cwd: &Path, args: &[&str]) -> Option<Vec<u8>> {
    let program = &install()?.program;
    let out = command(program).args(hardening(program, cwd, args)).args(args).current_dir(cwd).output().ok()?;
    out.status.success().then_some(out.stdout)
}

/// `-c` overrides for Branchdial's own background queries, which run whenever the
/// terminal enters a folder, including folders the user hasn't vetted (a project
/// downloaded with its `.git`). A repo's config can name programs that git runs
/// during `status`: an fsmonitor hook and clean/process filters. Those are turned
/// off here; commands the user runs in the terminal are unaffected. Filters from
/// the user's own (global/system) config, like Git LFS, keep working.
fn hardening(program: &Path, cwd: &Path, args: &[&str]) -> Vec<String> {
    let mut out: Vec<String> = ["-c", "core.fsmonitor=false", "-c", "core.hooksPath=/dev/null"].map(String::from).into();
    // Only commands that read file contents through the index apply filters.
    if !args.iter().any(|a| matches!(*a, "status" | "diff")) {
        return out;
    }
    for name in repo_filter_names(program, cwd) {
        for key in ["clean", "smudge", "process"] {
            out.extend(["-c".to_string(), format!("filter.{name}.{key}=")]);
        }
        out.extend(["-c".to_string(), format!("filter.{name}.required=false")]);
    }
    out
}

/// Filter drivers defined by the repo itself (local/worktree config, includes
/// followed). Reading config never runs anything. Without `--show-scope`
/// (git < 2.26) every defined filter is treated as the repo's.
fn repo_filter_names(program: &Path, cwd: &Path) -> Vec<String> {
    // Exit code 1 just means "no matching keys".
    let read = |args: &[&str]| {
        let out = command(program).args(args).current_dir(cwd).output().ok()?;
        matches!(out.status.code(), Some(0 | 1)).then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    };
    let (text, scoped) = match read(&["config", "--includes", "--show-scope", "--get-regexp", r"^filter\."]) {
        Some(t) => (t, true),
        None => (read(&["config", "--includes", "--get-regexp", r"^filter\."]).unwrap_or_default(), false),
    };
    let mut names: Vec<String> = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        if scoped && !matches!(parts.next(), Some("local" | "worktree")) {
            continue;
        }
        // filter.<name>.<key>; the name itself may contain dots.
        let Some(key) = parts.next().and_then(|k| k.strip_prefix("filter.")) else { continue };
        if let Some((name, _)) = key.rsplit_once('.') {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    names
}

/// Install hint for the current OS, shown when git is missing.
pub fn install_hint() -> &'static str {
    if cfg!(windows) {
        "Install Git for Windows from https://git-scm.com/download/win, then restart Branchdial."
    } else if cfg!(target_os = "macos") {
        "Run `xcode-select --install` (or install git with Homebrew), then restart Branchdial."
    } else {
        "Install git with your package manager (e.g. `sudo apt install git`), then restart Branchdial."
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GitInfo {
    /// None when git can't be found.
    install: Option<GitInstall>,
    install_hint: &'static str,
}

#[tauri::command]
pub async fn git_info() -> Result<GitInfo, String> {
    Ok(GitInfo { install: install().cloned(), install_hint: install_hint() })
}

/// Facts about the repo that recipes can reference as built-in variables.
#[derive(Debug, Clone, Default)]
pub struct RepoContext {
    /// The terminal's directory.
    pub cwd: Option<PathBuf>,
    pub root: Option<PathBuf>,
    /// None on a detached HEAD.
    pub current_branch: Option<String>,
    pub default_branch: Option<String>,
    pub operation: Option<Operation>,
    /// Uncommitted changes, from `git status`.
    pub changes: Vec<ChangedFile>,
    /// Branch patterns that need confirmation before pushing (filled in by the caller).
    pub protected: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangedFile {
    /// Relative to the repo root, forward slashes. Untracked directories end in `/`.
    pub path: String,
    /// Short status, e.g. "M", "A", "D", "R", "??", "U".
    pub code: String,
    pub tracked: bool,
}

/// Uncommitted changes in the repo (staged, unstaged, untracked; ignored files excluded).
pub fn changed_files(root: &Path) -> Vec<ChangedFile> {
    let Some(out) = git_raw(root, &["--no-optional-locks", "status", "--porcelain=v2", "--ignore-submodules=dirty", "-z"]) else { return vec![] };
    let mut files = vec![];
    let mut fields = out.split(|b| *b == 0).map(|f| String::from_utf8_lossy(f).into_owned());
    while let Some(line) = fields.next() {
        let kind = line.chars().next();
        match kind {
            Some('1') | Some('2') | Some('u') => {
                // Path is the last space-separated field (paths may contain spaces, so count fields).
                let n = match kind {
                    Some('1') => 8,
                    Some('2') => 9,
                    _ => 10,
                };
                let path = line.splitn(n + 1, ' ').nth(n).unwrap_or("").to_string();
                let xy: Vec<char> = line.chars().skip(2).take(2).collect();
                let code = if kind == Some('u') {
                    "U".to_string()
                } else {
                    let c = if xy.first() != Some(&'.') { xy[0] } else { *xy.get(1).unwrap_or(&'M') };
                    c.to_string()
                };
                if kind == Some('2') {
                    fields.next(); // original path of a rename
                }
                files.push(ChangedFile { path, code, tracked: true });
            }
            Some('?') => files.push(ChangedFile { path: line[2..].to_string(), code: "??".into(), tracked: false }),
            _ => {}
        }
    }
    files
}

/// Files with unresolved conflicts.
pub fn conflicted_files(root: &Path) -> Vec<String> {
    git(root, &["diff", "--name-only", "--diff-filter=U"])
        .map(|s| s.lines().map(String::from).collect())
        .unwrap_or_default()
}

impl RepoContext {
    /// True if the terminal is somewhere below the repo root rather than at it.
    pub fn cwd_is_not_root(&self) -> bool {
        match (&self.cwd, &self.root) {
            (Some(cwd), Some(root)) => !same_path(cwd, root),
            _ => false,
        }
    }
}

/// Compares paths from different sources. They can name the same folder in
/// different ways: C:/x vs C:\x, a symlink vs its target (macOS /var vs
/// /private/var), or a Windows 8.3 short name (RUNNER~1) vs the long one.
/// So both are resolved to their real location first.
pub fn same_path(a: &Path, b: &Path) -> bool {
    let norm = |p: &Path| {
        let real = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
        let s = real.to_string_lossy().replace('\\', "/");
        // canonicalize on Windows returns the \\?\C:\... form.
        let s = s.strip_prefix("//?/").unwrap_or(&s);
        let s = s.trim_end_matches('/').to_string();
        if cfg!(windows) {
            s.to_lowercase()
        } else {
            s
        }
    };
    norm(a) == norm(b)
}

pub fn repo_root(cwd: &Path) -> Option<PathBuf> {
    git(cwd, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}

pub fn context(cwd: &Path) -> RepoContext {
    let Some(root) = repo_root(cwd) else {
        return RepoContext { cwd: Some(cwd.to_path_buf()), ..Default::default() };
    };
    let current_branch = git(&root, &["symbolic-ref", "--short", "-q", "HEAD"]).filter(|b| !b.is_empty());
    let default_branch = default_branch(&root);
    let operation = operation_in_progress(&root);
    let changes = changed_files(&root);
    RepoContext {
        cwd: Some(cwd.to_path_buf()),
        root: Some(root),
        current_branch,
        default_branch,
        operation,
        changes,
        protected: vec![],
    }
}

pub fn local_branch_exists(root: &Path, name: &str) -> bool {
    git(root, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{name}")]).is_some()
}

pub fn remote_branch_exists(root: &Path, name: &str) -> bool {
    git(root, &["show-ref", "--verify", "--quiet", &format!("refs/remotes/origin/{name}")]).is_some()
}

/// Local branches, then origin's branches that have no local counterpart.
pub fn branch_names(root: &Path) -> Vec<String> {
    branch_list(root).into_iter().map(|b| b.name).collect()
}

/// A branch for the branch switcher.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchEntry {
    pub name: String,
    /// Only on origin so far; `git switch <name>` creates the local tracking branch.
    pub remote_only: bool,
}

/// Local branches, then branches that exist only on origin.
pub fn branch_list(root: &Path) -> Vec<BranchEntry> {
    let mut list: Vec<BranchEntry> = git(root, &["branch", "--format=%(refname:short)"])
        .map(|s| s.lines().map(|n| BranchEntry { name: n.to_string(), remote_only: false }).collect())
        .unwrap_or_default();
    if let Some(remote) = git(root, &["branch", "-r", "--format=%(refname:short)"]) {
        for r in remote.lines() {
            if let Some(n) = r.strip_prefix("origin/") {
                if n != "HEAD" && !list.iter().any(|b| b.name == n) {
                    list.push(BranchEntry { name: n.to_string(), remote_only: true });
                }
            }
        }
    }
    list.retain(|b| !b.name.is_empty() && !b.name.starts_with('('));
    list
}

/// origin/HEAD if the remote advertises it, else the first of main/master that exists locally.
fn default_branch(root: &Path) -> Option<String> {
    if let Some(r) = git(root, &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"]) {
        if let Some(b) = r.strip_prefix("origin/") {
            return Some(b.to_string());
        }
    }
    ["main", "master"]
        .into_iter()
        .find(|b| git(root, &["show-ref", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_some())
        .map(String::from)
}

/// A multi-step git operation that is waiting for the user (conflicts, edits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Operation {
    Merge,
    Rebase,
    CherryPick,
    Revert,
}

impl Operation {
    pub fn as_str(self) -> &'static str {
        match self {
            Operation::Merge => "merge",
            Operation::Rebase => "rebase",
            Operation::CherryPick => "cherry-pick",
            Operation::Revert => "revert",
        }
    }
}

pub fn operation_in_progress(cwd: &Path) -> Option<Operation> {
    let git_dir = PathBuf::from(git(cwd, &["rev-parse", "--absolute-git-dir"])?);
    if git_dir.join("rebase-merge").is_dir() || git_dir.join("rebase-apply").is_dir() {
        Some(Operation::Rebase)
    } else if git_dir.join("MERGE_HEAD").is_file() {
        Some(Operation::Merge)
    } else if git_dir.join("CHERRY_PICK_HEAD").is_file() {
        Some(Operation::CherryPick)
    } else if git_dir.join("REVERT_HEAD").is_file() {
        Some(Operation::Revert)
    } else {
        None
    }
}

#[cfg(test)]
pub mod testutil {
    use std::path::{Path, PathBuf};
    use std::process::Command;

    /// A throwaway repo under the OS temp dir, with a local bare repo as "origin".
    /// Never touches a real remote. Removed on drop.
    pub struct TempRepo {
        pub base: PathBuf,
        pub work: PathBuf,
        pub origin: PathBuf,
    }

    impl TempRepo {
        pub fn new(tag: &str) -> Self {
            let base = std::env::temp_dir().join(format!(
                "branchdial-test-{tag}-{}-{}",
                std::process::id(),
                std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
            ));
            let work = base.join("work");
            let origin = base.join("origin.git");
            std::fs::create_dir_all(&work).unwrap();
            run(&base, &["init", "-q", "--bare", "-b", "main", origin.to_str().unwrap()]);
            run(&work, &["init", "-q", "-b", "main"]);
            run(&work, &["config", "user.email", "test@example.invalid"]);
            run(&work, &["config", "user.name", "Branchdial Test"]);
            run(&work, &["config", "core.autocrlf", "false"]);
            run(&work, &["remote", "add", "origin", origin.to_str().unwrap()]);
            std::fs::write(work.join("README.md"), "hello\n").unwrap();
            run(&work, &["add", "-A"]);
            run(&work, &["commit", "-q", "-m", "init"]);
            run(&work, &["push", "-q", "-u", "origin", "main"]);
            Self { base, work, origin }
        }

        pub fn git(&self, args: &[&str]) {
            run(&self.work, args)
        }
    }

    impl Drop for TempRepo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    pub fn run(cwd: &Path, args: &[&str]) {
        let out = Command::new("git").args(args).current_dir(cwd).output().unwrap();
        assert!(out.status.success(), "git {args:?} failed: {}", String::from_utf8_lossy(&out.stderr));
    }
}

#[cfg(test)]
mod tests {
    use super::testutil::TempRepo;
    use super::*;

    /// The fallback used when only Git Bash (not git.exe) is on PATH.
    #[cfg(windows)]
    #[test]
    fn finds_git_next_to_git_bash() {
        let bash = crate::shell::find_git_bash().unwrap();
        let git_exe = bash.parent().and_then(Path::parent).unwrap().join("cmd").join("git.exe");
        assert!(probe(&git_exe).is_some(), "{}", git_exe.display());
    }

    #[test]
    fn context_in_repo() {
        let r = TempRepo::new("ctx");
        r.git(&["checkout", "-q", "-b", "feature/x"]);
        let ctx = context(&r.work);
        assert_eq!(ctx.current_branch.as_deref(), Some("feature/x"));
        assert_eq!(ctx.default_branch.as_deref(), Some("main"));
        assert!(ctx.root.is_some());
    }

    #[test]
    fn same_path_sees_through_spelling_differences() {
        let r = TempRepo::new("samepath");
        let root = repo_root(&r.work).unwrap();
        // git's spelling vs the temp dir's (symlinked on macOS, 8.3 names on Windows CI).
        assert!(same_path(&r.work, &root), "{} vs {}", r.work.display(), root.display());
        assert!(same_path(&r.work, Path::new(&format!("{}/", r.work.display()))));
        std::fs::create_dir_all(r.work.join("sub")).unwrap();
        assert!(!same_path(&r.work.join("sub"), &root));
        assert!(!context(&r.work).cwd_is_not_root());
        assert!(context(&r.work.join("sub")).cwd_is_not_root());
    }

    /// The macOS case, reproduced anywhere symlinks work.
    #[cfg(unix)]
    #[test]
    fn same_path_through_a_symlink() {
        let r = TempRepo::new("symlink");
        let link = r.base.join("link");
        std::os::unix::fs::symlink(&r.work, &link).unwrap();
        assert!(same_path(&link, &r.work));
        assert!(!context(&link).cwd_is_not_root());
    }

    #[test]
    fn lists_changed_files() {
        let r = TempRepo::new("changes");
        std::fs::write(r.work.join("README.md"), "edited\n").unwrap();
        std::fs::write(r.work.join("new file.txt"), "x").unwrap();
        std::fs::create_dir_all(r.work.join("dist")).unwrap();
        std::fs::write(r.work.join("dist/app.js"), "x").unwrap();
        std::fs::write(r.work.join("staged.txt"), "x").unwrap();
        r.git(&["add", "staged.txt"]);
        let mut files = changed_files(&r.work);
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let got: Vec<_> = files.iter().map(|f| (f.path.as_str(), f.code.as_str(), f.tracked)).collect();
        assert_eq!(
            got,
            vec![("README.md", "M", true), ("dist/", "??", false), ("new file.txt", "??", false), ("staged.txt", "A", true)]
        );
    }

    #[test]
    fn context_outside_repo() {
        let dir = std::env::temp_dir().join(format!("branchdial-norepo-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        // GIT_CEILING_DIRECTORIES isn't needed: temp dirs aren't inside repos on CI or dev machines.
        assert!(context(&dir).root.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn detects_merge_in_progress() {
        let r = TempRepo::new("merge");
        r.git(&["checkout", "-q", "-b", "a"]);
        std::fs::write(r.work.join("README.md"), "from a\n").unwrap();
        r.git(&["commit", "-q", "-am", "a"]);
        r.git(&["checkout", "-q", "main"]);
        std::fs::write(r.work.join("README.md"), "from main\n").unwrap();
        r.git(&["commit", "-q", "-am", "main"]);
        assert_eq!(operation_in_progress(&r.work), None);
        let status = Command::new("git").args(["merge", "a"]).current_dir(&r.work).output().unwrap().status;
        assert!(!status.success());
        assert_eq!(operation_in_progress(&r.work), Some(Operation::Merge));
    }

    /// A downloaded repo's own config names programs git would run during `status`
    /// (fsmonitor hook, clean/process filters, defined directly and via an include).
    /// Branchdial's background queries must not run any of them.
    #[test]
    fn background_queries_never_run_repo_configured_programs() {
        let r = TempRepo::new("trap");
        let marker = r.work.parent().unwrap().join("pwned");
        let m = marker.to_string_lossy().replace('\\', "/");
        let append = |what: &str| format!("echo {what} >> '{m}'");
        r.git(&["config", "core.fsmonitor", &format!("{}; false", append("fsmonitor"))]);
        r.git(&["config", "filter.evil.clean", &format!("{}; cat", append("clean"))]);
        r.git(&["config", "filter.evil.process", &format!("{}; false", append("process"))]);
        // A second driver hidden in an included file.
        let extra = r.work.parent().unwrap().join("extra.cfg");
        std::fs::write(&extra, format!("[filter \"sneaky\"]\n\tclean = {}; cat\n", append("included"))).unwrap();
        r.git(&["config", "include.path", &extra.to_string_lossy().replace('\\', "/")]);
        std::fs::write(r.work.join(".git/info/attributes"), "*.txt filter=evil\n*.md filter=sneaky\n").unwrap();
        // Same size, new mtime: git has to re-read (and filter) the contents.
        std::fs::write(r.work.join("README.md"), "hellO\n").unwrap();
        std::fs::write(r.work.join("a.txt"), "x").unwrap();

        let _ = crate::status::read(&r.work);
        let _ = changed_files(&r.work);
        let _ = context(&r.work);
        let _ = branch_list(&r.work);
        let _ = conflicted_files(&r.work);
        assert!(!marker.exists(), "ran: {}", std::fs::read_to_string(&marker).unwrap_or_default());
        assert_eq!(changed_files(&r.work).len(), 2, "status still works");

        // Control: plain git really does run them, so the assertion above means something.
        let _ = Command::new("git").args(["status", "--porcelain"]).current_dir(&r.work).output();
        assert!(marker.exists(), "the trap should fire without Branchdial's overrides");
    }
}
