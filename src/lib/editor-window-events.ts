/** 에디터 창과 메인 창이 주고받는 말. 보조 창 진입 판정을 담당한다.
 *
 * 창이 갈리면 JS 컨텍스트가 갈린다 — `designmode/store.ts`·`composer-focus.ts`의
 * in-memory pub-sub은 경계를 넘지 못하므로 창 사이는 전부 Tauri 이벤트로 오간다.
 */

import type { DesignCaptureRecord } from "./designmode/types";
import type { InboxItem } from "./notifications";

/** 창은 앱 전체에 하나다(PRD §5.5). label이 상수이므로 capability도 정적 파일 하나면 된다. */
export const EDITOR_WINDOW_LABEL = "editor";

export const EDITOR_SESSION_EVENT = "editor://session-changed"; // 메인 → 에디터
export const EDITOR_ASK_EVENT = "editor://ask"; // 에디터 → 메인
export const EDITOR_STATUS_EVENT = "editor://status"; // 메인 → 에디터
export const EDITOR_CLOSED_EVENT = "editor://closed"; // 에디터 → 메인 (팝인)
export const EDITOR_READY_EVENT = "editor://ready"; // 에디터 → 메인 (초기 상태 요청)
/** Root-bound project editor asks main for the active theme using its dynamic window label. */
export const PROJECT_EDITOR_READY_EVENT = "project-editor://ready";
/** Project editor window closed. Carries the display root so the main window can rescan it. */
export const PROJECT_EDITOR_CLOSED_EVENT = "project-editor://closed";
export const EDITOR_AUTOSAVED_EVENT = "editor://autosaved"; // 에디터 → 메인 (되돌릴 지점)
export const EDITOR_AUTOSAVE_BLOCKED_EVENT = "editor://autosave-blocked"; // 에디터 → 메인
export const EDITOR_REVERTED_EVENT = "editor://reverted"; // 메인 → 에디터 (되돌렸으니 다시 읽어라)
export const EDITOR_CAPTURE_EVENT = "editor://capture"; // 에디터 → 메인 (⌘L 선택 첨부)
export const EDITOR_REVEAL_EVENT = "editor://reveal"; // 메인 → 에디터 (링크 클릭 배달)
/** 창이 접힌 것이 아니라 **사라졌다** — Rust(`WindowEvent::Destroyed`) 또는 세션 없는 창의 닫기가 보낸다.
 *  `editor://closed`와 달리 파일 목록이 없다. 메인 창은 팝아웃 상태를 풀고 DB에 남은 목록으로 복원한다. */
export const EDITOR_GONE_EVENT = "editor://gone"; // 에디터·Rust → 메인

/** 창이 어느 세션에 붙는지. 갈아탈 때마다 새로 온다. */
export interface EditorSessionPayload {
  task_id: number;
  /** 작업이 사는 호스트. 창은 작업 목록을 갖지 않으므로 메인 창이 실어 보낸다 (ADR 0133). */
  host: string;
  /** 창 제목·상태바 표시용. 없으면 세션 번호만 보인다. */
  branch: string | null;
  /** 문서 링크의 절대 경로를 루트 기준으로 풀 때 쓴다. 창은 작업 목록이 없으므로 메인 창이 실어 보낸다. */
  worktree_path: string | null;
  /** 팝아웃 시점에 메인 창이 열어 두었던 파일 — 창이 이어받는다. */
  open_paths: string[];
  active_path: string | null;
  /** 원격 워크트리는 언어 서버를 띄울 수 없다(`EditorPane.tsx:57`). false면 ⌘B를 걸지 않는다. */
  supports_lsp: boolean;
}

/** 메인 창에서 클릭한 파일 링크. 창이 나가 있는 동안 대화의 링크는 전부 이리로 온다.
 *
 * `task_id`는 목적지 검증용이다 — 저장이 막혀 창이 옛 세션에 머무는 구간이 있고(`EDITOR_AUTOSAVE_BLOCKED_EVENT`),
 * 그때는 같은 상대 경로가 다른 워크트리의 다른 파일을 가리킨다. */
export interface EditorRevealPayload {
  task_id: number;
  host: string;
  path: string;
  /** `src/App.tsx:1187` 표기로 온 링크의 착지 줄. null이면 파일만 연다. */
  line: number | null;
  column: number | null;
}

/** 버블이 보내는 질문. 선택 코드와 질문 텍스트를 함께 싣는다. */
export interface EditorAskPayload {
  task_id: number;
  file_path: string;
  start_line: number;
  end_line: number;
  selection_text: string;
  question: string;
}

/** ⌘L 선택 첨부 — 캡처 store는 창마다 따로이므로(파일 헤더 주석) 레코드째 배달한다.
 *  목적지 세션은 record 안의 `task_id`다. id는 발신 창이 `scopeLocalCaptureId`로 스코프를 섞어
 *  보내므로 두 창의 seq가 겹쳐도 충돌하지 않는다. */
export type EditorCapturePayload = DesignCaptureRecord;

/** 본문이 아니라 진행 여부만 건넨다 — 대화 렌더링을 두 번째 창에 이식하지 않기 위한 경계. */
export type EditorStatus = "idle" | "busy" | "done" | "error";

/** 팝인 시 메인 창이 이어받을 상태.
 *
 * **정상 닫기에서만 온다.** 크래시·강제 종료·메인 창 선종료에서는 오지 않으므로,
 * 파일 목록의 지속성은 이 이벤트가 아니라 DB 저장이 책임진다. */
export interface EditorClosedPayload {
  task_id: number;
  host: string;
  open_paths: string[];
  active_path: string | null;
  notification?: EditorNotificationPayload;
}

export type EditorNotificationPayload = {
  action: "result" | "changes";
  item: InboxItem;
  request_id: string;
};

/** 자동 저장 한 건의 되돌릴 지점. `content`가 null이면 파일이 커서 버퍼를 잡지 않았다. */
export interface AutosaveUndoEntry {
  path: string;
  content: string | null;
}

/** 조용히 디스크에 썼음을 알린다 — 메인 창은 이것으로 되돌리기 바를 띄운다. */
export interface EditorAutosavedPayload {
  task_id: number;
  entries: AutosaveUndoEntry[];
}

/**
 * 저장하지 못해 세션을 갈아타지 못했다.
 *
 * `conflict`는 에이전트가 같은 파일을 고쳤다는 뜻이고, `failed`는 쓰기 자체가 실패한 것이다
 * (원격 워크트리에서 흔하다). 어느 쪽이든 창은 이전 세션에 머문다.
 */
export interface EditorAutosaveBlockedPayload {
  task_id: number;
  path: string;
  reason: "conflict" | "failed";
  detail: string | null;
}

/** `main.tsx` 진입 분기. 접두사만 같은 값을 통과시키면 다른 창이 에디터로 열린다. */
export function isEditorEntry(search: string): boolean {
  return new URLSearchParams(search).get("window") === EDITOR_WINDOW_LABEL;
}

export function isProjectEditorEntry(search: string): boolean {
  return new URLSearchParams(search).get("window") === "project-editor";
}
