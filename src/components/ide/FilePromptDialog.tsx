import { useEffect, useRef, useState } from "react";
import { validateName } from "../../lib/file-ops";
import { Button, Dialog } from "../ui";

interface Props {
  open: boolean;
  title: string;
  /** 어디에 만드는지 보이게 하는 부제 — 대상 디렉터리 경로. */
  subtitle: string;
  /** 초기값 — 이름 변경이면 현재 이름. */
  initial: string;
  /** 그 디렉터리에 이미 있는 이름들 — 중복을 제출 전에 잡는다. */
  taken: ReadonlySet<string>;
  confirmLabel: string;
  /** 백엔드가 거부한 사유 — 있으면 다이얼로그를 열어 둔 채 표시한다. */
  serverError: string | null;
  onConfirm: (name: string) => void;
  onCancel: () => void;
}

/** 생성·이름 변경의 이름을 받는 모달.
 *
 * 검증은 **입력 즉시** 인라인으로 한다 — 제출 후 실패시키면 사용자가 무엇이 문제였는지
 * 두 단계 뒤에 알게 된다(PRD §6.2 S-02). */
export function FilePromptDialog({
  open,
  title,
  subtitle,
  initial,
  taken,
  confirmLabel,
  serverError,
  onConfirm,
  onCancel,
}: Props) {
  const [value, setValue] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (!open) return;
    setValue(initial);
    // 확장자 앞까지만 선택 — 이름만 바꾸는 게 대부분이다(PRD §6.2 S-02).
    queueMicrotask(() => {
      const el = inputRef.current;
      if (!el) return;
      el.focus();
      const dot = initial.startsWith(".") ? -1 : initial.lastIndexOf(".");
      el.setSelectionRange(0, dot > 0 ? dot : initial.length);
    });
  }, [open, initial]);

  if (!open) return null;

  // 초기값 그대로는 "중복"이 아니다 — 이름 변경에서 자기 이름에 걸리면 안 된다.
  const error = validateName(value, value.trim() === initial ? undefined : taken);

  const submit = () => {
    if (!error) onConfirm(value.trim());
  };

  return (
    <Dialog label={title} onClose={onCancel} className="w-80 p-4">
      <div className="text-sm text-text">{title}</div>
      <div className="text-xs text-text-muted truncate mb-2" title={subtitle}>
        {subtitle}
      </div>
      <input
        ref={inputRef}
        className="w-full h-8 px-2 rounded-md bg-surface border border-border text-sm text-text"
        value={value}
        onChange={(e) => setValue(e.target.value)}
        onKeyDown={(e) => {
          // Esc는 Dialog가 닫는다 — 여기서도 잡으면 onCancel이 두 번 불린다.
          if (e.key === "Enter") submit();
        }}
        aria-label={title}
        aria-invalid={error !== null}
      />
      <div className="min-h-5 text-xs text-status-failed">
        {value.trim() ? (error ?? serverError ?? "") : (serverError ?? "")}
      </div>
      <div className="flex justify-end gap-2 mt-1">
        <Button variant="ghost" size="sm" onClick={onCancel}>
          취소
        </Button>
        <Button variant="primary" size="sm" disabled={error !== null} onClick={submit}>
          {confirmLabel}
        </Button>
      </div>
    </Dialog>
  );
}
