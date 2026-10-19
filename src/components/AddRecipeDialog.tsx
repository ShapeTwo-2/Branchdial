import { useEffect, useState } from "react";

interface Props {
  globalFile: string | null;
  repoRoot: string | null;
  onClose: () => void;
}

const SNIPPET = `- name: Amend last commit
  description: Add staged changes to the last commit, keep its message
  confirm: true
  steps:
    - git commit --amend --no-edit
`;

/** Until the in-app editor exists: where recipe files live and what they look like. */
export function AddRecipeDialog({ globalFile, repoRoot, onClose }: Props) {
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onClose]);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(SNIPPET);
      setCopied(true);
    } catch {
      setCopied(false);
    }
  };

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="dialog" role="dialog" aria-label="Add a recipe">
        <header className="dialog-header">
          <h2>Add a recipe</h2>
          <p className="muted">Recipes are YAML. Add one to either file, then press ⟳ next to "Recipes".</p>
        </header>
        <ul className="file-list">
          <li>
            <strong>Just for you:</strong> <code>{globalFile ?? "~/.branchdial/recipes.yml"}</code>
          </li>
          <li>
            <strong>Shared with your team:</strong>{" "}
            <code>{repoRoot ? `${repoRoot}/.branchdial.yml` : ".branchdial.yml in the repo root"}</code>
          </li>
        </ul>
        <pre className="snippet">{SNIPPET}</pre>
        <p className="muted small">
          Tip: wrap a step in single quotes if it contains <code>": "</code>, e.g.{" "}
          <code>{`- 'git commit -m "fix: {{msg}}"'`}</code>
        </p>
        <footer className="dialog-footer">
          <button className="btn" onClick={copy}>
            {copied ? "Copied" : "Copy example"}
          </button>
          <button className="btn btn-primary" onClick={onClose} autoFocus>
            Close
          </button>
        </footer>
      </div>
    </div>
  );
}
