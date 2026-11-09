import { useCallback, useEffect, useState, type ReactElement } from "react";

import { quizAvailability, type QuizAvailability } from "../../lib/ipc";
import { modeFor, type QuizMode } from "../../lib/quiz";
import { QuizPanel } from "../QuizPanel";
import { QuizReviewPanel } from "../QuizReviewPanel";
import { Icon } from "./icons";

/**
 * 홈에서 큐를 다시 세는 주기.
 *
 * 대기 게이트(5초)보다 훨씬 성기다 — 여기서 재는 것은 "지금 기다리는가"가 아니라 "풀 것이
 * 쌓였는가"이고, 그 수는 스케줄 틱에서만 늘어난다.
 */
export const HOME_QUIZ_POLL_MS = 60_000;

/**
 * 홈의 복습 카드.
 *
 * 대기 퀴즈(설계 0044)는 기다림이 임계값을 넘겨야 열린다 — 그래서 기다릴 일이 없는 날에는
 * 쌓인 문제를 볼 길이 아예 없었다. 이 카드는 경과 게이트 없이 큐를 그대로 보여주고, 언제
 * 풀지는 사용자가 고른다.
 *
 * **낼 것이 없으면 렌더하지 않는다**(ADR 0114와 같은 규칙). 빈 카드가 요구하는 행동은
 * 스케줄 등록인데 그건 홈에서 할 일이 아니고, 홈은 자리를 비워 두는 편이 낫다.
 */
export function HomeQuizCard(): ReactElement | null {
  const [counts, setCounts] = useState<QuizAvailability | null>(null);
  const [open, setOpen] = useState<QuizMode | null>(null);

  const load = useCallback(async () => {
    try {
      setCounts(await quizAvailability());
    } catch {
      // 조회 실패는 카드 부재로 나타난다 — 홈의 나머지를 막지 않는다.
      setCounts(null);
    }
  }, []);

  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), HOME_QUIZ_POLL_MS);
    return () => clearInterval(timer);
  }, [load]);

  /** 패널을 닫으면 다시 센다 — 푼 만큼 줄어야 카드가 정직하다. */
  const close = useCallback(() => {
    setOpen(null);
    void load();
  }, [load]);

  if (!counts) return null;
  const mode = modeFor(counts);
  if (!mode) return null;

  return (
    <>
      <div className="text-xs uppercase tracking-wide text-text-muted mb-2">복습</div>
      <button
        type="button"
        onClick={() => setOpen(mode)}
        className="w-full text-left flex items-center gap-2.5 px-3 py-2.5 border border-border rounded-md mb-6 hover:bg-surface"
      >
        <span className="text-status-awaiting shrink-0">
          <Icon name="bulb" size={16} />
        </span>
        <span className="text-sm text-text">
          {mode === "quiz" ? "풀 문제가 쌓여 있습니다" : "검수를 기다리는 문제가 있습니다"}
        </span>
        <span className="ml-auto text-xs text-text-muted font-code shrink-0">
          {[
            counts.askable > 0 ? `${counts.askable}문제` : null,
            counts.pending_review > 0 ? `검수 ${counts.pending_review}` : null,
          ]
            .filter(Boolean)
            .join(" · ")}
        </span>
      </button>
      {open ? (
        // 대기 게이트가 쓰는 자리와 같다 — 어디서 열렸든 퀴즈는 늘 같은 모서리에 뜬다.
        <div className="fixed bottom-4 right-4 z-40 w-80">
          {open === "review" ? (
            <QuizReviewPanel
              onDone={() => (counts.askable > 0 ? setOpen("quiz") : close())}
              doneLabel={counts.askable > 0 ? "퀴즈로" : "닫기"}
            />
          ) : (
            /* 홈에서 직접 연 것이라 기다리는 응답이 없다 — 도착 배지를 띄울 일이 없다. */
            <QuizPanel
              responseArrived={false}
              pendingReview={counts.pending_review}
              onClose={close}
              onReview={() => setOpen("review")}
            />
          )}
        </div>
      ) : null}
    </>
  );
}
