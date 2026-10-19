//! Recipe YAML schema and its validation into runnable [`Recipe`]s.

use super::template;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Variables every recipe can use without declaring them.
/// `operation` is merge / rebase / cherry-pick / revert, or empty.
pub const BUILTIN_VARS: [&str; 4] = ["current_branch", "default_branch", "repo_root", "operation"];
/// Extra variables available to recipes with a `version:` block.
pub const VERSION_VARS: [&str; 3] = ["version", "new_version", "version_files"];

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecipeDef {
    pub id: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub confirm: bool,
    /// Hides a recipe with the same id from an earlier source.
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub inputs: Vec<InputDef>,
    /// Conditions that must hold before the recipe can run.
    #[serde(default)]
    pub requires: Vec<RequireDef>,
    pub version: Option<VersionDef>,
    #[serde(default)]
    pub steps: Vec<StepDef>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequireDef {
    pub when: String,
    pub message: String,
}

/// Turns on version bumping: `{{version}}`, `{{new_version}}`, `{{version_files}}`
/// and the `update_version_files: true` step.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VersionDef {
    /// Id of the input holding patch / minor / major (or an explicit X.Y.Z).
    pub bump: String,
    /// Manifests to update, relative to the repo root; missing ones are skipped.
    #[serde(default = "default_version_files")]
    pub files: Vec<String>,
}

fn default_version_files() -> Vec<String> {
    super::version::DEFAULT_FILES.iter().map(|s| s.to_string()).collect()
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InputDef {
    pub id: String,
    pub label: Option<String>,
    pub placeholder: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default, rename = "type")]
    pub kind: InputKind,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    pub default: Option<serde_json::Value>,
    /// Branch inputs only: true = must exist (locally or on origin), false = must not exist locally.
    pub exists: Option<bool>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum InputKind {
    #[default]
    Text,
    Branch,
    Select,
    Bool,
}

/// What a step does.
#[derive(Debug, Clone, PartialEq)]
pub enum Action {
    /// A command template.
    Run(String),
    /// Expands to one visible command per version file (needs a `version:` block).
    UpdateVersionFiles,
}

/// A step is a bare command string, `{ run, when, always }`, or
/// `{ update_version_files: true, when, always }`.
#[derive(Debug, Clone)]
pub struct StepDef {
    pub action: Action,
    pub when: Option<String>,
    pub always: bool,
}

impl<'de> Deserialize<'de> for StepDef {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            run: Option<String>,
            #[serde(default)]
            update_version_files: bool,
            when: Option<String>,
            #[serde(default)]
            always: bool,
        }
        let v = serde_yaml::Value::deserialize(d)?;
        let m = match v {
            serde_yaml::Value::String(run) => return Ok(StepDef { action: Action::Run(run), when: None, always: false }),
            serde_yaml::Value::Mapping(ref m) => m,
            _ => return Err(D::Error::custom("a step must be a command string or { run, when, always }")),
        };
        // `- git commit -m "fix: x"` parses as a one-key map; say so instead of "unknown field".
        if !m.contains_key("run") && !m.contains_key("update_version_files") {
            return Err(D::Error::custom(format!(
                "step {:?} was read as `key: value` because it contains \": \". \
                 Wrap the whole command in single quotes",
                m.keys().next().and_then(|k| k.as_str()).unwrap_or("?")
            )));
        }
        let f: Full = serde_yaml::from_value(v).map_err(D::Error::custom)?;
        let action = match (f.run, f.update_version_files) {
            (Some(run), false) => Action::Run(run),
            (None, true) => Action::UpdateVersionFiles,
            _ => return Err(D::Error::custom("a step needs either `run` or `update_version_files: true`, not both")),
        };
        Ok(StepDef { action, when: f.when, always: f.always })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Builtin,
    Global,
    Repo,
}

/// `when:` conditions: `flag`, `!flag`, `name == value`, `name != value`.
#[derive(Debug, Clone, PartialEq)]
pub enum Cond {
    Truthy(String),
    Falsy(String),
    Eq(String, String),
    Ne(String, String),
}

impl Cond {
    pub fn parse(s: &str) -> Result<Cond, String> {
        let s = s.trim();
        let unquote = |v: &str| {
            let v = v.trim();
            for q in ['"', '\''] {
                if let Some(inner) = v.strip_prefix(q).and_then(|x| x.strip_suffix(q)) {
                    return inner.to_string();
                }
            }
            v.to_string()
        };
        let cond = if let Some((l, r)) = s.split_once("==") {
            Cond::Eq(l.trim().into(), unquote(r))
        } else if let Some((l, r)) = s.split_once("!=") {
            Cond::Ne(l.trim().into(), unquote(r))
        } else if let Some(name) = s.strip_prefix('!') {
            Cond::Falsy(name.trim().into())
        } else {
            Cond::Truthy(s.into())
        };
        if !is_ident(cond.var()) {
            return Err(format!("invalid when: {s:?} (use `flag`, `!flag`, `name == value` or `name != value`)"));
        }
        Ok(cond)
    }

    pub fn var(&self) -> &str {
        match self {
            Cond::Truthy(v) | Cond::Falsy(v) | Cond::Eq(v, _) | Cond::Ne(v, _) => v,
        }
    }

    /// `value` is the variable's value as text ("true"/"false" for bools).
    pub fn eval(&self, value: &str) -> bool {
        let truthy = !value.is_empty() && value != "false";
        match self {
            Cond::Truthy(_) => truthy,
            Cond::Falsy(_) => !truthy,
            Cond::Eq(_, v) => value == v,
            Cond::Ne(_, v) => value != v,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Step {
    pub action: Action,
    pub when: Option<Cond>,
    pub always: bool,
}

#[derive(Debug, Clone)]
pub struct Requirement {
    pub cond: Cond,
    pub message: String,
}

#[derive(Debug, Clone)]
pub struct Recipe {
    pub id: String,
    pub name: String,
    pub description: Option<String>,
    pub confirm: bool,
    pub inputs: Vec<InputDef>,
    pub requires: Vec<Requirement>,
    pub version: Option<VersionDef>,
    pub steps: Vec<Step>,
    pub source: Source,
}

fn is_ident(s: &str) -> bool {
    s.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub fn slugify(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c.to_ascii_lowercase());
        } else if !out.ends_with('-') && !out.is_empty() {
            out.push('-');
        }
    }
    out.trim_end_matches('-').to_string()
}

impl RecipeDef {
    /// The id used for overriding: explicit, else derived from the name.
    pub fn key(&self) -> Option<String> {
        self.id.clone().or_else(|| self.name.as_deref().map(slugify)).filter(|k| !k.is_empty())
    }

    pub fn validate(self, source: Source) -> Result<Recipe, String> {
        let id = self.key().ok_or("needs a name or an id")?;
        if !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            return Err(format!("id {id:?} may only contain letters, digits, - and _"));
        }
        let name = self.name.ok_or("needs a name")?;
        if self.steps.is_empty() {
            return Err("needs at least one step".into());
        }

        let mut known: HashSet<&str> = BUILTIN_VARS.into_iter().collect();
        for input in &self.inputs {
            if !is_ident(&input.id) {
                return Err(format!("input id {:?} must be letters, digits and _ (not starting with a digit)", input.id));
            }
            if BUILTIN_VARS.contains(&input.id.as_str()) {
                return Err(format!("input id {:?} is reserved for a built-in variable", input.id));
            }
            if !known.insert(&input.id) {
                return Err(format!("input id {:?} is used twice", input.id));
            }
            validate_input(input)?;
        }

        if let Some(v) = &self.version {
            if !self.inputs.iter().any(|i| i.id == v.bump) {
                return Err(format!("version.bump refers to unknown input {:?}", v.bump));
            }
            if v.files.is_empty() {
                return Err("version.files is empty".into());
            }
            known.extend(VERSION_VARS);
        }

        let parse_cond = |w: &str, at: &str| -> Result<Cond, String> {
            let c = Cond::parse(w).map_err(|e| format!("{at}: {e}"))?;
            if !known.contains(c.var()) {
                return Err(format!("{at}: refers to unknown variable {:?}", c.var()));
            }
            Ok(c)
        };

        let requires = self
            .requires
            .iter()
            .map(|r| Ok(Requirement { cond: parse_cond(&r.when, "requires")?, message: r.message.clone() }))
            .collect::<Result<Vec<_>, String>>()?;

        let mut steps = Vec::with_capacity(self.steps.len());
        for (n, s) in self.steps.into_iter().enumerate() {
            let at = format!("step {}", n + 1);
            match &s.action {
                Action::Run(run) => {
                    if run.contains('\n') || run.contains('\r') {
                        return Err(format!("{at}: must be a single line"));
                    }
                    for p in template::placeholders(run).map_err(|e| format!("{at}: {e}"))? {
                        if !known.contains(p.as_str()) {
                            return Err(format!("{at}: unknown variable {{{{{p}}}}}"));
                        }
                    }
                    // List variables may only appear unquoted.
                    template::render(run, |name| {
                        Ok(if name == "version_files" {
                            template::Value::Words(vec![])
                        } else {
                            template::Value::Quoted(String::new())
                        })
                    })
                    .map_err(|e| format!("{at}: {e}"))?;
                }
                Action::UpdateVersionFiles if self.version.is_none() => {
                    return Err(format!("{at}: update_version_files needs a `version:` block on the recipe"));
                }
                Action::UpdateVersionFiles => {}
            }
            let when = s.when.as_deref().map(|w| parse_cond(w, &at)).transpose()?;
            steps.push(Step { action: s.action, when, always: s.always });
        }

        Ok(Recipe {
            id,
            name,
            description: self.description,
            confirm: self.confirm,
            inputs: self.inputs,
            requires,
            version: self.version,
            steps,
            source,
        })
    }
}

fn validate_input(input: &InputDef) -> Result<(), String> {
    let id = &input.id;
    match (input.kind, &input.default) {
        (InputKind::Select, _) if input.options.is_empty() => {
            return Err(format!("input {id:?}: a select needs options"));
        }
        (InputKind::Select, Some(serde_json::Value::String(d))) if !input.options.contains(d) => {
            return Err(format!("input {id:?}: default {d:?} is not one of the options"));
        }
        (InputKind::Bool, Some(d)) if !d.is_boolean() => {
            return Err(format!("input {id:?}: default must be true or false"));
        }
        (InputKind::Text | InputKind::Branch | InputKind::Select, Some(d)) if !d.is_string() => {
            return Err(format!("input {id:?}: default must be text"));
        }
        _ => {}
    }
    if input.kind != InputKind::Select && !input.options.is_empty() {
        return Err(format!("input {id:?}: options only apply to type: select"));
    }
    if input.kind != InputKind::Branch && input.exists.is_some() {
        return Err(format!("input {id:?}: exists only applies to type: branch"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(yaml: &str) -> Result<Recipe, String> {
        let def: RecipeDef = serde_yaml::from_str(yaml).map_err(|e| e.to_string())?;
        def.validate(Source::Global)
    }

    #[test]
    fn user_example_parses() {
        let r = parse(
            r#"
name: New branch + commit + push
description: Create a branch, commit all changes, push and set upstream
confirm: false
inputs:
  - { id: branch, label: "Branch name", placeholder: "feature/login", required: true }
  - { id: msg, label: "Commit message", required: true }
steps:
  - git checkout -b "{{branch}}"
  - git add -A
  - git commit -m "{{msg}}"
  - git push -u origin "{{branch}}"
"#,
        )
        .unwrap();
        assert_eq!(r.id, "new-branch-commit-push");
        assert_eq!(r.steps.len(), 4);
        assert_eq!(r.inputs[0].kind, InputKind::Text);
    }

    #[test]
    fn step_objects_and_conditions() {
        let r = parse(
            r#"
name: X
inputs: [{ id: pop, type: bool, default: true }]
steps:
  - run: git stash pop
    when: pop
  - run: git checkout "{{current_branch}}"
    always: true
"#,
        )
        .unwrap();
        assert_eq!(r.steps[0].when, Some(Cond::Truthy("pop".into())));
        assert!(r.steps[1].always);
    }

    #[test]
    fn rejects_bad_recipes() {
        assert!(parse("name: X\nsteps: [\"git push {{nope}}\"]").unwrap_err().contains("unknown variable"));
        assert!(parse("name: X\nsteps: []").unwrap_err().contains("at least one step"));
        assert!(parse("name: X\ninputs: [{id: current_branch}]\nsteps: [x]").unwrap_err().contains("reserved"));
        assert!(parse("name: X\ninputs: [{id: s, type: select}]\nsteps: [x]").unwrap_err().contains("options"));
        assert!(parse("name: X\nsteps: [{run: x, when: nope}]").unwrap_err().contains("unknown variable"));
        assert!(parse("name: X\nsteps: [{run: x, wehn: y}]").is_err());
        assert!(parse("name: X\nstep: [x]").is_err());
        let colon = parse("name: X\nsteps:\n  - git commit -m \"fix: y\"").unwrap_err();
        assert!(colon.contains("single quotes"), "{colon}");
    }

    #[test]
    fn conditions() {
        assert!(Cond::parse("no_ff").unwrap().eval("true"));
        assert!(!Cond::parse("no_ff").unwrap().eval("false"));
        assert!(Cond::parse("!no_ff").unwrap().eval("false"));
        assert!(Cond::parse("strategy == rebase").unwrap().eval("rebase"));
        assert!(Cond::parse("strategy != 'merge'").unwrap().eval("rebase"));
        assert!(Cond::parse("a b").is_err());
    }

    #[test]
    fn slugs() {
        assert_eq!(slugify("New branch + commit + push"), "new-branch-commit-push");
        assert_eq!(slugify("  Undo last commit!  "), "undo-last-commit");
    }
}
