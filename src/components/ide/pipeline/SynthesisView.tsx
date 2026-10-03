import { useEffect, useState } from "react";
import { reviewGet, type ReviewRecord } from "../../../lib/ipc";
import {
  boundOutput,
  groupFindings,
  synthesisOf,
  vendorFindingsOf,
  vendorLabel,
  type Finding,
  type SynthFinding,
} from "../../../lib/pipeline";
import { ErrorLine } from "./parts";

function FindingItem({ f }: { f: SynthFinding | Finding }) {
  const where = f.file ? `${f.file}${f.line ? `:${f.line}` : ""}` : null;
  const sources = "sources" in f && f.sources?.length ? f.sources.map(vendorLabel).join(", ") : null;
  return (
    <li className="text-sm">
      <span>{f.summary}</span>
      <span className="ml-2 text-xs text-text-muted">
        {[f.requirement, where, sources && `출처 ${sources}`].filter(Boolean).join(" · ")}
      </span>
      {f.evidence ? (
        <details>
          <summary className="cursor-pointer text-xs text-text-muted">근거</summary>
          <pre className="whitespace-pre-wrap break-words font-code text-xs">{boundOutput(f.evidence, 2000)}</pre>
        </details>
      ) : null}
    </li>
  );
}

/** blocking은 위에 강조하고 advisory는 아래로 접는다. 종합 JSON이 아니면 원문을 그대로 보인다. */
export function SynthesisBody({ record }: { record: ReviewRecord }) {
  const synthesis = synthesisOf(record.result);
  const { blocking, advisory } = groupFindings(synthesis);
  return (
    <div className="space-y-3">
      {synthesis ? (
        <>
          <p className="text-sm">
            종합 판정: <strong>{synthesis.decision === "pass" ? "통과" : "수정 필요"}</strong>
            {synthesis.summary ? ` — ${synthesis.summary}` : ""}
          </p>
          <div role="group" aria-label="blocking 지적" className={blocking.length ? "rounded border border-status-failed p-2" : ""}>
            <h3 className="text-xs font-medium text-status-failed">blocking {blocking.length}건</h3>
            <ul className="space-y-1">{blocking.map((f, i) => <FindingItem key={i} f={f} />)}</ul>
          </div>
          <div role="group" aria-label="advisory 지적">
            <h3 className="text-xs font-medium text-text-muted">advisory {advisory.length}건</h3>
            <ul className="space-y-1">{advisory.map((f, i) => <FindingItem key={i} f={f} />)}</ul>
          </div>
        </>
      ) : (
        <pre className="whitespace-pre-wrap break-words font-code text-xs">{boundOutput(record.result.synthesis) || "종합 결과가 없습니다."}</pre>
      )}
      <div>
        <h3 className="text-xs font-medium text-text-muted">벤더별 원 지적</h3>
        {record.result.items.map((item) => {
          const findings = item.ok ? vendorFindingsOf(record.result, item.vendor, item.text) : null;
          return (
            <details key={item.vendor}>
              <summary className="cursor-pointer text-sm">
                {vendorLabel(item.vendor)} {item.ok ? (findings ? `· ${findings.length}건` : "") : "· 실패"}
              </summary>
              {findings ? (
                <ul className="space-y-1">{findings.map((f, i) => <FindingItem key={i} f={f} />)}</ul>
              ) : (
                <pre className="whitespace-pre-wrap break-words font-code text-xs">{boundOutput(item.text)}</pre>
              )}
            </details>
          );
        })}
      </div>
    </div>
  );
}

/** 리뷰 한 건을 `review_get`으로 불러와 종합과 벤더별 지적을 보인다. */
export function SynthesisView({ reviewId }: { reviewId: number }) {
  const [record, setRecord] = useState<ReviewRecord | null>(null);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    let live = true;
    setRecord(null);
    reviewGet(reviewId).then(
      (r) => live && setRecord(r),
      (e: unknown) => live && setError(String(e)),
    );
    return () => {
      live = false;
    };
  }, [reviewId]);
  if (error) return <ErrorLine message={error} />;
  if (!record) return <p className="text-sm text-text-muted">리뷰를 불러오는 중…</p>;
  return <SynthesisBody record={record} />;
}
