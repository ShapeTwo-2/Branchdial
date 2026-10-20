# Security policy

## Reporting a vulnerability

Please report vulnerabilities **privately**, not in a public issue:

1. Open the repository's **Security** tab.
2. Click **Report a vulnerability** (GitHub private vulnerability reporting).
3. Describe what an attacker can do, the steps or a proof of concept, and the Branchdial
   version and OS you tested on.

Only the maintainers can see the report. If you can't use GitHub's form, open a public issue
asking for a private contact, without any details of the problem.

## Supported versions

Only the **latest release** gets security fixes. Branchdial updates itself (it asks before
installing), so fixes reach users through a new release rather than patches to old versions.

| Version | Supported |
|---|---|
| Latest release | Yes |
| Anything older | No: update to the latest |

## What to expect

- **Acknowledgement** within 7 days.
- **Assessment** within 14 days: whether we consider it a vulnerability, how severe, and a
  rough plan.
- **Fix and release:** as fast as the severity calls for. We keep you updated and may ask you to
  check the fix.
- **Disclosure:** we publish a GitHub security advisory when the fix is released, crediting you
  unless you prefer otherwise. Please don't disclose before that, or before 90 days have passed
  if we go quiet.

## What counts

Branchdial is a terminal, so running commands is expected. What we treat as a vulnerability is
someone **other than the user** getting commands to run, or tricking the user into running
something other than what they were shown. For example:

- a repository (cloned or downloaded, including its `.git` folder), a file name, a branch name or
  program output that makes Branchdial run a command;
- a recipe preview that doesn't match what actually runs;
- a repo's `.branchdial.yml` that takes effect without the user reviewing it;
- anything that reaches the app's webview or IPC from outside;
- an update that installs without a valid signature.

Not vulnerabilities on their own:

- a recipe the user trusted doing what its file says;
- commands the user types or runs themselves;
- getting around the **mistake prevention** features (protected-branch confirmations, the file
  picker, the stash prompt). They catch slips, not attacks; the README's Safety section explains
  the difference.
