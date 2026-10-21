# Changelog

Each `## <version>` section is shown in Branchdial's "What's new" dialog after an update, and
becomes the text of that version's GitHub release (and of the in-app update prompt). Write the
section before running *Version bump*. Supported: `### headings`, `- bullets`, `**bold**`,
`` `code` ``.

## 1.1.0

Security release: update recommended.

### New

- **Branch switcher:** click the branch name under the repo name to search every branch,
  including ones that only exist on origin, and switch to it in the terminal.
- **What's new:** after an update, Branchdial shows what changed (this window). Reopen it
  any time from the bottom of the sidebar.

### Security

- Opening a downloaded project can no longer run code from its `.git` folder.
- Recipe values and file names can't be turned into commands, even file names with control
  characters.
- When a team recipe file changes, you review a diff of what changed, not the whole file again.
- Invisible and text-direction characters are shown as red `⟨U+…⟩` badges wherever you review
  commands, so a command can't look different from what it runs.
- Terminal links show their real address and never open by themselves; clicking copies a web link.
- Updates are signed, and the build pipeline is locked to exact versions of its tools.

See the README's Safety section and `SECURITY.md` for details.

### Might affect custom recipes

- A placeholder inside `$(…)`, backticks, `bash -c "…"`, `eval` or `ssh` is now refused when the
  recipe file loads, because a second shell parses it again and quoting can't protect it. Run the
  command directly instead: `git switch "{{branch}}"`.
- Form values containing invisible or text-direction characters are refused.
- *Version bump* runs `npm version` with `--ignore-scripts`, so `package.json`
  `preversion`/`version`/`postversion` scripts no longer run.

## 1.0.2

- **Delete branch** recipe: switch to another branch, delete one locally, and optionally on
  origin too (only if it's merged into the branch you switched to).
- **Updates from inside the app:** Branchdial checks for new versions on startup and asks
  before installing. Turn it off with `checkForUpdates: false` in `~/.branchdial/config.yml`.
- The setup installer shows the Branchdial icon.
