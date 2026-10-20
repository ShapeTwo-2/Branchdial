import { useCallback, useEffect, useRef, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { Terminal } from "./components/Terminal";
import { RecipeDialog } from "./components/RecipeDialog";
import { RunPanel, type RunView } from "./components/RunPanel";
import { AddRecipeDialog } from "./components/AddRecipeDialog";
import { StatusBar } from "./components/StatusBar";
import { TrustDialog } from "./components/TrustDialog";
import { UpdateDialog } from "./components/UpdateDialog";
import {
  gitInfo,
  onGitStatus,
  onPrompt,
  onPtyExit,
  onRunDone,
  onRunStarted,
  onRunStep,
  recipePlan,
  recipeRun,
  recipesList,
  statusRefresh,
  updateCheck,
  updatesAutoCheck,
  type Update,
} from "./lib/ipc";
import type { GitInfo, InputValues, Recipe, RecipeList, RepoStatus, SpawnInfo } from "./lib/types";

/** The built-in recipe the branch dropdown runs. */
const SWITCH_RECIPE = "switch-branch";

const SKIPPED_KEY = "branchdial.skippedVersion";

function skippedVersion(): string | null {
  try {
    return localStorage.getItem(SKIPPED_KEY);
  } catch {
    return null;
  }
}

export default function App() {
  const [listening, setListening] = useState(false);
  const [git, setGit] = useState<GitInfo | null>(null);
  const [status, setStatus] = useState<RepoStatus | null>(null);
  const [spawn, setSpawn] = useState<SpawnInfo | null>(null);
  const [ready, setReady] = useState(false);
  const [list, setList] = useState<RecipeList | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [run, setRun] = useState<RunView | null>(null);
  const [openRecipe, setOpenRecipe] = useState<{ recipe: Recipe; initial?: InputValues } | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [trustOpen, setTrustOpen] = useState(false);
  // null: closed; "check": the user asked to check now; otherwise found on startup.
  const [updateOpen, setUpdateOpen] = useState<Update | "check" | null>(null);
  const cwdRef = useRef<string | null>(null);
  const focusTerminal = useRef<() => void>(() => {});

  const refresh = useCallback(() => {
    recipesList()
      .then((l) => {
        setList(l);
        setListError(null);
      })
      .catch((e) => setListError(String(e)));
  }, []);

  // Subscribe before the terminal mounts so the shell's first prompt isn't missed.
  useEffect(() => {
    const subs = [
      onPrompt((p) => {
        setReady(true);
        if (p.cwd && p.cwd !== cwdRef.current) {
          cwdRef.current = p.cwd;
          refresh();
        }
      }),
      onPtyExit(() => setReady(false)),
      onRunStarted((s) =>
        setRun({
          name: s.name,
          steps: s.steps,
          states: s.steps.map(() => "pending"),
          exitCodes: s.steps.map(() => null),
          notes: s.steps.map(() => null),
          done: null,
        }),
      ),
      onRunStep((e) =>
        setRun((r) => {
          if (!r) return r;
          const states = [...r.states];
          const exitCodes = [...r.exitCodes];
          const notes = [...r.notes];
          states[e.index] = e.state;
          exitCodes[e.index] = e.exitCode;
          notes[e.index] = e.note;
          return { ...r, states, exitCodes, notes };
        }),
      ),
      onRunDone((d) => {
        setRun((r) => (r ? { ...r, done: d } : r));
        refresh();
      }),
      onGitStatus(setStatus),
    ];
    Promise.all(subs).then(() => setListening(true));
    return () => {
      subs.forEach((p) => void p.then((off) => off()));
    };
  }, [refresh]);

  // Startup check: is git installed?
  useEffect(() => {
    gitInfo()
      .then(setGit)
      .catch(() => setGit({ install: null, installHint: "Could not check for git." }));
  }, []);

  // Startup update check, a few seconds in so it doesn't compete with the shell starting.
  // Offline or failing checks stay silent; "Check for updates" shows the error.
  useEffect(() => {
    const timer = setTimeout(async () => {
      try {
        if (!(await updatesAutoCheck())) return;
        const u = await updateCheck();
        if (u && u.version !== skippedVersion()) setUpdateOpen((open) => open ?? u);
      } catch {
        // ignore
      }
    }, 5000);
    return () => clearTimeout(timer);
  }, []);

  // Commits made elsewhere (an editor, another terminal) show up when you come back.
  useEffect(() => {
    const onFocus = () => void statusRefresh();
    window.addEventListener("focus", onFocus);
    return () => window.removeEventListener("focus", onFocus);
  }, []);

  const running = !!run && !run.done;
  let disabledReason: string | null = null;
  if (git && !git.install) disabledReason = "Recipes need git. See the message above.";
  else if (!spawn) disabledReason = "Starting the shell…";
  else if (!spawn.integration)
    disabledReason = "Recipes need bash or zsh with Branchdial's shell integration. Remove custom shell args from ~/.branchdial/config.yml.";
  else if (!ready) disabledReason = "Waiting for the shell prompt…";
  else if (running) disabledReason = "A recipe is running.";

  const dismissRun = useCallback(() => {
    setRun(null);
    focusTerminal.current();
  }, []);

  // Branch dropdown: a clean switch runs straight away in the terminal. Anything that
  // needs a decision (uncommitted changes, a confirmation, an error) opens the recipe
  // dialog with the branch filled in.
  const switchRecipe = list?.recipes.find((r) => r.id === SWITCH_RECIPE) ?? null;
  const switchBranch = async (branch: string) => {
    if (!switchRecipe) return;
    const values = { branch };
    try {
      const p = await recipePlan(SWITCH_RECIPE, values, { staged: null, stashFirst: false });
      // Skipping the preview is only for Branchdial's own one-step `git switch`. A
      // .branchdial.yml (or the global file) can replace this recipe by id; then its
      // steps are shown in the dialog like any other recipe before they run.
      const ownSwitch =
        switchRecipe.source === "builtin" && p.steps.length === 1 && p.steps[0].command.startsWith("git switch ");
      if (ownSwitch && p.errors.length === 0 && !p.dirtySwitch && p.risks.length === 0 && !switchRecipe.confirm) {
        await recipeRun(SWITCH_RECIPE, values, p.steps.map((s) => s.command), {
          staged: null,
          stashFirst: false,
          allowDirtySwitch: false,
          confirmed: false,
        });
        focusTerminal.current();
        return;
      }
    } catch {
      // The dialog re-plans and shows what went wrong.
    }
    setOpenRecipe({ recipe: switchRecipe, initial: values });
  };

  const closeDialog = () => {
    setOpenRecipe(null);
    setAddOpen(false);
    setTrustOpen(false);
    setUpdateOpen(null);
    focusTerminal.current();
  };

  return (
    <div className="app">
      <Sidebar
        statusBar={
          <StatusBar
            git={git}
            status={status}
            onRefresh={() => void statusRefresh()}
            switcher={switchRecipe && { disabledReason, onSwitch: (b) => void switchBranch(b) }}
          />
        }
        list={list}
        listError={listError}
        disabledReason={disabledReason}
        onRefresh={refresh}
        onOpen={(recipe) => setOpenRecipe({ recipe })}
        onAdd={() => setAddOpen(true)}
        onReviewTrust={() => setTrustOpen(true)}
        onCheckUpdates={() => setUpdateOpen("check")}
      />
      <main className="main">
        {run && <RunPanel run={run} status={status} onDismiss={dismissRun} />}
        {listening && (
          <Terminal
            onSpawned={(info) => {
              setSpawn(info);
              setReady(false);
              cwdRef.current = null;
            }}
            focusRef={focusTerminal}
          />
        )}
      </main>
      {openRecipe && (
        <RecipeDialog
          key={openRecipe.recipe.id}
          recipe={openRecipe.recipe}
          initial={openRecipe.initial}
          onClose={closeDialog}
        />
      )}
      {trustOpen && list?.pendingTrust && (
        <TrustDialog
          pending={list.pendingTrust}
          onClose={closeDialog}
          onTrusted={() => {
            closeDialog();
            refresh();
          }}
        />
      )}
      {updateOpen && (
        <UpdateDialog
          found={updateOpen === "check" ? null : updateOpen}
          running={running}
          onSkip={(version) => {
            try {
              localStorage.setItem(SKIPPED_KEY, version);
            } catch {
              // ignore
            }
            closeDialog();
          }}
          onClose={closeDialog}
        />
      )}
      {addOpen && <AddRecipeDialog globalFile={list?.globalFile ?? null} repoRoot={list?.repoRoot ?? null} onClose={closeDialog} />}
    </div>
  );
}
