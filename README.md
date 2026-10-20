# Branchdial

A real terminal with a sidebar of one-click **workflow buttons** for common git tasks.

Developers chain git commands into aliases. Branchdial turns those workflows into buttons, and
every button **types its commands into a visible, real terminal**, so you always see exactly what
runs. It uses the `git` CLI, so your aliases, hooks, credentials and config keep working.

- **Transparent:** a live preview shows the exact commands before you run them, then each one is
  typed into the terminal, one at a time. You can type in the terminal normally too.
- **Safe by default:** form values can never inject shell commands; pushes to protected branches,
  force pushes and resets need confirmation; you pick which files a commit includes; a stopped
  merge gets Resolve / Abort buttons.
- **Yours:** recipes are plain YAML, per user or shared with your team in the repo.

## Requirements

| OS | You need |
|---|---|
| Windows 10/11 | [Git for Windows](https://git-scm.com/download/win) (provides `git` and Git Bash, the default shell). WebView2 is built into Windows 11. |
| macOS | `git`: run `xcode-select --install`, or install it with Homebrew. |
| Linux | `git` from your package manager (e.g. `sudo apt install git`) and WebKitGTK 4.1. |

Recipes run in **bash or zsh**. The *Version bump* recipe also uses `perl` (included with Git
Bash, macOS and most Linux systems) and `npm` when the repo has a `package.json`.

## Install

Download the installer for your platform from the
[Releases](../../releases) page: `.msi` / `.exe` for Windows, `.dmg` for macOS, `.deb` /
`.AppImage` for Linux.

Or build it yourself (see [Development](#development)).

### Updates

Branchdial checks for a new version a few seconds after it starts and asks before installing
it: **Update now** downloads it, verifies its signature, installs it and restarts; **Later** asks
again next launch; **Skip this version** stays quiet until the next one. **Check for updates** at
the bottom of the sidebar checks at any time. Updates wait while a recipe is running. Set
`checkForUpdates: false` in the [config](#configuration) to turn off the startup check.

On Linux only the `.AppImage` updates itself; with the `.deb`, install the new one from Releases.

## Using it

- **Terminal:** a real shell (Git Bash on Windows, your `$SHELL` elsewhere). `cd` into a repo and
  the sidebar follows. Ctrl+C copies when text is selected, otherwise interrupts; Ctrl+V pastes.
- **Status bar:** current branch, commits ahead/behind its upstream, changed files, and a banner
  while a merge or rebase is in progress. It refreshes after every command and when the window
  regains focus.
- **Branch switcher:** click the branch name to search every branch, including ones that so far
  exist only on origin (tagged `origin`). Pick one (or use ↑/↓ and Enter) and `git switch` runs in
  the terminal; an origin-only branch becomes a local branch that tracks it. With uncommitted
  changes you first choose between stashing them and taking them along.
- **Recipe buttons:** click one, fill in the form, check the command preview, press Run. Steps
  run one at a time; the first failing step stops the recipe and the panel shows which one and
  its exit code. Stop (or Ctrl+C) cancels.

### Built-in recipes

| Recipe | What it runs |
|---|---|
| New branch + commit + push | `checkout -b`, `add -A` (with file picker), `commit -m`, `push -u` |
| Delete branch | `switch` to another branch, `branch -d` (refuses unmerged branches), optionally `push origin --delete` + `fetch --prune` (only if the branch is merged into the one you switched to) |
| Merge into… | merge the current branch into dev / staging / prod (`pull --ff-only`, optional `--no-ff`), push, and always return to your branch |
| Sync with main | `fetch`, then `rebase` or `merge` the default branch from origin |
| Switch branch | `git switch` (what the branch switcher runs) |
| Stash & switch | stash everything (incl. untracked), switch, optionally pop only that stash |
| Version bump + tag + push | update package.json / Cargo.toml / tauri.conf.json (+ lock files), one `chore: release vX.Y.Z` commit, annotated tag, `push --follow-tags` |
| Abort merge/rebase & go back | abort whatever is in progress and return to the previous branch |
| Undo last commit | `reset --soft HEAD~1` (changes stay staged) |
| Clean merged branches | delete local branches merged into the default branch with `branch -d` (keeps the current and default branch, main, master, dev, develop, staging, prod, production and `release/*`) |

## Safety

- **No injection.** Values from forms are always inserted as one single-quoted shell word.
  Branch names are validated (no spaces, no leading `-`, git's ref rules).
- **What you see is what runs.** The backend renders the commands, and refuses to run if they
  changed since the preview (for example, you switched branches meanwhile).
- **Confirmation** for steps that push to a protected branch, force-push, delete a remote branch,
  reset, force-delete a branch (`branch -D`) or `clean -f`. Branchdial follows `checkout`/`switch`
  steps, so in *Merge into prod* it knows the later `git push` goes to `prod`. Protected
  branches default to `main`, `master`, `prod`, `production`, `staging` and `release/*`.
  (It reads commands as written; it can't see inside your git aliases.)
- **Uncommitted changes + branch switch:** the form asks whether to stash first (adds a visible
  `git stash push -u` step) or continue anyway.
- **File picker before committing:** a `git add -A` before a commit becomes a checklist of the
  changed files. `.env` files, private keys, `node_modules/`, build output (`dist/`, `build/`,
  `target/`, …) and logs start unchecked.
- **Conflicts:** when a recipe stops mid-merge or mid-rebase, the panel offers **Resolve** (lists
  the conflicted files; once they're fixed and `git add`ed, finishes the merge and runs the
  rest of the recipe) and **Abort & go back** (aborts and returns to the branch you started on).
- **Team recipes need your trust.** A repo's `.branchdial.yml` could contain any command, so its
  buttons only appear after you've reviewed the file, and again whenever it changes.

## Recipes

Branchdial loads recipes from three places; a later one replaces an earlier one with the same
`id`:

1. the built-in set,
2. `~/.branchdial/recipes.yml`: just for you,
3. `.branchdial.yml` in the repo root: shared with your team (after you trust it).

A file is a list of recipes, or a map with `recipes:` and settings:

```yaml
protectedBranches: [main, "release/*"]   # repo files and the global file only
recipes:
  - id: bcp                        # used for overriding; defaults to a slug of the name
    name: New branch + commit + push
    description: Create a branch, commit all changes, push and set upstream
    confirm: false                 # always ask before running
    inputs:
      - { id: branch, label: Branch name, type: branch, exists: false, required: true }
      - { id: msg, label: Commit message, required: true }
    steps:
      - git checkout -b "{{branch}}"
      - git add -A
      - git commit -m "{{msg}}"
      - git push -u origin "{{branch}}"

  - { id: undo-commit, disabled: true }   # hide a built-in
```

### Inputs

| Field | Meaning |
|---|---|
| `id` | name used in `{{placeholders}}` (letters, digits, `_`) |
| `label`, `placeholder` | shown in the form |
| `required` | must be filled in |
| `type` | `text` (default), `branch` (validated name, suggestions), `select` (needs `options`), `bool` (checkbox) |
| `default` | initial value |
| `exists` | branch inputs: `true` = must exist locally or on origin, `false` = must be a new name |

### Steps

A step is a command string, or an object:

```yaml
steps:
  - git fetch origin
  - run: git merge --no-ff "{{current_branch}}"
    when: no_ff                     # flag, !flag, name == value, name != value
  - run: git checkout "{{current_branch}}"
    always: true                    # runs even after a failed step (not while mid-merge)
```

- Placeholders are inserted as a safely quoted word. They work unquoted, as a whole `"{{x}}"`
  word, or inside double quotes (`"WIP: {{msg}}"`); inside single quotes they're rejected.
- Built-in variables: `{{current_branch}}`, `{{default_branch}}`, `{{repo_root}}`, and
  `{{operation}}` (`merge`, `rebase`, `cherry-pick`, `revert`, or empty).
- **YAML gotcha:** a step containing `": "` must be wrapped in single quotes, e.g.
  `- 'git commit -m "fix: {{msg}}"'`. Branchdial's error message points this out.

### Requirements before running

```yaml
requires:
  - { when: operation, message: "Nothing to abort." }
```

### Version bumps

A recipe with a `version:` block gets `{{version}}`, `{{new_version}}` and `{{version_files}}`
(a list, usable only unquoted), plus a step that updates the files:

```yaml
- id: version-bump
  name: Version bump + tag + push
  inputs:
    - { id: bump, type: select, options: [patch, minor, major], default: patch }
  version:
    bump: bump        # the input holding patch / minor / major (or an explicit 1.2.3)
    files: [package.json, src-tauri/Cargo.toml, src-tauri/tauri.conf.json]  # missing ones are skipped
  steps:
    - update_version_files: true   # npm version for package.json, an in-place edit for the others
    - git add -- {{version_files}}
    - 'git commit -m "chore: release v{{new_version}}" -- {{version_files}}'
    - git tag -a "v{{new_version}}" -m "v{{new_version}}"
    - git push --follow-tags
```

The current version comes from the first file that has one (all must agree), or from the latest
`vX.Y.Z` tag if none do. `package-lock.json` and `Cargo.lock` are updated alongside. If the
terminal is in a subfolder, the recipe starts with a `cd` to the repo root.

## Configuration

`~/.branchdial/config.yml` (every field optional):

```yaml
shell:
  path: C:\Program Files\Git\bin\bash.exe   # default: Git Bash on Windows, $SHELL elsewhere
  args: [--login, -i]                       # custom args turn off recipe support (see below)
startDir: ~/code                            # default: your home directory
protectedBranches: [main, master, "release/*"]
checkForUpdates: false                      # default: true
```

**How steps know when a command finished:** Branchdial starts bash with
`--rcfile ~/.branchdial/shell/bash-init.sh` (zsh through a `ZDOTDIR` shim). That script loads
your usual startup files (`/etc/profile`, then `~/.bash_profile` or `~/.bashrc`) and adds a
prompt hook that reports each command's exit code to Branchdial through an invisible escape
sequence. If you give the shell your own `args`, Branchdial launches it exactly as configured
and recipes are disabled.

## Development

Prerequisites: [Node.js](https://nodejs.org) 20+, [Rust](https://rustup.rs) (stable), git, and the
[Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for your OS (WebView2 and the
MSVC Build Tools on Windows; WebKitGTK on Linux).

```sh
npm install
npm run tauri dev      # run with hot reload
npm run tauri build    # build an installer for your platform
cd src-tauri && cargo test
```

The stack is Tauri 2 (Rust) with a React + TypeScript + Vite frontend, xterm.js for the terminal
and portable-pty for the shell. Recipes are parsed in Rust with serde_yaml.

Tests run real git in throwaway repos under the system temp directory, with a local bare repo as
`origin`; they never touch a real remote. CI runs the build and tests on Windows, macOS and Linux
for every push to `main` and every pull request. Pushing a `v*` tag builds installers for all
three into a draft GitHub release.

**Releasing updates:** the release workflow signs the update files with the private key in the
`TAURI_SIGNING_PRIVATE_KEY` (and `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`) repo secrets and uploads
`latest.json`, which installed apps read from the latest *published* release, so users are offered
the update only once you publish the draft. The matching public key is in
`src-tauri/tauri.conf.json`. Keep a backup of the private key: without it, installed copies can't
verify, and so won't install, any future update. A local `npm run tauri build` needs the same two
variables set.

## Code signing policy

Free code signing for Windows is provided by [SignPath.io](https://signpath.io), certificate by
[SignPath Foundation](https://signpath.org).

- Committers and reviewers: [Members team](https://github.com/orgs/ShapeTwo-2/people)
- Approvers: [Owners](https://github.com/orgs/ShapeTwo-2/people?query=role%3Aowner)

Every signed file is built from this repository's source by the GitHub Actions release workflow.
A release is signed only after an approver reviews and approves the signing request.

## Privacy

Branchdial does not collect or send any data about you or your use of the app. It connects to the
internet in two cases only:

- **Update check:** on startup (unless `checkForUpdates: false`) and when you click
  **Check for updates**, it downloads `latest.json` from this repository's GitHub releases, and
  downloads the update itself only if you choose to install it. GitHub sees the request like any
  web request ([GitHub's privacy statement](https://docs.github.com/site-policy/privacy-policies/github-general-privacy-statement)).
- **Commands you run:** git commands in the terminal or in recipes (fetch, pull, push…) talk to
  the remotes you configured, exactly as they would in any other terminal.

## License

[MIT](LICENSE)
