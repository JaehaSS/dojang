// diff 라인 주석(B-1) 순수 로직 — diff 탭 거터·스레드와 변경 목록 재전송 줄이 사용.
// hunkLines는 Rust `annotations::quote_line`/`diffmodel::parse_unified`와 동일한 라인번호
// 추적 규칙을 따른다 — side/line 계산이 서버와 어긋나면 재매칭이 깨진다.

import type { DiffHunk, RematchedAnnotation } from "./ipc";

export interface HunkLine {
  kind: "context" | "add" | "del";
  text: string;
  oldLine: number | null;
  newLine: number | null;
  /** 주석 작성 시 사용할 side — del은 old, 그 외(context/add)는 new. */
  side: "old" | "new";
}

/** hunk 안의 각 라인에 old/new 라인번호를 부여한다(Rust `quote_line`과 동일 카운팅). */
export function hunkLines(hunk: DiffHunk): HunkLine[] {
  let oldN = hunk.old_range[0];
  let newN = hunk.new_range[0];
  const out: HunkLine[] = [];
  for (const line of hunk.lines) {
    if (line.kind === "context") {
      out.push({ kind: "context", text: line.text, oldLine: oldN, newLine: newN, side: "new" });
      oldN += 1;
      newN += 1;
    } else if (line.kind === "del") {
      out.push({ kind: "del", text: line.text, oldLine: oldN, newLine: null, side: "old" });
      oldN += 1;
    } else {
      out.push({ kind: "add", text: line.text, oldLine: null, newLine: newN, side: "new" });
      newN += 1;
    }
  }
  return out;
}

/** 주석이 걸리는 라인 번호 — side 규칙(del→old, 그 외→new)과 짝을 이룬다.
 *  통합·분할 렌더러가 같은 키를 만들려면 반드시 이 함수를 거쳐야 한다. */
export function annotationLine(line: HunkLine): number {
  return line.newLine ?? line.oldLine ?? 0;
}

/** (hunk id, line, side) 조합의 조회 키 — 인라인 스레드 그룹핑용. */
export function positionKey(hunkId: string, line: number, side: string): string {
  return `${hunkId}:${line}:${side}`;
}

/** 현재 diff에 재매칭된(orphaned가 아닌) 주석을 위치별로 묶는다. */
export function groupAnnotationsByPosition(
  annotations: RematchedAnnotation[],
): Map<string, RematchedAnnotation[]> {
  const map = new Map<string, RematchedAnnotation[]>();
  for (const annotation of annotations) {
    if (annotation.orphaned || !annotation.matched_hunk_id) continue;
    const key = positionKey(annotation.matched_hunk_id, annotation.line, annotation.side);
    const list = map.get(key);
    if (list) list.push(annotation);
    else map.set(key, [annotation]);
  }
  return map;
}

/** hunk_id 재매칭에 실패한 주석 — 삭제되지 않고 고아 배지로 보존된다. */
export function orphanedAnnotations(annotations: RematchedAnnotation[]): RematchedAnnotation[] {
  return annotations.filter((annotation) => annotation.orphaned);
}

export const HUMAN_AUTHOR = "human";

/** 에이전트가 MCP `review_comment_add`로 남긴 노트인가. author가 없는 옛 응답은 사람 주석이다. */
export function isAgentAnnotation(annotation: Pick<RematchedAnnotation, "author">): boolean {
  return annotation.author !== undefined && annotation.author !== HUMAN_AUTHOR;
}

/** 재전송 대상 id 목록 — 액션바 `[주석 n건 재전송]`이 사용. 사람이 쓴 draft만이다.
 *  에이전트 노트는 `sent`로 태어나지만, 여기서도 한 번 더 거른다 — 에이전트가 쓴 글이
 *  에이전트에게 되돌아가는 경로는 서버(`annotations_resend`)와 화면 양쪽에서 막는다. */
export function draftAnnotationIds(annotations: RematchedAnnotation[]): string[] {
  return annotations
    .filter((annotation) => annotation.status === "draft" && !isAgentAnnotation(annotation))
    .map((annotation) => annotation.id);
}
