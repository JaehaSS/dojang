import { invoke } from "@tauri-apps/api/core";
export interface ProviderFeature { key: string; state: "supported" | "unsupported" | "unknown"; enabled: boolean | null; reason: string; }
export interface TaskObservation {
  task_id: number; revision: string; checked_at: number; task_state: string;
  status: "running" | "queued" | "input_needed" | "review_needed" | "remaining_work" | "ended" | "unknown";
  registered_execution: boolean; cleanup_required: boolean; pending_questions: number; review_processes: number;
  side_questions: number; deliveries: number; approvals: number; child_processes: number | null;
  capabilities: { agent: string; route: string; features: ProviderFeature[] };
}
export const taskObservationGet = (id: number) => invoke<TaskObservation>("task_observation_get", { id });
export const observationLabel: Record<TaskObservation["status"], string> = {
  running: "실행 중", queued: "실행 대기", input_needed: "사용자 입력 필요", review_needed: "결과 검토 필요",
  remaining_work: "잔여 작업 존재", ended: "작업 기록 종결", unknown: "관측 불가",
};
