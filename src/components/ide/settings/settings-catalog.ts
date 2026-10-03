/**
 * 설정 항목 메타의 유일한 원천 — 탭 귀속·검색·위험 배지가 전부 여기서 나온다.
 *
 * 화면과 카탈로그를 따로 두면 반드시 어긋난다(원장 #363: 같은 형식을 두 파서가 읽으면
 * 갈라진다). 그래서 렌더된 섹션·행은 `data-setting-id`로 여기 있는 id를 달고,
 * 테스트가 그 대응을 강제한다.
 */

/** 설정 탭 — 퀵오픈이 특정 탭으로 바로 열 수 있게 이름을 밖에 준다. */
export type SettingsTab =
  | "appearance"
  | "editor"
  | "run"
  | "connection"
  | "notifications"
  | "mcp"
  | "skills"
  | "knowledge"
  | "memory"
  | "mobile"
  | "schedules";

/** 탭 묶음 — `settings`는 값을 바꾸는 곳, `manage`는 환경을 확인·운영하는 곳(ADR 0191). */
export type SettingsTabGroup = "settings" | "manage";

export interface SettingsTabSpec {
  key: SettingsTab;
  label: string;
  group: SettingsTabGroup;
  /** 탭 옆에 다는 짧은 배지 — "베타"처럼 상태를 알리되 라벨 자체를 바꾸지 않는다. */
  badge?: string;
}

export const SETTINGS_TABS: SettingsTabSpec[] = [
  { key: "appearance", label: "모양새", group: "settings" },
  { key: "editor", label: "에디터", group: "settings" },
  { key: "run", label: "작업 실행", group: "settings" },
  { key: "connection", label: "연결", group: "settings" },
  { key: "notifications", label: "알림·음성", group: "settings" },
  { key: "mcp", label: "MCP 서버", group: "manage" },
  { key: "skills", label: "스킬", group: "manage" },
  { key: "knowledge", label: "지식", group: "manage" },
  { key: "memory", label: "메모리", group: "manage" },
  { key: "mobile", label: "모바일", group: "manage", badge: "베타" },
  { key: "schedules", label: "스케줄", group: "manage" },
];

export const DEFAULT_SETTINGS_TAB: SettingsTab = "appearance";

/**
 * 위험 등급 — 색만으로 구분하지 않고 글자를 함께 쓴다(DESIGN.md).
 * `cost`: 켜거나 올리면 벤더 호출·요금이 는다. `safety`: 끄면 보호가 사라진다.
 */
export type SettingRisk = "cost" | "safety";

export interface SettingEntry {
  /** 화면의 `data-setting-id`와 같은 값. 검색 결과가 이것으로 스크롤한다. */
  id: string;
  label: string;
  tab: SettingsTab;
  /** 검색에서 읽는 한 줄 설명. 화면 설명과 같을 필요는 없다. */
  hint?: string;
  /** 라벨에 없는 말로도 찾게 하는 별칭 — 영문 키·옛 이름·동의어. */
  keywords?: string[];
  risk?: SettingRisk;
}

export const SETTINGS_CATALOG: SettingEntry[] = [
  // ── 모양새 ──────────────────────────────────────────────
  {
    id: "theme",
    label: "테마",
    tab: "appearance",
    hint: "UI·에디터·터미널 색을 한 번에 바꾼다",
    keywords: ["theme", "색", "팔레트", "다크", "라이트", "dark", "light"],
  },
  {
    id: "font",
    label: "폰트",
    tab: "appearance",
    hint: "코드 폰트와 UI 폰트, 크기",
    keywords: ["font", "글꼴", "글자 크기", "monospace"],
  },
  {
    id: "font-code",
    label: "코드 폰트",
    tab: "appearance",
    hint: "에디터·터미널이 쓰는 고정폭 글꼴과 크기",
    keywords: ["code font", "monospace", "고정폭"],
  },
  {
    id: "font-ui",
    label: "UI 폰트",
    tab: "appearance",
    hint: "앱 전체가 쓰는 글꼴과 크기",
    keywords: ["ui font", "인터페이스"],
  },
  // ── 에디터 ──────────────────────────────────────────────
  {
    id: "editor-tree",
    label: "파일 트리",
    tab: "editor",
    hint: "트리 글자 크기 등 파일 트리 치수",
    keywords: ["tree", "사이드바", "파일 목록"],
  },
  {
    id: "editor-code",
    label: "코드 편집",
    tab: "editor",
    hint: "미니맵·자동 줄바꿈·탭 크기",
    keywords: ["monaco", "minimap", "미니맵", "word wrap", "줄바꿈", "탭 크기"],
  },

  // ── 작업 실행 ───────────────────────────────────────────
  {
    id: "agent-models",
    label: "에이전트 모델",
    tab: "run",
    hint: "작업·대화 실행 시 CLI에 넘기는 --model",
    keywords: ["model", "claude", "codex", "gemini", "opus", "sonnet"],
    risk: "cost",
  },
  {
    id: "debate-round-cap",
    label: "토론 라운드 상한",
    tab: "run",
    hint: "한 발화가 여는 라운드 수의 상한",
    keywords: ["debate", "토론", "라운드", "round"],
    risk: "cost",
  },
  {
    id: "max-concurrent",
    label: "동시 실행 세션 수",
    tab: "run",
    hint: "한 번에 진행할 수 있는 작업 수",
    keywords: ["concurrency", "동시", "병렬", "세션"],
    risk: "cost",
  },
  {
    id: "task-defaults",
    label: "작업 생성 기본값",
    tab: "run",
    hint: "새 작업을 만들 때 적용되는 값 묶음",
    keywords: ["기본값", "default", "작업 생성"],
  },
  {
    id: "run-environment",
    label: "실행 환경",
    tab: "run",
    hint: "에이전트 프로세스가 도는 조건",
    keywords: ["환경", "environment", "프로세스"],
  },
  {
    id: "session-style",
    label: "세션 방식",
    tab: "run",
    hint: "새 작업을 대화로 할지 터미널(CLI 그대로)로 할지",
    keywords: ["terminal", "터미널", "대화", "conversation", "cli", "컴포저", "session"],
  },
  {
    id: "use-worktree",
    label: "워크트리 격리 (기본값)",
    tab: "run",
    hint: "끄면 새 작업이 메인 체크아웃에서 직접 실행된다",
    keywords: ["worktree", "격리", "isolation"],
    risk: "safety",
  },
  {
    id: "refresh-base",
    label: "base 브랜치 최신화 (기본값)",
    tab: "run",
    hint: "워크트리를 만들기 전에 base를 원격 최신으로 맞춘다",
    keywords: ["base", "브랜치", "fetch", "fast-forward"],
  },
  {
    id: "lsp-autoinject",
    label: "LSP 자동 연결",
    tab: "run",
    hint: "워크트리 언어를 감지해 LSP-MCP 브리지를 자동 추가한다",
    keywords: ["lsp", "language server", "rust-analyzer", "mcp"],
  },
  {
    id: "block-unverified",
    label: "검증 실패 시 Approve 차단",
    tab: "run",
    hint: "Verify 통과 전에는 Approve & Merge를 막는다",
    keywords: ["approve", "verify", "merge", "차단", "게이트"],
    risk: "safety",
  },
  {
    id: "default-shell",
    label: "기본 셸",
    tab: "run",
    hint: "에이전트 PTY 실행 셸 (읽기 전용)",
    keywords: ["shell", "zsh", "bash", "pty", "터미널"],
  },
  {
    id: "capture-profile",
    label: "캡처·회고 실행 프로파일",
    tab: "run",
    hint: "캡처·회고 호출이 쓰는 모델·effort·린 인보케이션",
    keywords: ["capture", "reflect", "회고", "추출", "effort", "lean"],
    risk: "cost",
  },
  {
    id: "english-assist",
    label: "영어로 일하기 보조",
    tab: "run",
    hint: "입력창 ⌘J 영어 프롬프트, 답변 번역 ⌥⌘J, 표현 짚기, 번역 모델",
    keywords: ["translate", "번역", "영어", "english", "프롬프트", "⌘J", "표현", "단어장", "vocab"],
    risk: "cost",
  },

  // ── 연결 ────────────────────────────────────────────────
  {
    id: "agent-cli",
    label: "에이전트 CLI",
    tab: "connection",
    hint: "CLI 인증·버전·사용량 토큰",
    keywords: ["cli", "인증", "auth", "버전", "version", "usage", "사용량"],
  },

  // ── 알림·음성 ───────────────────────────────────────────
  {
    id: "os-notifications",
    label: "OS 알림",
    tab: "notifications",
    hint: "시스템 알림 권한과 테스트 알림",
    keywords: ["notification", "알림", "권한", "permission"],
  },
  {
    id: "voice",
    label: "음성 입력",
    tab: "notifications",
    hint: "push-to-talk 핫키·STT 서버·커맨드와 받아쓰기",
    keywords: ["voice", "stt", "받아쓰기", "핫키", "hotkey", "ohr", "전사"],
  },

  // ── 모바일(관리) ────────────────────────────────────────
  {
    id: "mobile-pairing",
    label: "기기 페어링",
    tab: "mobile",
    hint: "폰을 이 Mac에 연결한다",
    keywords: ["mobile", "폰", "qr", "pairing", "페어링", "pwa"],
  },

  // ── 고급 ────────────────────────────────────────────────
  {
    id: "advanced-features",
    label: "고급·실험 기능 표시",
    tab: "appearance",
    hint: "캐시 관측치·앙상블 실험 지표·Design Mode 같은 개발자용·실험 기능을 노출한다",
    keywords: ["experimental", "실험", "고급", "developer", "개발자", "advanced"],
  },
];

const norm = (value: string) => value.toLowerCase().replace(/\s+/g, "");

/**
 * 검색 — 라벨 선두 일치가 가장 위, 그다음 라벨 포함 · 키워드 · 설명 순.
 * 동점은 카탈로그 순서를 따른다(사람이 정한 순서가 무작위보다 낫다).
 */
export function searchSettings(query: string, limit = 8): SettingEntry[] {
  const q = norm(query);
  if (!q) return [];
  const scored: { entry: SettingEntry; score: number; index: number }[] = [];
  SETTINGS_CATALOG.forEach((entry, index) => {
    const label = norm(entry.label);
    let score: number | null = null;
    if (label.startsWith(q)) score = 0;
    else if (label.includes(q)) score = 1;
    else if ((entry.keywords ?? []).some((k) => norm(k).includes(q))) score = 2;
    else if (entry.hint && norm(entry.hint).includes(q)) score = 3;
    if (score !== null) scored.push({ entry, score, index });
  });
  scored.sort((a, b) => a.score - b.score || a.index - b.index);
  return scored.slice(0, limit).map((s) => s.entry);
}

export function tabLabel(key: SettingsTab): string {
  return SETTINGS_TABS.find((t) => t.key === key)?.label ?? key;
}
