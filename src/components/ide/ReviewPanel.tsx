import { useState } from "react";
import type { MultiReviewResult, ReviewDetail } from "../../lib/ipc";
import { Icon } from "./icons";
import { Markdown } from "./Markdown";
import { ReviewModal } from "../ReviewModal";
import { IconButton } from "../ui";

interface Props {
  result: MultiReviewResult;
  detail?: ReviewDetail;
  onClose: () => void;
}

/** 현재 작업 diff 멀티벤더 리뷰 플로팅 패널 — 종합 판정 + 벤더별 카드(컴팩트). */
export function ReviewPanel({ result, detail, onClose }: Props) {
  const [showDetail, setShowDetail] = useState(false);

  return (
    <>
      <div className="absolute right-4 bottom-20 z-30 w-[30rem] max-h-96 overflow-auto rounded-lg border border-border-strong bg-raised shadow-e2 p-3">
        <div className="flex items-center justify-between mb-2">
          <span className="text-xs uppercase tracking-wide text-text-muted">
            리뷰 · {result.items.length}개 모델
          </span>
          <div className="flex items-center gap-2">
            {detail && (
              <IconButton size="sm" label="상세 정보" onClick={() => setShowDetail(true)}>
                <Icon name="search" size={14} />
              </IconButton>
            )}
            <IconButton size="sm" label="닫기" onClick={onClose}>
              <Icon name="x" size={14} />
            </IconButton>
          </div>
        </div>

      {result.synthesis && (
        <div className="mb-2 p-2 rounded bg-bg border border-border">
          <div className="text-xs text-text-muted mb-1">종합 의견</div>
          <Markdown text={result.synthesis} />
        </div>
      )}

      {result.items.length === 0 ? (
        <div className="text-text-muted text-sm text-center py-4">결과가 없습니다.</div>
      ) : (
        <div className="flex flex-col gap-2">
          {result.items.map((it) => (
            <div key={it.vendor} className="rounded border border-border p-2">
              <div className="flex items-center gap-2 mb-1">
                <span className="font-medium text-sm font-code">{it.vendor}</span>
                <span
                  className={`ml-auto text-[11px] px-1.5 py-0.5 rounded shrink-0 ${
                    it.ok ? "bg-status-done/15 text-status-done" : "bg-dangerbg text-status-failed"
                  }`}
                >
                  {it.ok ? "완료" : "실패"}
                </span>
              </div>
              <div className={`max-h-48 overflow-auto ${it.ok ? "" : "text-text-muted text-sm"}`}>
                {it.ok ? <Markdown text={it.text} /> : <div className="whitespace-pre-wrap">{it.text}</div>}
              </div>
            </div>
          ))}
        </div>
      )}
      </div>

      {showDetail && detail && <ReviewModal detail={detail} onClose={() => setShowDetail(false)} />}
    </>
  );
}
