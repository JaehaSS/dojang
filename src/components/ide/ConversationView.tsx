import { memo, useCallback, useEffect, useLayoutEffect, useReducer, useRef, useState, type ReactNode } from "react";
import type { SubagentThread } from "../../lib/activity";
import {
  captureScrollAnchor,
  isConversationNearBottom,
  restoreScrollAnchor,
  type ConversationScrollAnchor,
} from "./conversationScroll";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";
import { TranslatableReply, isReplyTranslateShortcut, type ReplyHighlights } from "./MessageTranslation";
import { isMostlyKorean } from "../../lib/translate";
import { SubagentCard } from "./SubagentCard";
import { fmtAge } from "../../lib/usage";
import { useQuizGate } from "../../lib/use-quiz-gate";
import { InsightCard } from "../InsightCard";
import { QuizPanel } from "../QuizPanel";
import { QuizReviewPanel } from "../QuizReviewPanel";
import type { ToolOutputContent } from "../../lib/ipc";
import { normalizeToolOutputContents } from "../../lib/tool-output-media";
import { ToolOutput } from "./ToolOutput";

/** `inherited`는 모든 role에 붙는다 — 이어받기 체인에서 **다른 작업**의 이벤트임을 표시하고,
 *  뷰는 이 플래그로 흐리게 그린다(살짝, 눈에 거슬리지 않게). eventToItems가 원본 이벤트의
 *  `inherited`를 그대로 옮겨 붙인다. */
export type ConvoItem =
  | { role: "user"; text: string; expandedText?: string; inherited?: boolean }
  | { role: "interaction"; interactionId: string; inherited?: boolean }
  | { role: "text"; text: string; streamId?: string; complete?: boolean; parentId?: string; inherited?: boolean }
  | { role: "tool"; name: string; summary: string; toolId?: string; parentId?: string; inherited?: boolean }
  | { role: "tool_result"; summary: string; is_error: boolean; toolUseId?: string; parentId?: string; inherited?: boolean }
  | { role: "tool_output"; contents: ToolOutputContent[]; toolUseId: string; parentId?: string; inherited?: boolean }
  /** 서브 에이전트 실행 모델 관측 — 렌더용이 아니라 카드 헤더 칩의 원천이다. */
  | { role: "subagent_model"; parentId: string; model: string; inherited?: boolean }
  | { role: "meta"; cost: number; turns: number; tokensIn: number; tokensOut: number; inherited?: boolean }
  | { role: "error"; text: string; inherited?: boolean }
  /** 의도적 컨텍스트 절단 지점 · 이어받기 경계 · 이력 절단 안내가 공유하는 구분선. */
  | { role: "divider"; text: string; inherited?: boolean };

/** 대화 이벤트 매퍼의 입력 형태 — 라이브 convo://event(`id` 있음)와 저장 히스토리(`id` 없음, "user" 포함)를
 *  하나로 통일하는 느슨한 구조 타입. eventToItems는 `id`를 안 쓴다. 새 kind 추가 시 여기 + ipc ConvoEvent 갱신. */
export type ConvoEventLike = {
  kind: string;
  item_id?: string;
  interaction_id?: string;
  complete?: boolean;
  text?: string;
  name?: string;
  summary?: string;
  is_error?: boolean;
  cost_usd?: number;
  num_turns?: number;
  tokens_in?: number;
  tokens_out?: number;
  /** context_usage — 원자적 컨텍스트 관측. 뷰 아이템은 만들지 않는다. */
  context_tokens?: number;
  context_window?: number | null;
  observed_at?: number | null;
  source?: string | null;
  valid?: boolean | null;
  /** model_snapshot — 요청/실제 실행 모델. 뷰 아이템은 안 만들고 헤더 CTX %의 분모가 소비. */
  requested?: string;
  resolved?: string;
  /** subagent_model — 서브 에이전트가 실제로 돈 모델. 서브 카드 헤더 칩이 소비. */
  model?: string;
  /** debate_ended — 토론이 끝난 이유. 구분선 문구만 가른다. */
  reason?: string;
  /** 서브 에이전트 상관관계 — tool_use의 id, tool_result의 대응 id, 소속 부모(Task) id. */
  tool_id?: string;
  tool_use_id?: string;
  parent_id?: string;
  contents?: ToolOutputContent[];
  /** 이어받기 체인에서 원본 작업(들)로부터 물려받은 이벤트인가 — `commands.rs::inherited_convo_history`가 박는다.
   *  현재 작업 id로 실행하면 엉뚱한 워크트리를 건드리므로, 이 이벤트에 걸린 파괴적/작업 귀속 동작은 잠가야 한다. */
  inherited?: boolean;
  /** inherited 이벤트가 속한 원본 작업 id. resumed_from 경계 이벤트 자신의 것이기도 하다. */
  source_task_id?: number;
  /** history_truncated — 상한 초과로 잘려나간 이벤트 수. */
  dropped?: number;
  /** resume_external — 세션홈에서 이어받은 벤더 세션의 식별자(앞 8자만, 백엔드가 자른다). */
  session_id?: string;
  /** resume_external — 그 세션이 원래 있던 작업 디렉터리. */
  cwd?: string | null;
  /** resume_external — 그 세션이 마지막으로 쓰인 시각(epoch seconds). */
  last_active?: number | null;
  /** resume_external — 그 세션이 담고 있던 메시지 수. */
  messages?: number | null;
};

/** 턴 가드가 경고 뒤에 에이전트 응답 전문을 붙일 때의 구분자 —
 *  src-tauri/src/convo/turn_guard.rs의 LAST_RESPONSE_MARKER와 문자 단위로 같아야 한다.
 *  전문은 Result.text를 최종 답변으로 읽는 백엔드 소비자를 위한 것이고, 대화 뷰에는 같은
 *  응답이 이미 text 이벤트로 렌더링돼 있으므로 여기서 잘라 중복 노출을 막는다. */
export const GUARD_LAST_RESPONSE_MARKER = "\n\n에이전트의 마지막 응답:\n";

/** 토론 종료 구분선 문구 — 이유마다 다른 것은 문자열뿐이다(설계 0020 §5). */
const DEBATE_END_LABEL: Record<string, string> = {
  consensus: "토론이 합의로 끝났습니다",
  round_cap: "토론이 라운드 상한에서 끝났습니다",
  aborted: "토론을 중단했습니다",
  error: "턴이 실패해 토론이 끝났습니다",
};

/** kind 태그 대화 이벤트 → 대화 뷰 아이템 매핑. 라이브 스트림·저장 히스토리·앙상블 트랜스크립트 공용.
 *  `index`/`all`은 Array.prototype.flatMap이 자동으로 넘겨주는 인자 — 호출부 변경 없이 lookahead 가능.
 *  "user_expanded"는 직전 user 이벤트에 붙는 부가 정보라 단독 아이템을 만들지 않는다(user 처리 시 흡수). */
export const eventToItems = (
  ev: ConvoEventLike,
  index?: number,
  all?: ConvoEventLike[],
): ConvoItem[] => {
  // 승계분 표시는 어느 kind든 공통이라 매핑 본문과 분리한다 — 분기마다 inherited를 챙기면
  // 하나만 빠뜨려도 그 role은 조용히 원본 작업 것처럼 보인다(잠금 대상 판별의 근거를 잃는다).
  const items = mapConvoEvent(ev, index, all);
  return ev.inherited ? items.map((it) => ({ ...it, inherited: true })) : items;
};

const mapConvoEvent = (
  ev: ConvoEventLike,
  index?: number,
  all?: ConvoEventLike[],
): ConvoItem[] => {
  switch (ev.kind) {
    case "resumed_from":
      return [{ role: "divider", text: `여기까지 이어받은 대화 (작업 #${ev.source_task_id ?? "?"})` }];
    case "history_truncated":
      return [{ role: "divider", text: `이전 대화 ${ev.dropped ?? 0}건이 생략되었습니다` }];
    case "resume_no_context":
      // 위 이력은 보이는데 에이전트는 그것을 모른다. 말해 주지 않으면 사용자는 읽은 줄 알고
      // "아까 그거"로 말을 건다.
      return [
        {
          role: "divider",
          text: `작업 #${ev.source_task_id ?? "?"}의 세션이 남아 있지 않아 대화 기록만 이어받았습니다 — 에이전트는 위 내용을 기억하지 못합니다`,
        },
      ];
    case "resume_external": {
      // 세션홈에서 골라 이어받은 벤더 세션 — 어느 세션의, 어디서, 언제까지 쓰인 것을
      // 이어받았는지 밝힌다. session_id는 백엔드가 이미 앞 8자로 잘라 보낸다.
      const parts = [`세션 ${ev.session_id ?? "?"}에서 이어받았습니다`];
      if (ev.cwd) parts.push(`경로 ${ev.cwd}`);
      if (ev.last_active != null) {
        const now = Math.floor(Date.now() / 1000);
        parts.push(`마지막 사용 ${fmtAge(Math.max(0, now - ev.last_active))}`);
      }
      if (ev.messages != null) parts.push(`메시지 ${ev.messages}개`);
      return [{ role: "divider", text: parts.join(" · ") }];
    }
    case "user": {
      const next = index != null ? all?.[index + 1] : undefined;
      const expandedText = next?.kind === "user_expanded" ? next.text : undefined;
      return [{ role: "user", text: ev.text ?? "", expandedText }];
    }
    case "user_expanded":
      return [];
    case "context_cleared":
      return [{ role: "divider", text: ev.text ?? "컨텍스트를 비웠습니다" }];
    case "debate_ended":
      // 경계가 없으면 우측 발화가 "같은 세션이 기억하는 말"로 읽힌다 — 단일 뷰로 돌아가도 남는다.
      return [{ role: "divider", text: DEBATE_END_LABEL[ev.reason ?? ""] ?? "토론이 끝났습니다" }];
    case "interaction":
      return ev.interaction_id ? [{role: "interaction", interactionId: ev.interaction_id}] : [];
    case "text_update":
      return [{role: "text", text: ev.text ?? "", streamId: ev.item_id, complete: ev.complete}];
    case "text":
      return [{ role: "text", text: ev.text ?? "", parentId: ev.parent_id }];
    case "tool_use":
      return [
        {
          role: "tool",
          name: ev.name ?? "",
          summary: ev.summary ?? "",
          toolId: ev.tool_id,
          parentId: ev.parent_id,
        },
      ];
    case "tool_result":
      return [
        {
          role: "tool_result",
          summary: ev.summary ?? "",
          is_error: !!ev.is_error,
          toolUseId: ev.tool_use_id,
          parentId: ev.parent_id,
        },
      ];
    case "tool_output":
      if (!ev.tool_use_id) return [];
      const contents = normalizeToolOutputContents(ev.contents);
      return contents.length ? [{ role: "tool_output", toolUseId: ev.tool_use_id, contents, parentId: ev.parent_id }] : [];
    case "subagent_model":
      // 아이템으로 만들되 대화에 그려지지는 않는다 — 서브 에이전트 폴드가 카드 칩으로 접는다.
      if (!ev.parent_id || !ev.model) return [];
      return [{ role: "subagent_model", parentId: ev.parent_id, model: ev.model }];
    case "result":
      if (ev.is_error) {
        const text = ev.text ?? "";
        const cut = text.indexOf(GUARD_LAST_RESPONSE_MARKER);
        return [{ role: "error", text: cut < 0 ? text : text.slice(0, cut) }];
      }
      if ((ev.cost_usd ?? 0) > 0 || (ev.tokens_out ?? 0) > 0)
        return [
          {
            role: "meta",
            cost: ev.cost_usd ?? 0,
            turns: ev.num_turns ?? 0,
            tokensIn: ev.tokens_in ?? 0,
            tokensOut: ev.tokens_out ?? 0,
          },
        ];
      return [];
    default:
      return [];
  }
};

function SeparateQuestionAction({ text, onAsk }: { text: string; onAsk: (text: string) => void }) {
  return <button
    type="button"
    className="mt-1 rounded px-1 py-0.5 text-xs text-text-secondary hover:bg-raised hover:text-text focus-visible:outline focus-visible:outline-2 focus-visible:outline-primary"
    onMouseDown={(event) => event.preventDefault()}
    onClick={(event) => {
      const selection = window.getSelection();
      const parent = event.currentTarget.parentElement;
      const selected = selection && parent?.contains(selection.anchorNode) && parent.contains(selection.focusNode)
        ? selection.toString().trim() : "";
      onAsk(selected || text);
    }}
  >따로 질문</button>;
}

interface Props {
  renderQuestion?: (id: string) => ReactNode;
  interactionStatus?: ReactNode;
  /** 메인 작업 전환 때 자동 스크롤 상태를 세션별로 격리하는 식별자. */
  conversationId?: number | string;
  items: ConvoItem[];
  busy: boolean;
  subagents?: ReadonlyMap<string, SubagentThread<ConvoItem>>;
  activity?: {
    state: "starting" | "running" | "ended_without_result" | "unknown";
    started_at: number;
    last_event_at: number;
    last_operation: string | null;
    checked_at: number;
  } | null;
  /** 펼친 서브 에이전트 카드에서 전용 탭 열기. */
  onOpenSubagent?: (toolId: string) => void;
  /** 에이전트 답변의 파일/웹 링크 열기. */
  onOpenLink?: (link: string) => void;
  /** 링크 우클릭 — 좌표와 링크 원문을 위로 올린다. 메뉴를 무엇으로 채울지는 App이 정한다. */
  onLinkMenu?: (link: string, at: { x: number; y: number }) => void;
  /** 중앙 diff가 보이는 동안 자동 스크롤을 멈춘다. */
  hidden?: boolean;
  onAskSeparately?: (text: string) => void;
  /**
   * 대기 카드(퀴즈·검수·인사이트)를 이 대화의 흐름 안에 띄울 로컬 작업 id. 없으면 띄우지 않는다 —
   * 앙상블·서브 에이전트 뷰는 같은 컴포넌트를 쓰지만 사람이 기다리는 자리가 아니다.
   */
  waitTaskId?: number | null;
  /** 영어 답변에 `번역` 버튼을 달고 ⌥⌘J를 받는다. 사람이 읽는 메인 대화에서만 켠다. */
  translatable?: boolean;
}

/** 번역을 달 답변인가 — 스트리밍이 끝났고 한국어로 쓰이지 않았다. */
const replyReady = (it: ConvoItem, isLast: boolean, busy: boolean) =>
  it.role === "text" && (it.complete ?? (!busy || !isLast));

interface ConversationScrollState {
  scrollTop: number;
  shouldFollow: boolean;
}

/** user 말풍선 — 슬래시 스킬 확장이 있었으면 "실제 전송문 보기" 토글(기본 접힘)로 확장문을 노출. */
function UserBubble({ text, expandedText }: { text: string; expandedText?: string }) {
  const [expanded, setExpanded] = useState(false);
  return (
    <div className="flex flex-col items-end gap-1">
      <div className="max-w-[80%] rounded-lg bg-primary/15 text-text px-3 py-2 text-md whitespace-pre-wrap break-words">
        {text}
      </div>
      {expandedText && (
        <button
          className="text-[11px] text-text-muted hover:text-text-secondary"
          onClick={() => setExpanded((v) => !v)}
        >
          {expanded ? "실제로 보낸 내용 접기" : "실제로 보낸 내용 보기"}
        </button>
      )}
      {expanded && expandedText && (
        <div className="max-w-[80%] rounded-lg border border-border bg-bg px-3 py-2 text-xs font-code whitespace-pre-wrap break-words text-text-secondary">
          {expandedText}
        </div>
      )}
    </div>
  );
}

/** 구조화 대화 뷰 (Phase 2) — claude stream-json을 파싱한 이벤트를 대화로 렌더.
 *  user 말풍선 + assistant 텍스트 + 툴콜 카드. (마크다운 렌더는 후속: 현재 pre-wrap) */
const fmtTok = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`);

/** 세션 진입 시 한 번에 마운트하는 아이템 상한. 긴 트랜스크립트는 수천 건이라 통째로 올리면
 *  DOM 수천 노드를 만드는 동안 화면이 멈춘다(1020건 → 5899노드). 최신 쪽부터 이만큼만 올리고
 *  위로 거슬러 올라갈 때 STEP씩 확장한다. 확장량은 세션별로 기억해 재진입에도 보던 만큼 남는다. */
const INITIAL_WINDOW = 120;
const WINDOW_STEP = 200;

export const ConversationView = memo(function ConversationView({
  conversationId,
  renderQuestion,
  interactionStatus,
  items,
  busy,
  subagents,
  activity,
  onOpenSubagent,
  onOpenLink,
  onLinkMenu,
  hidden = false,
  onAskSeparately,
  waitTaskId = null,
  translatable = false,
}: Props) {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const shouldFollowRef = useRef(true);
  const activeConversationRef = useRef(conversationId);
  const wasHiddenRef = useRef(false);
  const restoringScrollRef = useRef(false);
  const scrollStateByConversationRef = useRef(
    new Map<number | string, ConversationScrollState>(),
  );
  const waitCardObserverRef = useRef<ResizeObserver | null>(null);
  /** 스크롤할 때마다 잡아 두는 보던 자리. 폭이 바뀐 뒤 이것으로 되돌린다. */
  const scrollAnchorRef = useRef<ConversationScrollAnchor | null>(null);
  /** 보던 자리를 잡았을 때의 폭. 이 값과 다르면 줄바꿈이 이미 바뀐 뒤다. */
  const anchorWidthRef = useRef(0);

  // 렌더 윈도우 — 세션별 확장량은 ref에 남기고 확장은 강제 리렌더로 반영한다.
  // state로 두면 세션 전환마다 초기화 타이밍을 맞춰야 하지만, ref는 키로 읽으므로 그럴 필요가 없다.
  const windowByConversationRef = useRef(new Map<number | string, number>());
  const [, bumpWindow] = useReducer((n: number) => n + 1, 0);
  /** 확장 직전 scrollHeight — 위쪽에 붙은 만큼 scrollTop을 밀어 보던 위치를 고정한다. */
  const expandAnchorRef = useRef<number | null>(null);

  // 펼친 번역 — 전체 기준 절대 인덱스. 세션이 바뀌면 비운다(인덱스는 대화마다 다르다).
  const [openTranslations, setOpenTranslations] = useState<ReadonlySet<number>>(() => new Set());
  useEffect(() => setOpenTranslations(new Set()), [conversationId]);
  const toggleTranslation = useCallback((index: number) => {
    setOpenTranslations((prev) => {
      const next = new Set(prev);
      if (!next.delete(index)) next.add(index);
      return next;
    });
  }, []);
  const translateTargetRef = useRef({ items, busy });
  translateTargetRef.current = { items, busy };
  useEffect(() => {
    if (!translatable || hidden) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || !isReplyTranslateShortcut(e)) return;
      const { items: all, busy: running } = translateTargetRef.current;
      for (let index = all.length - 1; index >= 0; index -= 1) {
        const it = all[index];
        if (it.role !== "text" || it.parentId || isMostlyKorean(it.text)) continue;
        if (!replyReady(it, index === all.length - 1, running)) continue;
        e.preventDefault();
        toggleTranslation(index);
        return;
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [translatable, hidden, toggleTranslation]);

  const windowKey = conversationId ?? "";
  const windowSize = windowByConversationRef.current.get(windowKey) ?? INITIAL_WINDOW;
  const hiddenCount = Math.max(0, items.length - windowSize);
  const visibleItems = hiddenCount === 0 ? items : items.slice(hiddenCount);

  const expandWindow = (): void => {
    const container = scrollContainerRef.current;
    expandAnchorRef.current = container ? container.scrollHeight : null;
    windowByConversationRef.current.set(windowKey, windowSize + WINDOW_STEP);
    bumpWindow();
  };

  // activity는 5초 폴링이 매번 새 객체를 주므로 원시값으로 좁힌다 — 같은 상태의 재조회가
  // 레이아웃 재계산(scrollHeight 읽기)을 유발하지 않게.
  const activityState = activity?.state ?? null;
  const activityOperation = activity?.last_operation ?? null;

  // 대기 카드는 "작업 중" 띠 아래, 대화 흐름 안에 놓인다 — 기다리는 자리와 읽는 자리가 같아야
  // 화면 구석을 따로 살피지 않는다. 이 대화의 턴만 잰다(useQuizGate의 taskId).
  const wait = useQuizGate(waitTaskId != null && busy, waitTaskId);
  const waitMode = waitTaskId == null ? null : wait.mode;

  // 카드 안의 변화("다음", 불러오기 완료, 퀴즈 단계 전환)는 이 컴포넌트를 리렌더하지 않는다 —
  // 아래 layout effect는 waitMode가 바뀔 때만 돈다. 그래서 카드 높이를 따로 지켜보고, 하단을
  // 따라가던 중이면 늘어난 만큼 내려 준다. 안 그러면 새 카드의 아랫부분이 접혀 사용자가 매번 긁어야 한다.
  const waitCardRef = useCallback((node: HTMLDivElement | null) => {
    waitCardObserverRef.current?.disconnect();
    waitCardObserverRef.current = null;
    if (!node || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      const container = scrollContainerRef.current;
      if (!container || !shouldFollowRef.current) return;
      container.scrollTop = container.scrollHeight;
    });
    observer.observe(node);
    waitCardObserverRef.current = observer;
  }, []);

  // 대화 영역 자체의 높이도 버튼 하나로 바뀐다 — 아래 컴포저·대기열·승인 바가 자라면 영역이 줄어드는데,
  // 브라우저는 scrollTop을 그대로 두므로 하단을 보던 사람에게는 마지막 답변이 그만큼 가려져 화면이 위로
  // 튄 것처럼 보인다. 스크롤 이벤트도 나지 않아 follow 판정이 이를 알 길이 없다. 따라가던 중이면 다시 붙인다.
  //
  // 폭도 마찬가지다 — 코드 열이 열리고 닫히면(팝아웃·팝인 포함) 줄바꿈이 달라져 같은 scrollTop이
  // 다른 문단을 가리킨다. WKWebView에는 스크롤 앵커링이 없으므로, 위로 읽어 올라가 있던 중이면
  // 보던 항목을 같은 자리에 다시 놓는다.
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container || typeof ResizeObserver === "undefined") return;
    const observer = new ResizeObserver(() => {
      if (container.clientHeight === 0) return;
      const widthChanged = container.clientWidth !== anchorWidthRef.current;
      anchorWidthRef.current = container.clientWidth;
      if (shouldFollowRef.current) {
        container.scrollTop = container.scrollHeight;
        return;
      }
      if (widthChanged) restoreScrollAnchor(container, scrollAnchorRef.current);
    });
    observer.observe(container);
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;
    if (hidden) {
      const saved = conversationId == null ? undefined : scrollStateByConversationRef.current.get(conversationId);
      if (conversationId != null && !wasHiddenRef.current && !saved) {
        scrollStateByConversationRef.current.set(conversationId, {
          scrollTop: container.scrollTop,
          shouldFollow: shouldFollowRef.current,
        });
      }
      wasHiddenRef.current = true;
      return;
    }
    if (wasHiddenRef.current) {
      wasHiddenRef.current = false;
      const saved = conversationId == null ? undefined : scrollStateByConversationRef.current.get(conversationId);
      if (saved) {
        shouldFollowRef.current = saved.shouldFollow;
        restoringScrollRef.current = true;
        container.scrollTop = saved.scrollTop;
        requestAnimationFrame(() => {
          restoringScrollRef.current = false;
        });
      }
      return;
    }
    // 윈도우 확장 — 늘어난 높이만큼 내려 보던 위치를 유지한다. follow 판정보다 우선.
    const anchor = expandAnchorRef.current;
    if (anchor != null) {
      expandAnchorRef.current = null;
      const grown = container.scrollHeight - anchor;
      if (grown > 0) {
        container.scrollTop += grown;
        return;
      }
    }
    if (activeConversationRef.current !== conversationId) {
      activeConversationRef.current = conversationId;
      // 항목 DOM은 인덱스 key로 재사용된다 — 이전 대화에서 잡은 자리는 이 대화의 것이 아니다.
      scrollAnchorRef.current = null;
      const savedState =
        conversationId == null
          ? undefined
          : scrollStateByConversationRef.current.get(conversationId);
      shouldFollowRef.current = savedState?.shouldFollow ?? true;
    }
    if (shouldFollowRef.current) {
      container.scrollTop = container.scrollHeight;
      return;
    }
    if (conversationId == null) return;
    const savedState = scrollStateByConversationRef.current.get(conversationId);
    if (!savedState) return;
    const maxScrollTop = Math.max(0, container.scrollHeight - container.clientHeight);
    if (maxScrollTop >= savedState.scrollTop) container.scrollTop = savedState.scrollTop;
    // subagents는 items에서 파생되므로 deps에 두지 않는다 — 같은 변화로 두 번 돌 뿐이다.
    // waitMode: 카드가 열리며 아래에 붙은 높이도 따라 내려가야 보인다.
  }, [conversationId, items, busy, windowSize, activityState, activityOperation, hidden, waitMode]);

  const updateFollowState = (): void => {
    if (hidden || restoringScrollRef.current) return;
    const container = scrollContainerRef.current;
    if (!container) return;
    // 폭이 바뀌어 줄바꿈이 끝난 뒤 들어온 스크롤(길이가 줄어 생긴 clamp)은 사용자가 움직인 것이
    // 아니다. 여기서 자리를 다시 잡으면 되돌릴 기준이 사라진다 — ResizeObserver가 먼저 정리한다.
    if (container.clientWidth !== anchorWidthRef.current) return;
    scrollAnchorRef.current = captureScrollAnchor(container);
    const savedState =
      conversationId == null
        ? undefined
        : scrollStateByConversationRef.current.get(conversationId);
    const maxScrollTop = Math.max(0, container.scrollHeight - container.clientHeight);
    // 세션 전환 직후 히스토리가 비는 동안 발생한 강제 clamp는 사용자의 저장 위치가 아니다.
    if (savedState && !savedState.shouldFollow && maxScrollTop < savedState.scrollTop) return;
    const shouldFollow = isConversationNearBottom(container);
    shouldFollowRef.current = shouldFollow;
    if (conversationId == null) return;
    scrollStateByConversationRef.current.set(conversationId, {
      scrollTop: container.scrollTop,
      shouldFollow,
    });
  };

  // 이 대화의 누적 지출 — 턴별 meta(비용/토큰) 합산. 벤더가 준 만큼만(claude=비용+토큰, codex=토큰).
  // 승계분은 빼고 센다. 그 턴들은 원본 작업이 이미 자기 게이지에 계상한 지출이라, 여기 더하면
  // 이어받을 때마다 같은 비용이 한 번씩 더 불어난 값이 화면에 남는다.
  const spend = items.reduce(
    (a, it) =>
      it.role === "meta" && !it.inherited
        ? { cost: a.cost + it.cost, tin: a.tin + it.tokensIn, tout: a.tout + it.tokensOut, turns: a.turns + 1 }
        : a,
    { cost: 0, tin: 0, tout: 0, turns: 0 },
  );
  const hasSpend = spend.turns > 0 && (spend.cost > 0 || spend.tin + spend.tout > 0);

  return (
    <div className="flex flex-1 min-h-0 flex-col">
      {hasSpend && (
        <div className="flex shrink-0 items-center gap-2 border-b border-border bg-bg px-4 py-1.5 text-[11px] font-code text-text-muted">
          <span className="text-text-secondary">누적</span>
          {spend.cost > 0 && <span>${spend.cost.toFixed(3)}</span>}
          {spend.tin + spend.tout > 0 && (
            <span>
              {fmtTok(spend.tin)} → {fmtTok(spend.tout)} tok
            </span>
          )}
          <span>· {spend.turns}턴</span>
        </div>
      )}
      <div
        ref={scrollContainerRef}
        className="min-h-0 flex-1 overflow-y-auto overflow-x-hidden space-y-3 px-4 py-3"
        onScroll={updateFollowState}
      >
        {items.length === 0 && !busy && (
          <div className="text-text-muted text-sm border border-border rounded-md p-4">
            대화를 시작하려면 아래에 입력하세요 — claude 스트리밍(구조화 출력)을 대화·툴 카드로 렌더합니다.
          </div>
        )}
        {hiddenCount > 0 && (
          <div className="flex justify-center">
            <button
              className="text-xs text-text-secondary border border-border rounded-md px-2.5 py-1.5 hover:border-border-strong"
              onClick={expandWindow}
            >
              이전 대화 {hiddenCount}개 더 보기
            </button>
          </div>
        )}
        {visibleItems.map((it, offset) => {
          // key는 전체 기준 절대 인덱스 — 윈도우가 밀려도 남는 아이템의 key가 유지되어
          // 확장·스트리밍이 전체 재마운트가 되지 않는다.
          const i = hiddenCount + offset;
          // 이어받기로 물려받은 이벤트는 **다른 작업의 것**이라 살짝 흐리게 그린다 — 지금 대화와
          // 같은 톤으로 섞이면 사용자가 자기가 한 말/본 결과로 착각한다. 과하게 낮추지 않는다.
          const dimCls = it.inherited ? "opacity-60" : "";
          if (it.role === "interaction") return <div key={i}>{renderQuestion?.(it.interactionId)}</div>;
          if (it.role === "user") {
            return (
              <div key={i} className={`flex flex-col items-end ${dimCls}`.trim()}>
                <UserBubble text={it.text} expandedText={it.expandedText} />
                {onAskSeparately && <SeparateQuestionAction text={it.text} onAsk={onAskSeparately} />}
              </div>
            );
          }
          if (it.role === "tool") {
            const subId =
              (it.name === "Task" || it.name === "Agent") && !it.parentId ? it.toolId : undefined;
            const thread = subId ? subagents?.get(subId) : undefined;
            if (thread) {
              // SubagentCard는 자체 카드라 className을 받지 않는다 — 감싸서 흐림을 입힌다.
              return (
                <div key={i} className={dimCls || undefined}>
                  <SubagentCard
                    thread={thread}
                    parentBusy={busy}
                    onOpenSubagent={onOpenSubagent}
                    onOpenLink={onOpenLink}
                    onLinkMenu={onLinkMenu}
                  />
                </div>
              );
            }
            const openable = subId && onOpenSubagent;
            return (
              <div
                key={i}
                className={`flex items-center gap-2 text-xs text-text-secondary border border-border rounded-md px-2.5 py-1.5 max-w-[85%] ${
                  openable ? "cursor-pointer hover:border-border-strong" : ""
                } ${dimCls}`.trim()}
                onClick={openable ? () => onOpenSubagent(subId) : undefined}
                role={openable ? "button" : undefined}
                title={openable ? "서브 에이전트 열기" : undefined}
              >
                <span className="text-primary-bright shrink-0">
                  <Icon name="plug" size={13} />
                </span>
                <span className="font-code shrink-0">{it.name}</span>
                {it.summary && <span className="font-code text-text-muted truncate">{it.summary}</span>}
                {openable && <span className="ml-auto shrink-0 text-text-muted">열기 ›</span>}
              </div>
            );
          }
          if (it.role === "tool_result") {
            return (
              <div
                key={i}
                className={`max-w-[85%] rounded-md border px-2.5 py-1.5 text-xs font-code whitespace-pre-wrap break-words overflow-y-auto max-h-40 ${
                  it.is_error
                    ? "border-dangerborder text-status-failed"
                    : "border-border text-text-muted bg-bg"
                } ${dimCls}`.trim()}
              >
                {it.summary || "(빈 결과)"}
              </div>
            );
          }
          if (it.role === "tool_output") return <div key={i} className={dimCls || undefined}><ToolOutput contents={it.contents} onOpenLink={onOpenLink} /></div>;
          if (it.role === "meta") {
            // claude는 달러+턴수, codex는 토큰만 제공 — 있는 것을 보여준다.
            const kf = (n: number) => (n >= 1000 ? `${(n / 1000).toFixed(1)}k` : `${n}`);
            return (
              <div key={i} className={`text-[11px] text-text-muted font-code ${dimCls}`.trim()}>
                {it.cost > 0 && (
                  <>
                    · ${it.cost.toFixed(3)} · {it.turns} turn{it.turns === 1 ? "" : "s"}{" "}
                  </>
                )}
                {it.tokensOut > 0 && (
                  <>
                    · {kf(it.tokensIn)} → {kf(it.tokensOut)} tok
                  </>
                )}
              </div>
            );
          }
          if (it.role === "divider") {
            // 위아래 대화가 이어져 보이면 안 된다 — 아래쪽 에이전트는 위를 기억하지 못한다.
            // 이어받기 경계·이력 절단 안내도 같은 자리를 쓴다 — 조용히 사라지면 안 되는 표시다.
            return (
              <div
                key={i}
                className={`flex items-center gap-2 py-2 text-[11px] text-text-muted ${dimCls}`.trim()}
                role="separator"
              >
                <span className="h-px flex-1 bg-border" />
                <span className="shrink-0">{it.text}</span>
                <span className="h-px flex-1 bg-border" />
              </div>
            );
          }
          if (it.role === "error") {
            return (
              <div key={i} className={`text-status-failed text-sm font-code whitespace-pre-wrap break-words ${dimCls}`.trim()}>
                {it.text}
              </div>
            );
          }
          // 카드 칩의 원천일 뿐 그릴 것이 없다. 보통은 parented라 메인 리스트에 오기 전에
          // 서브 스레드로 흡수되지만, 부모 스폰을 못 본 이벤트는 여기까지 흘러든다 —
          // 명시적으로 좁히지 않으면 아래 fallthrough가 그것까지 본문으로 그리려 든다.
          if (it.role === "subagent_model") return null;
          // 스트리밍 중인 것은 마지막 항목뿐이다 — 그것만 아직 자라는 버퍼다.
          const stable = it.complete ?? (!busy || offset < visibleItems.length - 1);
          const renderMarkdown = (source: string, highlights?: ReplyHighlights) => (
            <Markdown
              text={source}
              onOpenLink={onOpenLink}
              onLinkMenu={onLinkMenu}
              stable={stable}
              highlights={highlights?.phrases}
              renderHighlight={highlights?.render}
            />
          );
          const askAction = onAskSeparately && <SeparateQuestionAction text={it.text} onAsk={onAskSeparately} />;
          return (
            <div key={i} className={`max-w-[88%] text-md text-text ${dimCls}`.trim()}>
              {translatable && !isMostlyKorean(it.text) ? (
                <TranslatableReply
                  text={it.text}
                  open={openTranslations.has(i)}
                  ready={replyReady(it, i === items.length - 1, busy)}
                  onToggle={() => toggleTranslation(i)}
                  renderMarkdown={renderMarkdown}
                  actions={askAction}
                />
              ) : (
                <>
                  {renderMarkdown(it.text)}
                  {askAction}
                </>
              )}
            </div>
          );
        })}
        {interactionStatus}
        {busy && (
          <div className="border border-border rounded-md px-2.5 py-2 text-xs text-text-secondary space-y-1">
            <div className="flex items-center gap-2">
              <span className={activity?.state === "running" ? "text-primary-bright" : "animate-pulse text-primary-bright"}>●</span>
              <span>{activity?.state === "running" ? "에이전트 실행 중" : activity?.state === "starting" ? "에이전트 시작 중" : activity?.state === "ended_without_result" ? "로컬 실행 종료 — 응답을 받지 못했습니다" : activity?.state === "unknown" ? "실행 상태 확인 불가" : "에이전트 작업 중…"}</span>
            </div>
            {activity?.last_operation && <div className="font-code text-text-muted truncate">현재: {activity.last_operation}</div>}
          </div>
        )}
        {/* 낼 문제도 검수할 것도 없으면 게이트가 아예 열지 않는다 (이슈 #87).
            응답이 도착해도 닫지 않는다 — 배지로만 알린다(설계 0044 DR-3). 폭은 에이전트 답변과 맞춘다. */}
        {waitMode && (
          <div ref={waitCardRef} className="max-w-[88%]">
            {waitMode === "review" ? (
              <QuizReviewPanel onDone={wait.leaveReview} doneLabel={wait.canReturnToQuiz ? "퀴즈로" : "닫기"} />
            ) : waitMode === "insight" ? (
              // 퀴즈도 복습도 없을 때만 여기 온다 — 능동 회상이 수동 읽기를 이긴다.
              <InsightCard onClose={wait.dismiss} />
            ) : (
              <QuizPanel
                responseArrived={!busy}
                pendingReview={wait.pendingReview}
                onClose={wait.dismiss}
                onReview={wait.showReview}
              />
            )}
          </div>
        )}
      </div>
    </div>
  );
});

/** Replace a streaming message in place; append only its first snapshot. */
export function appendConvoEvent(items: ConvoItem[], ev: ConvoEventLike): ConvoItem[] {
  const additions=eventToItems(ev);
  if (ev.kind !== "text_update" || !ev.item_id) return [...items,...additions];
  const index=items.findIndex((item)=>item.role === "text" && item.streamId === ev.item_id);
  if (index<0) return [...items,...additions];
  return items.map((item,i)=>i===index?additions[0]:item);
}
export function coalesceConvoEvent<T extends ConvoEventLike>(events:T[],ev:T):T[]{
  if(ev.kind!=="text_update"||!ev.item_id)return [...events,ev];
  const index=events.findIndex((item)=>item.kind==="text_update"&&item.item_id===ev.item_id);
  return index<0?[...events,ev]:events.map((item,i)=>i===index?ev:item);
}
