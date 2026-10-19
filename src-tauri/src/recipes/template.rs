//! Renders `{{placeholder}}`s in a recipe step into a POSIX shell command.
//!
//! Values are always inserted as a single-quoted shell word, so nothing a user
//! types can be interpreted by the shell. A small lexer tracks the quoting
//! context of each placeholder:
//!
//! - unquoted:            `git switch {{b}}`        -> `git switch 'b'`
//! - a whole "..." word:  `git switch "{{b}}"`      -> `git switch 'b'`
//! - inside "...":        `-m "WIP: {{m}}"`         -> `-m "WIP: "'m'`
//! - inside '...':        rejected, since single quotes can't contain an escaped value.

/// Quotes `s` as one POSIX shell word.
pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[derive(Clone, Copy, PartialEq)]
enum State {
    Plain,
    Single,
    Double,
}

/// What to insert for a placeholder.
pub enum Value {
    /// Shell-quoted, safe.
    Quoted(String),
    /// Several shell words, each quoted (e.g. a list of files). Only allowed
    /// unquoted in the template.
    Words(Vec<String>),
    /// Inserted verbatim. Only for previews of missing values, e.g. `<branch>`;
    /// never sent to the shell.
    Display(String),
}

/// Parses `{{ ident }}` at the start of `s`; returns (ident, byte length).
fn placeholder_at(s: &str) -> Option<(&str, usize)> {
    let inner = s.strip_prefix("{{")?;
    let close = inner.find("}}")?;
    let name = inner[..close].trim();
    let valid = name.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    valid.then_some((name, 2 + close + 2))
}

/// Renders `template`, asking `lookup` for each placeholder's value.
pub fn render(
    template: &str,
    mut lookup: impl FnMut(&str) -> Result<Value, String>,
) -> Result<String, String> {
    let mut out = String::with_capacity(template.len() + 16);
    let mut state = State::Plain;
    // Byte index in `out` of the `"` that opened the current double-quoted run.
    let mut dq_open = 0;
    let mut i = 0;
    while i < template.len() {
        let rest = &template[i..];
        let c = rest.chars().next().unwrap();

        if state != State::Single {
            if let Some((name, len)) = placeholder_at(rest) {
                let text = match lookup(name)? {
                    Value::Quoted(v) => sh_quote(&v),
                    Value::Words(_) if state == State::Double => {
                        return Err(format!("{{{{{name}}}}} is a list of words; use it outside quotes"));
                    }
                    Value::Words(ws) => ws.iter().map(|w| sh_quote(w)).collect::<Vec<_>>().join(" "),
                    Value::Display(v) => v,
                };
                i += len;
                if state == State::Plain {
                    out.push_str(&text);
                    continue;
                }
                // Inside "...": close the double quotes, insert the single-quoted
                // word, reopen. Drop the empty "" pairs that would leave behind.
                if out.len() == dq_open + 1 {
                    out.truncate(dq_open);
                } else {
                    out.push('"');
                }
                out.push_str(&text);
                if template[i..].starts_with('"') {
                    i += 1;
                    state = State::Plain;
                } else {
                    dq_open = out.len();
                    out.push('"');
                }
                continue;
            }
        }

        match (state, c) {
            (State::Plain, '\'') => state = State::Single,
            (State::Single, '\'') => state = State::Plain,
            (State::Plain, '"') => {
                state = State::Double;
                dq_open = out.len();
            }
            (State::Double, '"') => state = State::Plain,
            (State::Plain | State::Double, '\\') => {
                // Copy the escaped character as-is.
                out.push(c);
                i += 1;
                if let Some(next) = template[i..].chars().next() {
                    out.push(next);
                    i += next.len_utf8();
                }
                continue;
            }
            (State::Single, '{') if placeholder_at(rest).is_some() => {
                return Err(format!(
                    "{} is inside single quotes, where it can't be inserted safely. Use double quotes or none.",
                    &rest[..placeholder_at(rest).unwrap().1]
                ));
            }
            _ => {}
        }
        out.push(c);
        i += c.len_utf8();
    }
    match state {
        State::Plain => Ok(out),
        State::Single => Err("Unterminated single quote".into()),
        State::Double => Err("Unterminated double quote".into()),
    }
}

/// Every placeholder name used in `template` (validates quoting too).
pub fn placeholders(template: &str) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    render(template, |n| {
        names.push(n.to_string());
        Ok(Value::Quoted(String::new()))
    })?;
    Ok(names)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn r(t: &str, vars: &[(&str, &str)]) -> Result<String, String> {
        let m: HashMap<_, _> = vars.iter().cloned().collect();
        render(t, |n| m.get(n).map(|v| Value::Quoted(v.to_string())).ok_or(format!("unknown {n}")))
    }

    #[test]
    fn whole_double_quoted_word_becomes_single_quoted() {
        assert_eq!(r(r#"git checkout -b "{{branch}}""#, &[("branch", "feat/x")]).unwrap(), "git checkout -b 'feat/x'");
    }

    #[test]
    fn unquoted_and_spaced_placeholder() {
        assert_eq!(r("git switch {{ b }}", &[("b", "x")]).unwrap(), "git switch 'x'");
    }

    #[test]
    fn placeholder_inside_double_quotes() {
        assert_eq!(r(r#"-m "WIP: {{m}}""#, &[("m", "hi")]).unwrap(), r#"-m "WIP: "'hi'"#);
        assert_eq!(r(r#"-m "{{m}} done""#, &[("m", "hi")]).unwrap(), r#"-m 'hi'" done""#);
        assert_eq!(r(r#""a {{m}} b""#, &[("m", "x")]).unwrap(), r#""a "'x'" b""#);
        assert_eq!(r(r#""{{a}}{{b}}""#, &[("a", "1"), ("b", "2")]).unwrap(), "'1''2'");
    }

    #[test]
    fn hostile_values_stay_literal() {
        let evil = r#"x"; rm -rf ~; echo "$(id)`id`'$HOME\"#;
        let out = r(r#"git commit -m "{{m}}""#, &[("m", evil)]).unwrap();
        assert_eq!(out, r#"git commit -m 'x"; rm -rf ~; echo "$(id)`id`'\''$HOME\'"#);
    }

    #[test]
    fn rejects_placeholder_in_single_quotes() {
        assert!(r("echo '{{m}}'", &[("m", "x")]).is_err());
        // Braces that aren't a placeholder are fine there.
        assert_eq!(r("awk '{print $1}'", &[]).unwrap(), "awk '{print $1}'");
    }

    #[test]
    fn escapes_and_quotes_are_tracked() {
        assert_eq!(r(r#"echo \"{{m}}"#, &[("m", "x")]).unwrap(), r#"echo \"'x'"#);
        assert_eq!(r(r#"echo "a\"{{m}}""#, &[("m", "x")]).unwrap(), r#"echo "a\""'x'"#);
        assert!(r(r#"echo "open"#, &[]).is_err());
    }

    #[test]
    fn word_lists() {
        let words = |n: &str| match n {
            "files" => Ok(Value::Words(vec!["a b".into(), "c".into()])),
            _ => Ok(Value::Quoted("x".into())),
        };
        assert_eq!(render("git add -- {{files}}", words).unwrap(), "git add -- 'a b' 'c'");
        assert!(render(r#"echo "{{files}}""#, words).is_err());
    }

    #[test]
    fn lists_placeholders() {
        assert_eq!(placeholders(r#"git push -u origin "{{branch}}" {{x}}"#).unwrap(), vec!["branch", "x"]);
    }

    #[test]
    fn sh_quote_basics() {
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
    }

    /// The rendered command really is safe: bash echoes the value back unchanged.
    #[test]
    fn bash_round_trip() {
        let Some(bash) = test_bash() else { return };
        let evil = r#"a"b'c $(echo PWNED) `echo PWNED` ; echo PWNED \ ${HOME} *"#;
        let cmd = r(r#"printf %s "{{v}}""#, &[("v", evil)]).unwrap();
        let out = std::process::Command::new(bash).args(["--norc", "-c", &cmd]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), evil);
    }

    fn test_bash() -> Option<std::path::PathBuf> {
        #[cfg(windows)]
        return crate::shell::find_git_bash();
        #[cfg(not(windows))]
        return Some("/bin/bash".into());
    }
}
