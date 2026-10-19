import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import type { DiffHunk, FileDiff } from "../lib/ipc";
import type { DiffRange } from "../lib/transport";
import { hunkLines } from "../lib/annotations";
import { describePatch, summarizePatch, toSplitRows, type Segment, type Side } from "../lib/diff";
import {
  getDiffTokenizer,
  hunkTokenStreams,
  mergeLinePieces,
  tokenStyle,
  type HunkTokens,
  type TokenSpan,
} from "../lib/diff-highlight";
import { getActiveTheme, subscribeTheme, type Theme } from "../lib/themes";

export type DiffMode = "unified" | "split";

const sideBg: Record<Side["kind"], string> = {
  ctx: "",
  add: "bg-addbg text-status-done",
  del: "bg-delbg text-status-failed",
  empty: "bg-empty",
  hunk: "text-primary-bright",
  gap: "bg-raised text-text-muted",
};

function SideCell({ side }: { side: Side }) {
  return (
    <div className={`flex flex-1 min-w-0 ${sideBg[side.kind]}`}>
      <span className="w-10 shrink-0 text-right pr-2 text-text-muted select-none">
        {side.n ?? ""}
      </span>
      <span className="whitespace-pre-wrap break-all">{side.text || " "}</span>
    </div>
  );
}

/** diff 한 줄의 본문 — 단어 단위 강조가 있으면 바뀐 구간만 2단계 톤으로 덧칠하고,
 *  토큰이 있으면 구문 색을 전경으로 얹는다(설계 DR-13). 두 축은 독립이라 조각 병합으로 끝난다.
 *  둘 다 없으면(토크나이저 미등록·통째로 바뀐 줄) 줄 틴트만 남기고 평문으로 렌더한다. */
export function DiffLineText({
  text,
  segments,
  tokens,
  theme,
  kind,
  className,
}: {
  text: string;
  segments?: Segment[];
  /** 이 줄의 구문 토큰. theme과 함께 와야 색이 붙는다. */
  tokens?: TokenSpan[];
  theme?: Theme;
  kind: string;
  className?: string;
}) {
  const colored = tokens && tokens.length > 0 && theme ? tokens : undefined;
  if (!segments && !colored) return <span className={className}>{text}</span>;
  const strong = kind === "add" ? "bg-addbg-strong" : "bg-delbg-strong";
  const pieces = mergeLinePieces(text, colored, segments, (type) =>
    theme ? tokenStyle(type, theme) : undefined,
  );
  return (
    <span className={className}>
      {pieces.map((piece, index) => (
        <span
          key={index}
          className={piece.changed ? strong : undefined}
          style={
            piece.color || piece.italic
              ? { color: piece.color, fontStyle: piece.italic ? "italic" : undefined }
              : undefined
          }
        >
          {piece.text}
        </span>
      ))}
    </span>
  );
}

/** 구문 색 해석용 활성 테마. use-theme.ts의 `useTheme`과 같되 서버 스냅샷을 준다 — diff
 *  렌더러는 `renderToStaticMarkup` 테스트로도 그려지고, 서버 스냅샷이 없으면 React가 던진다. */
export function useDiffTheme(): Theme {
  return useSyncExternalStore(subscribeTheme, getActiveTheme, getActiveTheme);
}

/** hunk별 구문 토큰(설계 DR-11). 토크나이저가 없으면 null — 호출부는 평문으로 그린다.
 *  hunk id는 내용 해시라 id 목록 + 언어가 같으면 결과도 같다. 호출부가 매 렌더 새 배열을
 *  넘겨도(filter) 다시 토큰화하지 않도록 배열 동일성 대신 이 서명으로 갱신을 판단한다. */
export function useHunkTokens(hunks: DiffHunk[], lang: string): Map<string, HunkTokens> | null {
  const signature = `${lang}\u0000${hunks.map((hunk) => hunk.id).join("\u0000")}`;
  const [state, setState] = useState<{ signature: string; tokens: Map<string, HunkTokens> } | null>(
    null,
  );
  const hunksRef = useRef(hunks);
  hunksRef.current = hunks;

  useEffect(() => {
    const tokenize = getDiffTokenizer();
    if (!tokenize) return;
    // 파일·언어가 바뀌는 사이에 끝난 이전 토큰화 결과는 버린다 — 다른 본문에 색을 칠하게 된다.
    let cancelled = false;
    const current = hunksRef.current;
    Promise.all(
      current.map(async (hunk) => {
        const streams = hunkTokenStreams(hunkLines(hunk));
        const [old, next] = await Promise.all([
          Promise.resolve(tokenize(streams.oldText, lang)),
          Promise.resolve(tokenize(streams.newText, lang)),
        ]);
        return [hunk.id, { old, new: next }] as const;
      }),
    )
      .then((entries) => {
        if (!cancelled) setState({ signature, tokens: new Map(entries) });
      })
      .catch(() => {
        // 토큰화 실패는 평문으로 남긴다 — 강조는 보조 정보다.
      });
    return () => {
      cancelled = true;
    };
  }, [signature, lang]);

  return state?.signature === signature ? state.tokens : null;
}

/** hunk 부분 승인(B-2) 선택 체크박스 — 통합·분할 모드가 공유한다.
 *  `selectedHunkIds`가 없으면(부분 승인 비활성) 아무것도 렌더하지 않는다. */
export function HunkCheckbox({
  hunk,
  selectedHunkIds,
  onToggle,
}: {
  hunk: DiffHunk;
  selectedHunkIds?: Set<string>;
  onToggle?: (hunk: DiffHunk) => void;
}) {
  if (!selectedHunkIds) return null;
  // 커밋된 hunk는 고를 것이 없다 — 유지도 폐기도 대상이 아니라 비활성 체크박스조차
  // 오해를 부른다(protected는 "체크 안 하면 지워짐"이라 비활성으로 남기는 것이 맞다).
  if (hunk.committed) {
    return (
      <span
        className="rounded bg-raised px-1 text-[10px] text-text-muted"
        title="이미 커밋된 변경 — 부분 적용이 건드리지 않습니다"
      >
        커밋됨
      </span>
    );
  }
  return (
    <input
      type="checkbox"
      checked={!hunk.protected && selectedHunkIds.has(hunk.id)}
      disabled={hunk.protected}
      title={
        hunk.protected
          ? "protected 경로 변경 — 부분 적용으로 유지할 수 없습니다(항상 제거됨)"
          : undefined
      }
      onChange={() => onToggle?.(hunk)}
    />
  );
}

export function DiffRangeToggle({
  range,
  onChange,
}: {
  range: DiffRange;
  onChange: (range: DiffRange) => void;
}) {
  const options: { value: DiffRange; label: string; title: string }[] = [
    { value: "session", label: "세션 전체", title: "이 작업이 시작된 뒤의 모든 변경 (커밋 포함)" },
    { value: "uncommitted", label: "미커밋", title: "아직 커밋하지 않은 변경만" },
  ];
  return (
    <div
      className="flex h-7 rounded-md border border-border bg-surface p-0.5"
      role="group"
      aria-label="Diff 범위"
    >
      {options.map((option) => (
        <button
          key={option.value}
          className={`rounded px-2.5 text-xs ${
            range === option.value
              ? "bg-raised text-text shadow-sm"
              : "text-text-muted hover:text-text-secondary"
          }`}
          aria-pressed={range === option.value}
          title={option.title}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function DiffModeToggle({
  mode,
  onChange,
  disabled = false,
  disabledTitle,
}: {
  mode: DiffMode;
  onChange: (mode: DiffMode) => void;
  /** 본문이 좁아 unified가 강제된 구간 — 고를 수 있는 척하지 않는다(설계 DR-6). */
  disabled?: boolean;
  disabledTitle?: string;
}) {
  const options: { value: DiffMode; label: string; title: string }[] = [
    { value: "unified", label: "통합", title: "한 열에서 추가·삭제 함께 보기" },
    { value: "split", label: "분할", title: "이전·이후를 두 열로 비교하기" },
  ];
  return (
    <div className="flex h-7 rounded-md border border-border bg-surface p-0.5" role="group" aria-label="Diff 보기 방식">
      {options.map((option) => (
        <button
          key={option.value}
          className={`rounded px-2.5 text-xs disabled:opacity-50 ${
            mode === option.value
              ? "bg-raised text-text shadow-sm"
              : "text-text-muted hover:text-text-secondary"
          }`}
          aria-pressed={mode === option.value}
          disabled={disabled}
          title={disabled ? (disabledTitle ?? option.title) : option.title}
          onClick={() => onChange(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
  );
}

export function DiffFileHeader({ file }: { file: FileDiff }) {
  const summary = summarizePatch(file.patch);
  return (
    <div className="min-h-9 shrink-0 border-b border-border bg-raised px-3 py-2 flex items-center gap-2 font-code text-xs">
      <span className="text-text-muted">{file.status}</span>
      <span className="min-w-0 flex-1 truncate text-text" title={file.path}>
        {file.path}
      </span>
      <span className="text-status-done">+{summary.additions}</span>
      <span className="text-status-failed">−{summary.deletions}</span>
    </div>
  );
}

export function PatchFallback({ patch }: { patch: string }) {
  return (
    <div className="m-3 rounded-md border border-border bg-surface px-3 py-4 text-sm text-text-muted">
      {describePatch(patch)}
    </div>
  );
}

export function SplitDiff({ patch }: { patch: string }) {
  const rows = toSplitRows(patch);
  const contentRows = rows.filter((row) => !["hunk", "gap"].includes(row.left.kind));
  if (contentRows.length === 0) return <PatchFallback patch={patch} />;
  return (
    <div className="text-xs font-code leading-relaxed min-w-[600px]">
      {rows.map((row, index) => {
        if (row.left.kind === "hunk") {
          return (
            <div key={index} className="px-3 text-primary-bright bg-empty">
              {row.left.text}
            </div>
          );
        }
        if (row.left.kind === "gap") {
          return (
            <div key={index} className="my-1 px-3 py-1 text-center bg-raised text-text-muted">
              {row.left.text}
            </div>
          );
        }
        return (
          <div key={index} className="flex">
            <SideCell side={row.left} />
            <div className="w-px bg-border shrink-0" />
            <SideCell side={row.right} />
          </div>
        );
      })}
    </div>
  );
}
