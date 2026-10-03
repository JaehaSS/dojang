import type { View } from "../components/ide/Sidebar";

/**
 * 음성 커맨드가 UI 에 요구하는 동작. 3단계(에이전트 화면 조작)에서 같은 타입을
 * 커맨드 버스로 노출한다 — 그래서 실행기(dispatchUiAction)를 한 곳에 둔다.
 */
export type VoiceAction =
  | { type: "view"; view: View }
  | { type: "newTask" }
  | { type: "submit" }
  | { type: "clear" };

/**
 * 별칭 뒤에 붙어도 의미를 바꾸지 않는 꼬리말 — 조사·서술어·"화면" 같은 군더더기.
 * 이걸 허용 목록으로 두는 이유는 부분 문자열 매칭을 막기 위해서다.
 * "위키 문서를 정리해 줘"는 커맨드가 아니라 딕테이션 대상이다.
 */
const TAIL = /^(?:(?:으?로)?(?:가|이동|열|열어|보여|켜|띄워)?(?:줘|자|라|요|주세요)?|화면|탭|창|모드)$/;

const COMMANDS: { aliases: string[]; action: VoiceAction }[] = [
  { aliases: ["새작업", "새로운작업", "새태스크", "작업추가"], action: { type: "newTask" } },
  { aliases: ["전송", "제출", "보내", "실행", "submit"], action: { type: "submit" } },
  { aliases: ["취소", "지워", "비워", "클리어", "clear"], action: { type: "clear" } },
];

const VIEWS: { aliases: string[]; view: View }[] = [
  { aliases: ["홈", "home"], view: "home" },
  { aliases: ["워크스페이스", "작업공간", "작업", "workspace"], view: "workspace" },
  { aliases: ["앙상블", "ensemble"], view: "ensemble" },
  { aliases: ["인사이트", "통계", "insights"], view: "insights" },
  { aliases: ["단어장", "vocab"], view: "vocab" },
  { aliases: ["환경설정", "설정", "settings"], view: "settings" },
];

/**
 * 전사 텍스트 정규화 — 소문자, 문장부호 제거, 공백 전부 제거.
 * 공백을 지우는 이유는 STT 가 "새 작업"과 "새작업"을 임의로 갈라 놓기 때문이다.
 */
const normalize = (text: string): string =>
  text
    .toLowerCase()
    .replace(/[\p{P}\p{S}]/gu, "")
    .replace(/\s+/gu, "");

/** 별칭으로 시작하고 나머지가 꼬리말뿐이면 매칭이다. 긴 별칭을 먼저 본다. */
const matchAlias = (normalized: string, aliases: string[]): boolean =>
  [...aliases]
    .sort((a, b) => b.length - a.length)
    .some((alias) => normalized.startsWith(alias) && TAIL.test(normalized.slice(alias.length)));

/**
 * 전사 텍스트를 UI 액션으로 해석한다. 매칭되지 않으면 `null` —
 * 오인식으로 화면이 튀는 것보다 아무것도 안 하는 편이 낫다(설계 0043 §Business Rules).
 */
export function routeTranscript(text: string): VoiceAction | null {
  const normalized = normalize(text);
  if (!normalized) return null;

  // 커맨드가 먼저다. "새 작업"이 "작업"(workspace)으로 흡수되면 새 작업을 만들 길이 없어진다.
  for (const { aliases, action } of COMMANDS) {
    if (matchAlias(normalized, aliases)) return action;
  }
  for (const { aliases, view } of VIEWS) {
    if (matchAlias(normalized, aliases)) return { type: "view", view };
  }
  return null;
}

/** HUD 에 "무엇이 실행됐는지" 한 마디로 보여줄 이름. */
export function actionLabel(action: VoiceAction): string {
  switch (action.type) {
    case "view":
      return `${VIEWS.find((v) => v.view === action.view)?.aliases[0] ?? action.view} 화면`;
    case "newTask":
      return "새 작업";
    case "submit":
      return "전송";
    case "clear":
      return "입력 비움";
  }
}
