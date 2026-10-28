import { invoke } from "@tauri-apps/api/core";
export type ReviewArtifact = {
  id: string;
  label: string;
  kind: "image" | "video" | "html" | "url";
  storage: "synced" | "local" | "url";
  path: string;
  poster?: string;
};
export type ReviewPin = {
  x: number; y: number;
  artifact?: string;
  timestamp?: number;
  viewport_width?: number; viewport_height?: number;
  scroll_x?: number; scroll_y?: number;
  note?: string;
};
export type ReviewVerdict = {
  verdict: string; choices: string[]; ranking: string[];
  pins: ReviewPin[]; note: string; resolved_at: string;
};
export type ReviewRound = {
  number: number; mode: "pick" | "approve" | "rank" | "markup";
  judge: string; criteria: string[];
  artifacts: ReviewArtifact[]; references: ReviewArtifact[];
  previous?: ReviewArtifact; top_k: number;
  requested_at: string; result?: ReviewVerdict;
};
export type ReviewManifest = {
  schema: number; task_id: string; status: string;
  current_round: number; rounds: ReviewRound[];
  implemented_by?: string; openrig_gate?: string;
  revision: string;
};
export type ReviewResolution = {
  task_id: string; verdict: string; choices: string[];
  ranking: string[]; pins: ReviewPin[]; note: string;
  expected_task_revision: string; expected_manifest_revision: string;
};
export const reviewShow = (taskId: string, path: string) =>
  invoke<ReviewManifest>("review_show", { path, taskId });
export const reviewResolve = (resolution: ReviewResolution, path: string) =>
  invoke<ReviewManifest>("review_resolve", { path, resolution });
export const reviewAsset = (assetPath: string, path: string) =>
  invoke<string>("review_asset", { path, assetPath });
