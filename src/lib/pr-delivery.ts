import { invoke } from "@tauri-apps/api/core";

export type PrDeliveryAction = "push" | "create" | "update";

export interface PrDeliveryInput { title?: string; body?: string; }
export interface PrDeliveryCommit { sha: string; subject: string; }
export interface PrDeliveryPreview {
  token: string;
  repository: string;
  branch: string;
  base: string;
  head: string;
  title: string;
  body: string;
  commits: PrDeliveryCommit[];
  verification_summary: string;
  template_path: string | null;
  expires_at: number;
}
export interface PrDeliveryResult {
  action: PrDeliveryAction;
  repository: string;
  branch: string;
  head: string;
  pr_number: number | null;
  url: string | null;
  message: string;
}

export const taskPrPreview = (id: number, input: PrDeliveryInput) =>
  invoke<PrDeliveryPreview>("task_pr_preview", { id, input });

export const taskPrExecute = (id: number, token: string, action: PrDeliveryAction) =>
  invoke<PrDeliveryResult>("task_pr_execute", { id, token, action: { action } });
