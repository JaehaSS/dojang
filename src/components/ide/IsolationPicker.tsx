import { useEffect, useRef, useState } from "react";
import { Icon } from "./icons";
import {
  isolationOptions,
  type IsolationChoice,
} from "./new-task-isolation";

interface Props {
  choice: IsolationChoice;
  /** 전역 기본값 — "기본을 따름"이 실제로 무엇인지 보여주기 위해 필요하다. */
  globalDefault: boolean;
  onPick: (choice: IsolationChoice) => void;
}

/**
 * 격리 선택 칩 — 다른 컴포저 칩(레포·에이전트·모델·Effort)과 같은 칩+팝오버 문법이다.
 *
 * **순환 칩을 대체한다.** 전에는 눌러서 3-state를 도는 버튼이었고 문제가 둘이었다.
 * 하나, 다음 상태를 예측할 수 없다 — 순환은 `DESIGN.md`가 규정한 SelectChip 패턴이 아니다.
 * 둘, **삭제를 말하지 않는다** — 격리 실행의 워크트리와 브랜치는 승인하든 버리든 함께
 * 지워지는데 "워크트리"라는 라벨은 그 사실을 전혀 담지 않는다.
 */
export function IsolationPicker({ choice, globalDefault, onPick }: Props) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    };
    window.addEventListener("mousedown", onDown);
    return () => window.removeEventListener("mousedown", onDown);
  }, [open]);

  const options = isolationOptions(globalDefault);
  const current = options.find((o) => o.choice === choice) ?? options[0];
  const pinned = choice !== "default";
  const isolated = choice === "pinned-on" || (choice === "default" && globalDefault);

  return (
    <div className="relative" ref={ref}>
      <button
        aria-label="격리 방식"
        aria-expanded={open}
        className={`flex items-center gap-1 text-xs border rounded-md px-2 py-1 hover:border-border-strong ${
          pinned ? "text-primary-bright border-primary/50" : "text-text-secondary border-border"
        }`}
        onClick={() => setOpen((v) => !v)}
        title={current.caption}
      >
        <Icon name={isolated ? "branch" : "folder"} size={14} />
        <span className="max-w-[160px] truncate">{isolated ? "워크트리" : "직접 실행"}</span>
        {pinned && <span className="text-text-muted">· 이 프로젝트</span>}
        <Icon name="chevronDown" size={12} />
      </button>

      {open && (
        <div className="absolute bottom-full left-0 mb-1 z-20 w-80 rounded-lg border border-border-strong bg-raised py-1 shadow-e2">
          <div className="px-3 py-1 text-xs text-text-muted">
            격리 방식 <span className="text-text-secondary">— 새 작업에 적용</span>
          </div>
          {options.map((o) => {
            const on = o.choice === choice;
            return (
              <button
                key={o.choice}
                className={`w-full text-left flex items-start gap-2 px-2 py-1.5 hover:bg-surface ${
                  on ? "text-text" : "text-text-secondary"
                }`}
                onClick={() => {
                  onPick(o.choice);
                  setOpen(false);
                }}
              >
                <span
                  className={`shrink-0 w-4 pt-0.5 ${on ? "text-primary-bright" : "text-text-muted"}`}
                >
                  <Icon
                    name={on ? "check" : o.choice === "pinned-off" ? "folder" : "branch"}
                    size={14}
                  />
                </span>
                <span className="flex-1 min-w-0">
                  <span className="block text-xs">{o.label}</span>
                  {/* 캡션이 이 컴포넌트의 존재 이유다 — 접거나 숨기지 않는다. */}
                  <span className="block text-[11px] text-text-muted leading-snug">
                    {o.caption}
                  </span>
                </span>
              </button>
            );
          })}
        </div>
      )}
    </div>
  );
}
