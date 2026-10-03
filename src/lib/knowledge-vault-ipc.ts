import { invoke } from "@tauri-apps/api/core";

import { LOCAL_HOST, type HostId } from "./transport";

/** 정리 세션 결과 — 창 label·루트와 스킬 설치 안내(차단이 아니다). `skill`은 실제로 부른 스킬 이름이다. */
export interface VaultSessionOpen { label: string; root: string; skill: string; skill_present: boolean; warning: string | null; }

function local(host: HostId) {
  if (host !== LOCAL_HOST) throw new Error("개인 지식창고는 로컬 세션에서만 사용할 수 있습니다");
}

export const vaultLocalComposerSend = (taskId: number, message: string, expectedClientRef: string, host: HostId = LOCAL_HOST) => { local(host); return invoke("knowledge_vault_local_composer_send", { id: taskId, message, expectedClientRef }); };
