import { useCallback, useEffect, useRef, useState } from "react";
import { Sidebar } from "./components/Sidebar";
import { Terminal } from "./components/Terminal";
import { RecipeDialog } from "./components/RecipeDialog";
import { RunPanel, type RunView } from "./components/RunPanel";
import { AddRecipeDialog } from "./components/AddRecipeDialog";
import { StatusBar } from "./components/StatusBar";
import { TrustDialog } from "./components/TrustDialog";
import {
  gitInfo,
  onGitStatus,
  onPrompt,
  onPtyExit,
  onRunDone,
  onRunStarted,
  onRunStep,
  recipesList,
  statusRefresh,
} from "./lib/ipc";
import type { GitInfo, Recipe, RecipeList, RepoStatus, SpawnInfo } from "./lib/types";

export default function App() {
  const [listening, setListening] = useState(false);
  const [git, setGit] = useState<GitInfo | null>(null);
  const [status, setStatus] = useState<RepoStatus | null>(null);
  const [spawn, setSpawn] = useState<SpawnInfo | null>(null);
  const [ready, setReady] = useState(false);
  const [list, setList] = useState<RecipeList | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  const [run, setRun] = useState<RunView | null>(null);
  const [openRecipe, setOpenRecipe] = useState<Recipe | null>(null);
  const [addOpen, setAddOpen] = useState(false);
  const [trustOpen, setTrustOpen] = useState(false);
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

  const closeDialog = () => {
    setOpenRecipe(null);
    setAddOpen(false);
    setTrustOpen(false);
    focusTerminal.current();
  };

  return (
    <div className="app">
      <Sidebar
        statusBar={<StatusBar git={git} status={status} onRefresh={() => void statusRefresh()} />}
        list={list}
        listError={listError}
        disabledReason={disabledReason}
        onRefresh={refresh}
        onOpen={setOpenRecipe}
        onAdd={() => setAddOpen(true)}
        onReviewTrust={() => setTrustOpen(true)}
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
      {openRecipe && <RecipeDialog key={openRecipe.id} recipe={openRecipe} onClose={closeDialog} />}
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
      {addOpen && <AddRecipeDialog globalFile={list?.globalFile ?? null} repoRoot={list?.repoRoot ?? null} onClose={closeDialog} />}
    </div>
  );
}
