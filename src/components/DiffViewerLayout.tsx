import type { DiffHunk, FileDiff, RematchedAnnotation } from "../lib/ipc";
import type { BaselineStatus } from "../lib/transport";
import { langFromPath } from "../lib/lang-from-path";
import type { PartialApplyState } from "./use-partial-apply";
import { AnnotatedUnifiedDiff } from "./DiffAnnotations";
import { AnnotatedSplitDiff } from "./DiffSplitView";
import { DiffFileHeader, PatchFallback, SplitDiff, type DiffMode } from "./DiffPresentation";

export const SHORTCUT_HINT = [
  "J / K — 다음·이전 hunk",
  "] / [ — 다음·이전 파일",
  "U / S — 통합·분할 전환",
  "V — 이 파일 확인함",
].join("\n");

/** 기준점이 재작성됐을 때만 뜬다. 평시에 아무것도 그리지 않아야 이 줄이 의미를 갖는다. */
export function BaselineNotice({ baseline }: { baseline: BaselineStatus | null }) {
  if (baseline?.kind !== "degraded") return null;
  return (
    <div className="shrink-0 border-b border-border bg-surface px-3 py-1.5 text-xs text-status-failed">
      기준점이 재작성돼(rebase·amend) 근사치를 보고 있습니다 — 이 작업과 무관한 변경이 섞일 수 있습니다.
    </div>
  );
}

interface BodyProps {
  current?: FileDiff;
  mode: DiffMode;
  fileHunks: DiffHunk[];
  annotations: RematchedAnnotation[];
  partial: PartialApplyState;
  onCreate: (input: { hunk_id: string; line: number; side: string; body_md: string }) => Promise<void>;
  onUpdate: (id: string, body_md: string) => Promise<void>;
  onRefresh: () => void;
}

export function DiffBody(props: BodyProps) {
  if (!props.current) {
    return (
      <div className="h-full flex flex-col items-center justify-center gap-3 text-text-muted">
        <span>이 작업에서 변경된 파일이 없습니다.</span>
        <button className="rounded border border-border px-3 py-1 text-xs" onClick={props.onRefresh}>
          다시 확인
        </button>
      </div>
    );
  }
  // 구조화 hunk가 없는 diff(바이너리·rename·mode 변경)만 폴백으로 내려간다.
  if (props.fileHunks.length === 0) {
    return (
      <>
        <DiffFileHeader file={props.current} />
        {props.mode === "split" ? (
          <SplitDiff patch={props.current.patch} />
        ) : (
          <PatchFallback patch={props.current.patch} />
        )}
      </>
    );
  }
  const Renderer = props.mode === "split" ? AnnotatedSplitDiff : AnnotatedUnifiedDiff;
  return (
    <>
      <DiffFileHeader file={props.current} />
      <Renderer
        hunks={props.fileHunks}
        annotations={props.annotations}
        onCreate={props.onCreate}
        onUpdateBody={props.onUpdate}
        selectedHunkIds={props.partial.selection}
        onToggleHunk={props.partial.toggle}
        lang={langFromPath(props.current.path)}
      />
    </>
  );
}
