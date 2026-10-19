//! Picks which shell to launch (the configured one, else Git Bash on Windows,
//! else `$SHELL`) and, for bash and zsh, wires in Branchdial's shell
//! integration: a prompt hook that reports each command's exit code.

use crate::config::{self, expand_home, Config};
use std::path::{Path, PathBuf};

const BASH_INIT: &str = include_str!("../shell-integration/bash-init.sh");
const ZSH_FILES: [(&str, &str); 4] = [
    (".zshenv", include_str!("../shell-integration/zsh/.zshenv")),
    (".zprofile", include_str!("../shell-integration/zsh/.zprofile")),
    (".zshrc", include_str!("../shell-integration/zsh/.zshrc")),
    (".zlogin", include_str!("../shell-integration/zsh/.zlogin")),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bash,
    Zsh,
    Other,
}

#[derive(Debug, Clone)]
pub struct ShellSpec {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub kind: Kind,
    /// False when the user supplied their own args; we then launch the shell
    /// exactly as configured and recipes are unavailable.
    pub use_integration: bool,
}

pub fn resolve(config: &Config) -> Result<ShellSpec, String> {
    if let Some(sc) = &config.shell {
        let program = expand_home(&sc.path);
        if !program.is_file() {
            return Err(format!(
                "Configured shell not found: {} (check ~/.branchdial/config.yml)",
                program.display()
            ));
        }
        let kind = kind_of(&program);
        return Ok(match &sc.args {
            Some(args) => ShellSpec { program, args: args.clone(), kind, use_integration: false },
            None => ShellSpec { args: default_args(kind, &program), program, kind, use_integration: true },
        });
    }
    let program = detect_default()?;
    let kind = kind_of(&program);
    Ok(ShellSpec { args: default_args(kind, &program), program, kind, use_integration: true })
}

/// The program's name without directory or `.exe`, lowercase. Accepts both
/// `/` and `\` as separators whatever the OS, so a configured Windows path
/// reads the same everywhere.
fn program_name(program: &Path) -> String {
    let s = program.to_string_lossy();
    let file = s.rsplit(['/', '\\']).next().unwrap_or(&s).to_lowercase();
    file.strip_suffix(".exe").map(String::from).unwrap_or(file)
}

fn kind_of(program: &Path) -> Kind {
    match program_name(program).as_str() {
        "bash" => Kind::Bash,
        "zsh" => Kind::Zsh,
        _ => Kind::Other,
    }
}

/// Login + interactive so the user's profile is loaded.
fn default_args(kind: Kind, program: &Path) -> Vec<String> {
    match kind {
        Kind::Bash => vec!["--login".into(), "-i".into()],
        Kind::Zsh => vec!["-l".into(), "-i".into()],
        Kind::Other => {
            let name = program_name(program);
            match name.as_str() {
                "fish" => vec!["-l".into(), "-i".into()],
                "pwsh" | "powershell" => vec!["-NoLogo".into()],
                _ => vec![],
            }
        }
    }
}

/// How to launch a shell with integration: replacement args plus extra env.
pub struct Launch {
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub integration: bool,
}

/// Writes the integration scripts to `~/.branchdial/shell/` and returns the
/// launch parameters. Falls back to a plain launch if the shell is not
/// supported or the files cannot be written.
pub fn prepare_launch(spec: &ShellSpec, nonce: &str) -> Launch {
    let plain = Launch { args: spec.args.clone(), env: vec![], integration: false };
    if !spec.use_integration {
        return plain;
    }
    let Some(dir) = config::app_dir().map(|d| d.join("shell")) else { return plain };
    let nonce_env = ("BRANCHDIAL_NONCE".to_string(), nonce.to_string());
    match spec.kind {
        Kind::Bash => {
            let path = dir.join("bash-init.sh");
            if write_if_changed(&path, BASH_INIT).is_err() {
                return plain;
            }
            // Forward slashes: MSYS bash accepts C:/... and it avoids escaping questions.
            let rc = path.to_string_lossy().replace('\\', "/");
            Launch { args: vec!["--rcfile".into(), rc, "-i".into()], env: vec![nonce_env], integration: true }
        }
        Kind::Zsh => {
            let zdir = dir.join("zsh");
            if ZSH_FILES.iter().any(|(name, body)| write_if_changed(&zdir.join(name), body).is_err()) {
                return plain;
            }
            let user_zdotdir = std::env::var("ZDOTDIR")
                .ok()
                .or_else(|| dirs::home_dir().map(|h| h.to_string_lossy().into_owned()))
                .unwrap_or_default();
            Launch {
                args: spec.args.clone(),
                env: vec![
                    nonce_env,
                    ("ZDOTDIR".into(), zdir.to_string_lossy().into_owned()),
                    ("BRANCHDIAL_USER_ZDOTDIR".into(), user_zdotdir),
                ],
                integration: true,
            }
        }
        Kind::Other => plain,
    }
}

fn write_if_changed(path: &Path, body: &str) -> std::io::Result<()> {
    // Scripts are read by bash/zsh, which choke on CRLF; include_str! keeps the
    // repo's line endings, so normalise here.
    let body = body.replace("\r\n", "\n");
    if std::fs::read_to_string(path).map(|cur| cur == body).unwrap_or(false) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, body)
}

#[cfg(windows)]
fn detect_default() -> Result<PathBuf, String> {
    if let Some(bash) = find_git_bash() {
        return Ok(bash);
    }
    Err("Git Bash was not found. Install Git for Windows (https://git-scm.com/download/win) \
         or set shell.path in ~/.branchdial/config.yml."
        .into())
}

/// Looks in the standard install locations, then next to whatever `git.exe` is on PATH.
#[cfg(windows)]
pub fn find_git_bash() -> Option<PathBuf> {
    let mut roots: Vec<PathBuf> = Vec::new();
    for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Ok(p) = std::env::var(var) {
            roots.push(PathBuf::from(p).join("Git"));
        }
    }
    if let Ok(p) = std::env::var("LOCALAPPDATA") {
        roots.push(PathBuf::from(p).join("Programs").join("Git"));
    }
    // git.exe usually lives in <root>\cmd\ or <root>\mingw64\bin\.
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            if dir.join("git.exe").is_file() {
                for up in dir.ancestors().skip(1).take(2) {
                    roots.push(up.to_path_buf());
                }
            }
        }
    }
    roots
        .into_iter()
        .map(|r| r.join("bin").join("bash.exe"))
        .find(|p| p.is_file())
}

#[cfg(not(windows))]
fn detect_default() -> Result<PathBuf, String> {
    if let Ok(sh) = std::env::var("SHELL") {
        let p = PathBuf::from(sh);
        if p.is_file() {
            return Ok(p);
        }
    }
    ["/bin/zsh", "/bin/bash", "/bin/sh"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .ok_or_else(|| "No shell found. Set shell.path in ~/.branchdial/config.yml.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(p: &str) -> Vec<String> {
        let path = Path::new(p);
        default_args(kind_of(path), path)
    }

    #[test]
    fn default_args_by_name() {
        assert_eq!(args("/bin/bash"), vec!["--login", "-i"]);
        assert_eq!(args(r"C:\Git\bin\bash.exe"), vec!["--login", "-i"]);
        assert_eq!(args("/bin/zsh"), vec!["-l", "-i"]);
        assert!(args("/bin/sh").is_empty());
    }

    #[test]
    fn kinds() {
        assert_eq!(kind_of(Path::new(r"C:\Git\bin\bash.exe")), Kind::Bash);
        assert_eq!(kind_of(Path::new("/usr/bin/zsh")), Kind::Zsh);
        assert_eq!(kind_of(Path::new("/usr/bin/fish")), Kind::Other);
    }

    #[cfg(windows)]
    #[test]
    fn finds_git_bash_on_dev_machine() {
        // Git for Windows is a hard requirement for developing Branchdial on Windows.
        assert!(find_git_bash().is_some());
    }
}
