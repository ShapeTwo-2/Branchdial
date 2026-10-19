// Typed wrappers around every Tauri command and event the frontend uses.
// Components never call `invoke` / `listen` directly.

import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  GitInfo,
  InputValues,
  Plan,
  PromptEvent,
  PtyExit,
  RecipeList,
  RepoStatus,
  RunChoices,
  RunDone,
  RunStarted,
  SpawnInfo,
  StepEvent,
} from "./types";

export function ptySpawn(cols: number, rows: number, onData: (chunk: string) => void): Promise<SpawnInfo> {
  const channel = new Channel<string>();
  channel.onmessage = onData;
  return invoke<SpawnInfo>("pty_spawn", { cols, rows, onData: channel });
}

export function ptyWrite(data: string): Promise<void> {
  return invoke("pty_write", { data });
}

export function ptyResize(cols: number, rows: number): Promise<void> {
  return invoke("pty_resize", { cols, rows });
}

export function recipesList(): Promise<RecipeList> {
  return invoke<RecipeList>("recipes_list");
}

export function gitInfo(): Promise<GitInfo> {
  return invoke<GitInfo>("git_info");
}

/** Result arrives as a `git://status` event. */
export function statusRefresh(): Promise<void> {
  return invoke("status_refresh");
}

export function gitBranches(): Promise<string[]> {
  return invoke<string[]>("git_branches");
}

export function recipePlan(
  id: string,
  values: InputValues,
  opts: { staged: string[] | null; stashFirst: boolean },
): Promise<Plan> {
  return invoke<Plan>("recipe_plan", { id, values, staged: opts.staged, stashFirst: opts.stashFirst });
}

/** `expected` is the command list the user reviewed; the backend refuses if it changed. */
export function recipeRun(id: string, values: InputValues, expected: string[], choices: RunChoices): Promise<void> {
  return invoke("recipe_run", { id, values, expected, choices });
}

/** Trusts the repo's .branchdial.yml as reviewed (the backend checks `hash` still matches). */
export function repoTrust(hash: string): Promise<void> {
  return invoke("repo_trust", { hash });
}

export function conflictFiles(): Promise<string[]> {
  return invoke<string[]>("conflict_files");
}

/** Finishes the merge/rebase, then runs the rest of the stopped recipe. */
export function conflictContinue(): Promise<void> {
  return invoke("conflict_continue");
}

export function conflictAbort(): Promise<void> {
  return invoke("conflict_abort");
}

export function recipeCancel(): Promise<void> {
  return invoke("recipe_cancel");
}

function on<T>(event: string, handler: (payload: T) => void): Promise<UnlistenFn> {
  return listen<T>(event, (e) => handler(e.payload));
}

export const onPtyExit = (h: (e: PtyExit) => void) => on("pty://exit", h);
export const onPrompt = (h: (e: PromptEvent) => void) => on("shell://prompt", h);
export const onRunStarted = (h: (e: RunStarted) => void) => on("run://started", h);
export const onRunStep = (h: (e: StepEvent) => void) => on("run://step", h);
export const onRunDone = (h: (e: RunDone) => void) => on("run://done", h);
export const onGitStatus = (h: (e: RepoStatus) => void) => on("git://status", h);
