import type { RepoLessons, SubjectCount } from "../../../lib/ipc";
import { RankBar } from "./parts";

/**
 * 발견 공간 — 교훈 주제 섹션(설계 2026-09-28 §6, 슬라이스 T3). 저장소별로 `docs/lessons.json`
 * 투영이 있어야 나타난다 — 투영이 없는 저장소는 조용히 빠진다(§6.3).
 *
 * 목록이 비어 있으면(투영을 낸 저장소가 없거나 호출이 실패했거나) 아무것도 그리지 않는다 —
 * `DiscoveryCards`와 같은 원칙(ledger #554: 빈 0을 그리지 않는다).
 */
export function LessonThemes({ repos }: { repos: RepoLessons[] }) {
  if (repos.length === 0) return null;

  return (
    <section className="pt-6" data-testid="lesson-themes">
      <div className="flex items-baseline justify-between mb-3">
        <h2 className="text-lg font-semibold">교훈 주제</h2>
      </div>
      <div className="grid gap-2.5">
        {repos.map((r) => (
          <RepoLessonsCard key={r.repo} repo={r} />
        ))}
      </div>
    </section>
  );
}

function RepoLessonsCard({ repo }: { repo: RepoLessons }) {
  const maxLessons = Math.max(1, ...repo.subjects.map((s) => s.lessons));

  return (
    <div className="bg-surface border border-border rounded-lg p-3">
      <div className="flex items-baseline justify-between gap-2 mb-2">
        <span className="text-sm font-medium truncate font-code" title={repo.repo}>
          {shortRepo(repo.repo)}
        </span>
        <span className="text-xs text-text-secondary shrink-0">
          이 구간 항목 {repo.entries_in_window}건
        </span>
      </div>

      {repo.subjects.length > 0 && (
        <div className="grid gap-1.5">
          {repo.subjects.map((s) => (
            <SubjectRow key={s.subject} subject={s} max={maxLessons} />
          ))}
        </div>
      )}

      {repo.abandoned.length > 0 && (
        <details className="mt-2.5">
          <summary className="cursor-pointer text-xs text-text-secondary">
            버린 길 {repo.abandoned.length}건
          </summary>
          <ul className="mt-1.5 grid gap-1">
            {repo.abandoned.map((a, i) => (
              <li key={i} className="text-xs text-text-secondary">
                <span className="font-code text-text-muted">
                  #{a.number} · {a.date}
                </span>{" "}
                {a.text}
              </li>
            ))}
          </ul>
        </details>
      )}
    </div>
  );
}

function SubjectRow({ subject, max }: { subject: SubjectCount; max: number }) {
  const delta = subject.prev_lessons == null ? null : subject.lessons - subject.prev_lessons;
  return (
    <div className="grid grid-cols-[minmax(0,1fr)_auto] items-center gap-2">
      <div className="min-w-0">
        <div className="flex items-baseline justify-between gap-2 mb-0.5">
          <span className="text-xs text-text truncate font-code" title={subject.subject}>
            {subject.subject}
          </span>
          <span className="text-xs text-text-secondary font-code shrink-0">
            {subject.lessons}
            {delta != null && delta !== 0 && (
              <span className="text-text-muted"> {delta > 0 ? "▲" : "▼"}{Math.abs(delta)}</span>
            )}
          </span>
        </div>
        <RankBar value={subject.lessons} max={max} />
      </div>
    </div>
  );
}

/**
 * 저장소 경로를 짧게 — 홈 디렉터리를 `~`로 접는다. 백엔드 `cards.rs::short_repo`와 같은
 * 발상이지만, 프런트에는 홈 디렉터리를 직접 물을 IPC가 없어 macOS 경로 관례
 * (`/Users/<name>/...`)를 정규식으로 접는다. 맞지 않으면(다른 OS·상대 경로) 원문 그대로 둔다.
 */
function shortRepo(path: string): string {
  return path.replace(/^\/Users\/[^/]+(\/|$)/, "~$1");
}
