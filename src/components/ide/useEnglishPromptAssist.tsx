import { useCallback, useEffect, useRef, useState } from "react";
import { hasHangul, studyExpressionsOn, translateText, useTranslateSettings } from "../../lib/translate";
import { ExpressionList, HighlightedText, useExpressions, useVisibleExpressions } from "./ExpressionStudy";

/** 입력창 ⌘J — 한글 IME 중에는 `e.key`가 자모로 오므로 물리 키(`e.code`)로 본다. */
export const isPromptAssistShortcut = (e: { metaKey: boolean; ctrlKey: boolean; altKey: boolean; shiftKey: boolean; code: string }) =>
  (e.metaKey || e.ctrlKey) && !e.altKey && !e.shiftKey && e.code === "KeyJ";

export type PromptAssistState =
  | { kind: "idle" }
  | { kind: "pending" }
  /** 참고 모드 — 영어 제안을 보여 주고 사용자가 직접 친다. */
  | { kind: "suggestion"; english: string }
  /** 바꿔 넣기 모드 — 초안을 영어로 바꿨다. 원문은 되돌리기(⌘Z)와 이 패널에 남는다. */
  | { kind: "replaced"; source: string; english: string }
  | { kind: "notice"; message: string }
  | { kind: "error"; message: string };

const IDLE: PromptAssistState = { kind: "idle" };

interface Options {
  value: string;
  onChange: (v: string) => void;
  /** 초안이 사는 세션 좌표. 바뀌면 진행 중인 요청의 결과를 버린다. */
  draftKey: string | null;
}

/**
 * 한국어 초안을 영어 프롬프트로 — 결과는 설정(`composer_mode`)에 따라 제안으로 보여 주거나
 * 초안을 바꿔 넣는다. 어느 쪽도 보내지 않는다.
 *
 * 결과는 늦게 온다(실측 약 3초). 그 사이 세션을 바꿨거나 초안을 고쳤으면 버린다 —
 * 다른 세션의 초안에 영어가 들어가거나, 방금 친 글자가 덮이면 안 된다.
 */
export function useEnglishPromptAssist({ value, onChange, draftKey }: Options) {
  const { composer_mode: mode } = useTranslateSettings();
  const [state, setState] = useState<PromptAssistState>(IDLE);
  const requestRef = useRef(0);
  const valueRef = useRef(value);
  const keyRef = useRef(draftKey);
  valueRef.current = value;
  keyRef.current = draftKey;

  const clear = useCallback(() => {
    requestRef.current += 1;
    setState(IDLE);
  }, []);

  useEffect(clear, [draftKey, clear]);

  // 알림·오류는 다음 편집에서 걷는다. 바꿔 넣은 원문은 초안을 다 지웠을 때 걷는다.
  useEffect(() => {
    setState((s) =>
      s.kind === "notice" || s.kind === "error" || (s.kind === "replaced" && value === "") ? IDLE : s,
    );
  }, [value]);

  const run = () => {
    if (state.kind === "pending") {
      clear();
      return;
    }
    const source = value;
    if (!source.trim()) return;
    if (!hasHangul(source)) {
      setState({ kind: "notice", message: "한국어가 없어 바꾸지 않았습니다" });
      return;
    }
    const id = ++requestRef.current;
    const key = draftKey;
    const requestedMode = mode;
    setState({ kind: "pending" });
    const stale = () => id !== requestRef.current || keyRef.current !== key;
    translateText("ko_to_en_prompt", source).then(
      (english) => {
        if (stale()) return;
        if (requestedMode === "reference") {
          setState({ kind: "suggestion", english });
          return;
        }
        if (valueRef.current !== source) {
          setState({ kind: "notice", message: "바꾸는 동안 초안이 달라져 적용하지 않았습니다" });
          return;
        }
        onChange(english);
        setState({ kind: "replaced", source, english });
      },
      (error: unknown) => {
        if (stale()) return;
        setState({ kind: "error", message: String(error) });
      },
    );
  };

  /** 처리했으면 true — 입력창의 나머지 키 처리를 건너뛴다. */
  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>): boolean => {
    if (!isPromptAssistShortcut(e)) return false;
    e.preventDefault();
    if (e.nativeEvent.isComposing || e.nativeEvent.keyCode === 229) return true;
    run();
    return true;
  };

  return { state, mode, onKeyDown, clear };
}

/** 입력창 위 패널 — 진행·제안·원문·오류를 한 자리에서 보여 준다. */
export function EnglishPromptAssistPanel({ state, onDismiss }: { state: PromptAssistState; onDismiss: () => void }) {
  // 영어 결과가 나오면 공부할 표현을 따로 짚는다. 바꿔 넣기 결과는 입력창 안이라 밑줄을 못 그으니 목록으로만 보인다.
  const english = state.kind === "suggestion" || state.kind === "replaced" ? state.english : null;
  const study = studyExpressionsOn(useTranslateSettings());
  const expressions = useVisibleExpressions(useExpressions(english, study));
  if (state.kind === "idle") return null;
  const close = (
    <button
      type="button"
      aria-label="닫기"
      className="shrink-0 rounded px-1 text-text-muted hover:bg-raised hover:text-text"
      onClick={onDismiss}
    >
      ✕
    </button>
  );
  if (state.kind === "pending") {
    return (
      <div role="status" className="mb-1 flex items-center gap-2 text-xs text-text-secondary">
        <span className="animate-pulse text-primary-bright">●</span>
        <span>영어로 바꾸는 중… (⌘J 다시 누르면 취소)</span>
      </div>
    );
  }
  if (state.kind === "notice" || state.kind === "error") {
    return (
      <div
        role={state.kind === "error" ? "alert" : "status"}
        className={`mb-1 flex items-start gap-2 text-xs ${state.kind === "error" ? "text-status-failed" : "text-text-secondary"}`}
      >
        <span className="min-w-0 flex-1 break-words">
          {state.kind === "error" ? `영어로 바꾸지 못했습니다: ${state.message}` : state.message}
        </span>
        {close}
      </div>
    );
  }
  if (state.kind === "suggestion") {
    return (
      <div aria-label="영어 제안" className="mb-1 flex items-start gap-2 rounded-md border border-border bg-raised px-3 py-2">
        <div className="min-w-0 flex-1">
          <div className="mb-0.5 text-[11px] text-text-muted">영어로는 이렇게 — 보고 직접 입력해 보세요</div>
          <div className="select-text whitespace-pre-wrap break-words text-sm text-text">
            <HighlightedText text={state.english} expressions={expressions} />
          </div>
          <ExpressionList expressions={expressions} context={state.english} />
        </div>
        {close}
      </div>
    );
  }
  return (
    <div className="mb-1">
      <details aria-label="한국어 원문" className="rounded-md border border-border px-3 py-1.5 text-xs text-text-secondary">
        <summary className="flex cursor-pointer items-center gap-2">
          <span className="flex-1">한국어 원문 보기 · ⌘Z로 되돌리기</span>
          {close}
        </summary>
        <div className="mt-1 whitespace-pre-wrap break-words text-text">{state.source}</div>
      </details>
      <ExpressionList expressions={expressions} context={state.english} />
    </div>
  );
}
