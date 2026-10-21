// Typed wrappers around every Tauri command and event the frontend uses.
// Components never call `invoke` / `listen` directly.

import { getVersion } from "@tauri-apps/api/app";
import { Channel, invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import type {
  BranchEntry,
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

/** Local branches, then those only on origin, for the branch switcher. */
export function gitBranchList(): Promise<BranchEntry[]> {
  return invoke<BranchEntry[]>("git_branch_list");
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

export type { Update };

/** This build's version (from tauri.conf.json). */
export function appVersion(): Promise<string> {
  return getVersion();
}

/** True if Branchdial had run on this machine before this launch (an update, not a fresh install). */
export function appRanBefore(): Promise<boolean> {
  return invoke<boolean>("app_ran_before");
}

/** `checkForUpdates` from ~/.branchdial/config.yml (default true). */
export function updatesAutoCheck(): Promise<boolean> {
  return invoke<boolean>("updates_auto_check");
}

/** The newer release, or null when this is the latest. The signature is checked on download. */
export function updateCheck(): Promise<Update | null> {
  return check();
}

/**
 * Downloads, verifies and installs the update, then restarts. The backend refuses
 * while a recipe is running and closes the shell first. On Windows the installer
 * quits the app itself, so this never returns there.
 */
export async function updateInstall(update: Update, onProgress: (done: number, total: number | null) => void): Promise<void> {
  let done = 0;
  let total: number | null = null;
  await update.download((e) => {
    if (e.event === "Started") total = e.data.contentLength ?? null;
    else if (e.event === "Progress") done += e.data.chunkLength;
    onProgress(done, total);
  });
  await invoke("updates_prepare_install");
  await update.install();
  await relaunch();
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
