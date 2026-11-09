import { useCallback, useEffect, useRef, useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import {
  formatObservedAt,
  prFailureReviewContext,
  safeGithubPullRequestUrl,
  safePublicHttpUrl,
  taskPrStatusGet,
  type PrCheck,
  type PrStatusSnapshot,
} from "../../lib/github-pr-status";

export interface PullRequestStatusPanelProps {
  taskId: number;
  disabled?: boolean;
  onAttachReview: (text: string) => void;
}

const checkLabel: Record<PrCheck["status"], string> = {
  pass: "통과",
  fail: "실패",
  pending: "진행 중",
  skipped: "생략",
  cancelled: "취소",
  unknown: "미확인",
};

const checkColor: Record<PrCheck["status"], string> = {
  pass: "text-status-done",
  fail: "text-status-failed",
  pending: "text-status-awaiting",
  skipped: "text-text-muted",
  cancelled: "text-text-muted",
  unknown: "text-text-muted",
};

const prStateLabel: Record<NonNullable<PrStatusSnapshot["pr"]>["state"], string> = { open: "열림", closed: "닫힘", merged: "병합됨" };
const reviewDecisionLabel = (decision: string) => ({ APPROVED: "승인됨", CHANGES_REQUESTED: "수정 요청", REVIEW_REQUIRED: "검토 필요", PENDING: "검토 대기" }[decision.toUpperCase()] ?? (decision ? "확인되지 않음" : "없음"));

const statusMessage = (snapshot: PrStatusSnapshot) => {
  if (snapshot.status === "no_pr") return "연결된 풀 리퀘스트가 없습니다.";
  if (snapshot.status === "unsupported") return "이 환경은 풀 리퀘스트 상태 조회를 지원하지 않습니다.";
  if (snapshot.status === "unavailable") return "풀 리퀘스트 상태를 조회하지 못했습니다.";
  return null;
};

function CheckRow({ check, onOpen }: { check: PrCheck; onOpen: (url: string) => void }) {
  const detailsUrl = safePublicHttpUrl(check.details_url);
  return <li className="border-t border-border py-2">
    <p><span className={checkColor[check.status]}>{checkLabel[check.status]}</span> · {check.name}</p>
    {check.description && <p className="mt-1 whitespace-pre-wrap break-words text-text-muted">{check.description}</p>}
    {detailsUrl
      ? <button type="button" title={detailsUrl} className="mt-1 text-xs text-primary-bright underline" onClick={() => onOpen(detailsUrl)}>검사 세부 정보 열기</button>
      : check.details_url && <p className="mt-1 text-xs text-text-muted">검사 세부 링크를 열 수 없습니다.</p>}
  </li>;
}

/** A user-refreshable, read-only PR snapshot.  It deliberately does not poll remote services. */
export function PullRequestStatusPanel({ taskId, disabled = false, onAttachReview }: PullRequestStatusPanelProps) {
  const [snapshot, setSnapshot] = useState<PrStatusSnapshot | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [openError, setOpenError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const generation = useRef(0);
  const inFlight = useRef(new Map<number, number>());

  const reload = useCallback(async () => {
    const current = inFlight.current.get(taskId);
    if (current === generation.current) return;
    const request = ++generation.current;
    inFlight.current.set(taskId, request);
    setLoading(true); setSnapshot(null); setError(null);
    try {
      const next = await taskPrStatusGet(taskId);
      if (generation.current === request) { setSnapshot(next); setError(null); }
    } catch (cause) {
      if (generation.current === request) { setSnapshot(null); setError(String(cause)); }
    } finally {
      if (inFlight.current.get(taskId) === request) inFlight.current.delete(taskId);
      if (generation.current === request) setLoading(false);
    }
  }, [taskId]);

  useEffect(() => {
    generation.current++;
    setSnapshot(null); setError(null); setOpenError(null); setLoading(false);
    void reload();
    return () => { generation.current++; };
  }, [taskId, reload]);

  const attach = () => {
    const context = snapshot && prFailureReviewContext(snapshot);
    if (context && !disabled) onAttachReview(context);
  };
  const openExternal = (url: string) => {
    setOpenError(null);
    void openUrl(url).catch((cause) => setOpenError(`링크를 열지 못했습니다: ${String(cause)}`));
  };
  const pr = snapshot?.status === "ready" ? snapshot.pr : null;
  const prUrl = safeGithubPullRequestUrl(pr?.url, snapshot?.repository, pr?.number);
  const message = snapshot && statusMessage(snapshot);
  const failureContext = snapshot && prFailureReviewContext(snapshot);

  return <section aria-label="PR·검사 상태" className="mt-3 border-t border-border pt-3">
    <div className="flex flex-wrap items-center gap-2">
      <h3 className="font-medium">PR·검사 상태</h3>
      <button type="button" disabled={loading} onClick={() => void reload()} className="text-text-muted hover:text-text disabled:opacity-40">PR 새로고침</button>
    </div>
    <p className="mt-1 text-xs text-text-muted">패널을 열 때와 사용자가 새로고침할 때만 원격 검사를 조회합니다. 미커밋 변경은 원격 검증에 포함되지 않습니다.</p>
    {loading && <p role="status" className="mt-2 text-sm text-text-muted">풀 리퀘스트 상태를 불러오는 중입니다.</p>}
    {error && <p role="alert" className="mt-2 whitespace-pre-wrap break-all text-sm text-status-failed">PR 상태 조회 실패: {error}</p>}
    {openError && <p role="alert" className="mt-2 whitespace-pre-wrap break-all text-sm text-status-failed">{openError}</p>}
    {message && <p role="status" className="mt-2 text-sm text-text-muted">{message}{snapshot?.message ? ` ${snapshot.message}` : ""}</p>}
    {snapshot && <div className="mt-2 space-y-2 text-sm">
      <p className="text-text-muted">저장소: {snapshot.repository ?? "알 수 없음"} · 작업 브랜치: {snapshot.branch}</p>
      <p className="text-text-muted">조회: {formatObservedAt(snapshot.observed_at)} · 로컬 HEAD: <span className="select-text break-all font-code">{snapshot.local_head ?? "알 수 없음"}</span></p>
      {pr && <article className="rounded border border-border bg-bg p-3">
        <div className="flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <p className="font-medium">PR #{pr.number}: {pr.title}</p>
          {prUrl ? <button type="button" title={prUrl} className="text-xs text-primary-bright underline" onClick={() => openExternal(prUrl)}>GitHub에서 PR 열기</button> : <span className="text-xs text-text-muted">PR 링크를 열 수 없습니다.</span>}
        </div>
        <p className="mt-1 text-text-muted">상태: {prStateLabel[pr.state]} · {pr.draft ? "초안" : "초안 아님"} · 검토 결정: {reviewDecisionLabel(pr.review_decision)}</p>
        <p className="text-text-muted">검토 요청: {pr.review_requests.length ? pr.review_requests.join(", ") : "없음"}</p>
        <p className="select-text break-all text-text-muted">PR HEAD: <span className="font-code">{pr.head_sha}</span> · {pr.head_branch} → {pr.base_branch}</p>
        {snapshot.local_head && snapshot.local_head !== pr.head_sha && <p role="alert" className="mt-2 text-status-awaiting">로컬 HEAD와 PR HEAD가 다릅니다. 이 원격 검사는 현재 로컬 결과의 검증으로 사용할 수 없습니다.</p>}
        {!snapshot.local_head && <p className="mt-2 text-text-muted">로컬 HEAD를 확인하지 못했습니다. 원격 검사가 현재 로컬 결과를 검증한다고 볼 수 없습니다.</p>}
        <div className="mt-3">
          <h4 className="font-medium">검사</h4>
          {pr.checks.length === 0
            ? <p className="text-text-muted">검사 기록이 없습니다. 통과를 확인할 수 없습니다.</p>
            : <ul aria-label="PR 검사 목록">{pr.checks.map((check, index) => <CheckRow key={`${check.name}:${index}`} check={check} onOpen={openExternal} />)}</ul>}
        </div>
        {failureContext && <button type="button" disabled={disabled} onClick={attach} className="mt-3 text-primary-bright disabled:opacity-40">실패 검사를 검토 사유에 연결</button>}
      </article>}
    </div>}
  </section>;
}
