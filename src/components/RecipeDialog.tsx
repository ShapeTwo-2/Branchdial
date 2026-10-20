import { useEffect, useMemo, useRef, useState } from "react";
import { gitBranches, recipePlan, recipeRun } from "../lib/ipc";
import type { InputDef, InputValues, Plan, Recipe } from "../lib/types";

interface Props {
  recipe: Recipe;
  /** Pre-filled answers (e.g. the branch picked in the branch dropdown). */
  initial?: InputValues;
  onClose: () => void;
}

type DirtyChoice = "stash" | "continue";

function initialValues(inputs: InputDef[]): InputValues {
  const v: InputValues = {};
  for (const i of inputs) {
    if (i.type === "bool") v[i.id] = typeof i.default === "boolean" ? i.default : false;
    else if (i.type === "select") v[i.id] = typeof i.default === "string" ? i.default : (i.options?.[0] ?? "");
    else v[i.id] = typeof i.default === "string" ? i.default : "";
  }
  return v;
}

const STATUS_LABEL: Record<string, string> = {
  M: "modified",
  A: "added",
  D: "deleted",
  R: "renamed",
  C: "copied",
  T: "type changed",
  U: "conflicted",
  "??": "new",
};

export function RecipeDialog({ recipe, initial, onClose }: Props) {
  const [values, setValues] = useState<InputValues>(() => ({ ...initialValues(recipe.inputs), ...initial }));
  // Files picked for `git add -A`; null until the first plan offers a picker.
  const [staged, setStaged] = useState<string[] | null>(null);
  const [dirtyChoice, setDirtyChoice] = useState<DirtyChoice>("stash");
  const [plan, setPlan] = useState<{ key: string; plan: Plan } | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [runError, setRunError] = useState<string | null>(null);
  // Fields the user has typed in. Errors on those show live; "Required" waits for Run,
  // so moving focus away (to Cancel, or another window) never pops up errors.
  const [edited, setEdited] = useState<Set<string>>(new Set());
  const [submitted, setSubmitted] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const [starting, setStarting] = useState(false);
  const [branches, setBranches] = useState<string[]>([]);
  const firstField = useRef<HTMLInputElement | HTMLSelectElement | null>(null);
  const runButton = useRef<HTMLButtonElement>(null);

  // Only stash when the plan actually reports a dirty switch.
  const dirtyKnown = !!plan?.plan.dirtySwitch;
  const stashFirst = dirtyKnown && dirtyChoice === "stash";
  const key = useMemo(() => JSON.stringify({ values, staged, stashFirst }), [values, staged, stashFirst]);

  // Suggestions for "switch to an existing branch" style inputs.
  const wantsBranches = recipe.inputs.some((i) => i.type === "branch" && i.exists !== false);
  useEffect(() => {
    if (wantsBranches) gitBranches().then(setBranches).catch(() => setBranches([]));
  }, [wantsBranches]);

  // The backend renders the commands; refresh the preview as the user types.
  useEffect(() => {
    let cancelled = false;
    const t = setTimeout(() => {
      recipePlan(recipe.id, values, { staged, stashFirst })
        .then((p) => {
          if (cancelled) return;
          setPlan({ key, plan: p });
          setPlanError(null);
          // First time the picker appears: start from the suggested files.
          if (p.staging && staged === null) {
            setStaged(p.staging.files.filter((f) => f.suggested).map((f) => f.path));
          }
        })
        .catch((e) => !cancelled && setPlanError(String(e)));
    }, 120);
    return () => {
      cancelled = true;
      clearTimeout(t);
    };
    // `key` stands in for values/staged/stashFirst (same content, stable identity).
  }, [recipe.id, key]);

  useEffect(() => {
    (firstField.current ?? runButton.current)?.focus();
  }, []);

  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.preventDefault();
        if (confirming) setConfirming(false);
        else onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [confirming, onClose]);

  const current = plan?.key === key ? plan.plan : null;
  const shown = current ?? plan?.plan ?? null;
  const fieldErrors = new Map<string, string>();
  const generalErrors: string[] = [];
  for (const e of current?.errors ?? []) {
    if (e.input) fieldErrors.set(e.input, e.message);
    else generalErrors.push(e.message);
  }
  const risks = current?.risks ?? [];
  const riskySteps = new Map<number, string[]>();
  for (const r of risks) riskySteps.set(r.step, [...(riskySteps.get(r.step) ?? []), r.message]);
  const needsConfirm = recipe.confirm || risks.length > 0;
  const canRun = !!current && current.errors.length === 0 && current.steps.length > 0 && !starting;

  const resetConfirm = () => {
    setConfirming(false);
    setRunError(null);
  };

  const set = (id: string, v: string | boolean) => {
    setValues((old) => ({ ...old, [id]: v }));
    setEdited((s) => (s.has(id) ? s : new Set(s).add(id)));
    resetConfirm();
  };

  const toggleFile = (path: string, on: boolean) => {
    setStaged((s) => {
      const cur = s ?? [];
      return on ? [...cur, path] : cur.filter((p) => p !== path);
    });
    resetConfirm();
  };

  const submit = async (e?: React.FormEvent) => {
    e?.preventDefault();
    setSubmitted(true);
    if (!canRun || !current) return;
    if (needsConfirm && !confirming) {
      setConfirming(true);
      runButton.current?.focus();
      return;
    }
    setStarting(true);
    try {
      await recipeRun(
        recipe.id,
        values,
        current.steps.map((s) => s.command),
        {
          staged,
          stashFirst,
          allowDirtySwitch: dirtyKnown && dirtyChoice === "continue",
          confirmed: confirming,
        },
      );
      onClose();
    } catch (err) {
      setRunError(String(err));
      setConfirming(false);
    } finally {
      setStarting(false);
    }
  };

  const staging = shown?.staging;
  const dirty = shown?.dirtySwitch;

  return (
    <div className="dialog-backdrop" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <form className="dialog" onSubmit={submit} aria-label={recipe.name}>
        <header className="dialog-header">
          <h2>{recipe.name}</h2>
          {recipe.description && <p className="muted">{recipe.description}</p>}
        </header>

        {recipe.inputs.length > 0 && (
          <div className="fields">
            {recipe.inputs.map((input, idx) => {
              const label = input.label ?? input.id;
              const err = fieldErrors.get(input.id);
              const hasText = String(values[input.id] ?? "").trim() !== "";
              const showErr = err && (submitted || (edited.has(input.id) && hasText));
              const refProp = idx === 0 ? { ref: firstField as React.Ref<never> } : {};
              if (input.type === "bool") {
                return (
                  <label key={input.id} className="field field-bool">
                    <input
                      type="checkbox"
                      checked={values[input.id] === true}
                      onChange={(e) => set(input.id, e.target.checked)}
                      {...refProp}
                    />
                    <span>{label}</span>
                  </label>
                );
              }
              return (
                <label key={input.id} className="field">
                  <span className="field-label">
                    {label}
                    {input.required && <span className="req"> *</span>}
                  </span>
                  {input.type === "select" ? (
                    <select value={String(values[input.id])} onChange={(e) => set(input.id, e.target.value)} {...refProp}>
                      {input.options?.map((o) => (
                        <option key={o} value={o}>
                          {o}
                        </option>
                      ))}
                    </select>
                  ) : (
                    <input
                      type="text"
                      list={input.type === "branch" && input.exists !== false ? "rb-branches" : undefined}
                      spellCheck={input.type !== "branch"}
                      autoComplete="off"
                      placeholder={input.placeholder ?? ""}
                      value={String(values[input.id] ?? "")}
                      onChange={(e) => set(input.id, e.target.value)}
                      className={showErr ? "invalid" : ""}
                      {...refProp}
                    />
                  )}
                  {showErr && <span className="field-error">{err}</span>}
                </label>
              );
            })}
          </div>
        )}

        {dirty && (
          <fieldset className="choice-box">
            <legend>
              You have {dirty.changed} uncommitted change{dirty.changed === 1 ? "" : "s"} and this recipe switches
              branches
            </legend>
            <label>
              <input
                type="radio"
                name="dirty"
                checked={dirtyChoice === "stash"}
                onChange={() => {
                  setDirtyChoice("stash");
                  resetConfirm();
                }}
              />
              <span>
                Stash them first <span className="muted">(recommended; get them back with git stash pop)</span>
              </span>
            </label>
            <label>
              <input
                type="radio"
                name="dirty"
                checked={dirtyChoice === "continue"}
                onChange={() => {
                  setDirtyChoice("continue");
                  resetConfirm();
                }}
              />
              <span>
                Continue anyway <span className="muted">(they come along to the other branch, or the switch fails)</span>
              </span>
            </label>
          </fieldset>
        )}

        {staging && staged && (
          <fieldset className="choice-box staging">
            <legend>
              Files to commit ({staged.length} of {staging.files.length})
              <span className="legend-actions">
                <button type="button" className="link-btn" onClick={() => setStaged(staging.files.map((f) => f.path))}>
                  all
                </button>
                <button type="button" className="link-btn" onClick={() => setStaged([])}>
                  none
                </button>
              </span>
            </legend>
            <div className="file-list-box">
              {staging.files.map((f) => (
                <label key={f.path} className={`stage-file${f.reason ? " flagged" : ""}`} title={f.reason ?? undefined}>
                  <input
                    type="checkbox"
                    checked={staged.includes(f.path)}
                    onChange={(e) => toggleFile(f.path, e.target.checked)}
                  />
                  <code className="stage-path">{f.path}</code>
                  <span className={`stage-code code-${f.code === "??" ? "new" : f.code}`}>{STATUS_LABEL[f.code] ?? f.code}</span>
                  {f.reason && <span className="stage-reason">⚠ {f.reason}</span>}
                </label>
              ))}
            </div>
          </fieldset>
        )}

        <div className="preview">
          <div className="preview-title">
            Commands that will be typed into the terminal
            {current && current.steps.length > 0 && <span className="muted"> ({current.steps.length})</span>}
          </div>
          {planError && <div className="error-box">{planError}</div>}
          <ol className="preview-steps">
            {shown?.steps.map((s, i) => {
              const r = current ? riskySteps.get(i) : undefined;
              return (
                <li key={i} className={`${current ? "" : "stale"}${r ? " risky" : ""}`}>
                  <code>{s.command}</code>
                  {s.always && (
                    <span className="tag" title="Runs even if an earlier step fails">
                      always
                    </span>
                  )}
                  {r && (
                    <span className="tag tag-risk" title={r.join("\n")}>
                      ⚠ needs confirmation
                    </span>
                  )}
                </li>
              );
            })}
          </ol>
          {generalErrors.map((m, i) => (
            <div key={i} className="error-box">
              {m}
            </div>
          ))}
          {current?.warnings.map((w, i) => (
            <div key={i} className="warning-box">
              {w}
            </div>
          ))}
        </div>
        <datalist id="rb-branches">
          {branches.map((b) => (
            <option key={b} value={b} />
          ))}
        </datalist>

        {runError && <div className="error-box">{runError}</div>}
        {confirming && (
          <div className="confirm-box" role="alert">
            {risks.length > 0 ? (
              <>
                <strong>Please confirm:</strong>
                <ul>
                  {risks.map((r, i) => (
                    <li key={i}>
                      Step {r.step + 1}: {r.message}
                    </li>
                  ))}
                </ul>
              </>
            ) : (
              <>This recipe asks for confirmation.</>
            )}{" "}
            Run the {current?.steps.length} commands above?
          </div>
        )}

        <footer className="dialog-footer">
          <span className="muted kbd-hint">Esc to {confirming ? "go back" : "close"}</span>
          <button type="button" className="btn" onClick={confirming ? () => setConfirming(false) : onClose}>
            {confirming ? "Back" : "Cancel"}
          </button>
          <button
            type="submit"
            ref={runButton}
            className={`btn btn-primary${confirming ? " btn-danger" : ""}`}
            disabled={!canRun && submitted}
          >
            {starting ? "Starting…" : confirming ? "Yes, run it" : needsConfirm ? "Review & run…" : "Run"}
          </button>
        </footer>
      </form>
    </div>
  );
}
