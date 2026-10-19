// Mirrors of the Rust structs in src-tauri/src. Keep field names in sync (serde uses camelCase).

export interface SpawnInfo {
  /** Absolute path of the shell binary that was started. */
  shell: string;
  args: string[];
  cwd: string;
  /** The prompt hook is installed, so recipes can run. */
  integration: boolean;
  /** Present on Windows: tells xterm.js it is talking to ConPTY. */
  windowsBuild: number | null;
}

export interface PtyExit {
  code: number | null;
}

export interface PromptEvent {
  exitCode: number;
  cwd: string;
}

export interface GitInstall {
  program: string;
  version: string;
}

export interface GitInfo {
  /** null when git isn't installed (or can't be found). */
  install: GitInstall | null;
  installHint: string;
}

export interface RepoStatus {
  cwd: string | null;
  /** null when the terminal is not inside a repository. */
  root: string | null;
  /** null on a detached HEAD. */
  branch: string | null;
  /** Short commit id when detached. */
  head: string | null;
  upstream: string | null;
  ahead: number;
  behind: number;
  staged: number;
  modified: number;
  untracked: number;
  conflicted: number;
  operation: GitOperation | null;
}

export type InputKind = "text" | "branch" | "select" | "bool";

export interface InputDef {
  id: string;
  label: string | null;
  placeholder: string | null;
  required: boolean;
  type: InputKind;
  options?: string[];
  default: string | boolean | null;
  /** Branch inputs: true = must exist, false = must be new. */
  exists: boolean | null;
}

export type RecipeSource = "builtin" | "global" | "repo";

export interface Recipe {
  id: string;
  name: string;
  description: string | null;
  source: RecipeSource;
  confirm: boolean;
  inputs: InputDef[];
}

export interface LoadError {
  file: string;
  message: string;
}

export interface PendingTrust {
  file: string;
  content: string;
  hash: string;
  /** An earlier version was trusted; this one differs. */
  changed: boolean;
}

export interface RecipeList {
  cwd: string | null;
  repoRoot: string | null;
  recipes: Recipe[];
  errors: LoadError[];
  globalFile: string | null;
  pendingTrust: PendingTrust | null;
}

export type InputValues = Record<string, string | boolean>;

export interface PlannedStep {
  command: string;
  always: boolean;
}

export interface FieldError {
  input: string | null;
  message: string;
}

export interface Risk {
  step: number;
  message: string;
}

export interface StageFile {
  path: string;
  code: string;
  suggested: boolean;
  reason: string | null;
}

export interface Plan {
  steps: PlannedStep[];
  errors: FieldError[];
  warnings: string[];
  risks: Risk[];
  dirtySwitch: { step: number; changed: number } | null;
  staging: { step: number; files: StageFile[] } | null;
}

/** Answers from the dialog that the backend re-checks before running. */
export interface RunChoices {
  staged: string[] | null;
  stashFirst: boolean;
  allowDirtySwitch: boolean;
  confirmed: boolean;
}

export type StepState = "pending" | "running" | "ok" | "failed" | "skipped";
export type GitOperation = "merge" | "rebase" | "cherry-pick" | "revert";

export interface RunStarted {
  name: string;
  steps: PlannedStep[];
}

export interface StepEvent {
  index: number;
  state: StepState;
  exitCode: number | null;
  note: string | null;
}

export interface RunDone {
  status: "ok" | "failed" | "cancelled";
  failedStep: number | null;
  exitCode: number | null;
  operation: GitOperation | null;
  /** Steps Resolve → Continue will run after finishing the operation. */
  remaining: number;
  /** Where Abort & go back returns to. */
  startBranch: string | null;
  message: string | null;
}
