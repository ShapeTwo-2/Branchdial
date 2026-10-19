import type { ReactNode } from "react";
import type { Recipe, RecipeList } from "../lib/types";

interface Props {
  statusBar: ReactNode;
  list: RecipeList | null;
  listError: string | null;
  /** Why recipe buttons are disabled right now, or null if they're usable. */
  disabledReason: string | null;
  onRefresh: () => void;
  onOpen: (recipe: Recipe) => void;
  onAdd: () => void;
  onReviewTrust: () => void;
  onCheckUpdates: () => void;
}

function basename(path: string) {
  return path.replace(/[\\/]+$/, "").split(/[\\/]/).pop() ?? path;
}

const SOURCE_LABEL = { builtin: null, global: "global", repo: "repo" } as const;

export function Sidebar({ statusBar, list, listError, disabledReason, onRefresh, onOpen, onAdd, onReviewTrust, onCheckUpdates }: Props) {
  return (
    <aside className="sidebar">
      <div className="sidebar-brand">Branchdial</div>
      {statusBar}
      <section className="sidebar-section recipes">
        <div className="section-title">
          <span>Recipes</span>
          <button className="icon-btn" title="Reload recipes" onClick={onRefresh}>
            ⟳
          </button>
        </div>
        {disabledReason && <div className="hint">{disabledReason}</div>}
        {list?.pendingTrust && (
          <div className="trust-banner">
            <span>
              {list.pendingTrust.changed
                ? "This repo's team recipes changed."
                : "This repo has team recipes (.branchdial.yml)."}
            </span>
            <button className="btn btn-small" onClick={onReviewTrust}>
              Review
            </button>
          </div>
        )}
        <div className="recipe-list">
          {list?.recipes.map((r) => (
            <button
              key={r.id}
              className="recipe-btn"
              disabled={!!disabledReason}
              title={r.description ?? r.name}
              onClick={() => onOpen(r)}
            >
              <span className="recipe-name">{r.name}</span>
              {SOURCE_LABEL[r.source] && <span className={`badge badge-${r.source}`}>{SOURCE_LABEL[r.source]}</span>}
            </button>
          ))}
        </div>
        {(listError || (list && list.errors.length > 0)) && (
          <div className="load-errors">
            {listError && <div>{listError}</div>}
            {list?.errors.map((e, i) => (
              <div key={i} title={e.file}>
                <strong>{basename(e.file)}:</strong> {e.message}
              </div>
            ))}
          </div>
        )}
        <button className="add-btn" onClick={onAdd}>
          + Add recipe
        </button>
      </section>
      <button className="update-btn" onClick={onCheckUpdates}>
        Check for updates
      </button>
    </aside>
  );
}
