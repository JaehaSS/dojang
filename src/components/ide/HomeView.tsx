import { useEffect, useMemo, useState } from "react";
import { useHostScope } from "../../lib/host-scope";
import { githubReposList, type GhRepo, type Task } from "../../lib/ipc";
import { ago } from "../../lib/fmt";
import { canArchiveTask } from "../../lib/task-archive";
import { CompletedTaskArchiveList } from "./CompletedTaskArchiveList";
import { badgeLabelFor } from "../../lib/agents";
import { taskDotColor } from "../../lib/task-status";
import { Icon } from "./icons";
import { GithubIssuesSection } from "./GithubIssuesSection";
import { ALL_REPOS, pickRepo } from "./github-repos";
import { HomeQuizCard } from "./HomeQuizCard";
import { BacklogSection } from "./BacklogSection";
import { HomeStatusStrip } from "./HomeStatusStrip";
import { recentItems } from "./home-items";
import { PendingApprovalSection } from "./PendingApprovalSection";
import { TodaySection } from "./TodaySection";
import { HomeProjectEditor } from "./HomeProjectEditor";

interface Props {
  tasks: Task[];
  onOpenTask: (task: Task) => void;
  onArchiveTask?: (task: Task) => void;
  onOpenEnsemble: (ensemble: string) => void;
  /** 승인/거부 후 목록 새로고침 (App의 기존 refresh 메커니즘 재사용). */
  onRefresh: () => void;
  onOpenProject?: (root: string) => Promise<string>;
}


const repoBase = (p: string) => p.split("/").filter(Boolean).pop() ?? p;

/** '최근' 목록의 행 — 앙상블 그룹과 단일 작업이 같은 줄 높이·여백을 공유한다. */
const ROW =
  "w-full text-left flex items-center gap-2.5 px-3 py-2.5 border-b border-border last:border-b-0 hover:bg-surface";

/** 홈 대시보드 — 계획(오늘·이슈) → 개입(승인) → 복습·최근 한 방향으로 읽힌다. */
export function HomeView({ tasks, onOpenTask, onArchiveTask, onOpenEnsemble, onRefresh, onOpenProject }: Props) {
  // 세션에 속하지 않는 화면이라 호스트를 스코프에서 받는다 (ADR 0133).
  const host = useHostScope();
  // 앙상블은 '비교 실행' 별도 섹션이 아니라 최근 목록 안의 접힌 그룹 행으로 산다.
  const recent = recentItems(tasks);
  // 홈은 repo 스코프가 없어 최근 작업의 repo를 기본값으로 삼는다(가장 흔한 실사용 시나리오).
  const latestRepo = [...tasks].sort((a, b) => b.created_at - a.created_at)[0]?.repo;

  // 최근 작업이 다녀간 레포를 최신순 distinct로. GitHub 레포인지는 백엔드가 판정한다
  // (`git remote get-url` — 프론트가 알 수 없다).
  const repoPaths = useMemo(() => {
    const last = new Map<string, number>();
    for (const task of tasks) {
      if (task.created_at > (last.get(task.repo) ?? 0)) last.set(task.repo, task.created_at);
    }
    return [...last.entries()].sort((a, b) => b[1] - a[1]).map(([path]) => path);
  }, [tasks]);
  const repoKey = repoPaths.join("\n");

  const [ghRepos, setGhRepos] = useState<GhRepo[]>([]);
  // 사용자가 버튼으로 고른 레포. 고르기 전에는 null이고 최근 작업 레포를 따라간다.
  const [chosenRepo, setChosenRepo] = useState<string | null>(null);

  useEffect(() => {
    if (repoPaths.length === 0) {
      setGhRepos([]);
      return;
    }
    let cancelled = false;
    githubReposList(host, repoPaths)
      .then((repos) => {
        if (!cancelled) setGhRepos(repos);
      })
      // 판정 실패는 조용히 넘긴다 — GitHub 섹션만 숨겨지고 홈의 나머지는 그대로 산다.
      .catch(() => {
        if (!cancelled) setGhRepos([]);
      });
    return () => {
      cancelled = true;
    };
    // repoPaths는 매 렌더 새 배열이라 내용 기준 키로 의존한다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [repoKey]);

  const activeRepo = pickRepo(ghRepos, chosenRepo ?? latestRepo);
  // 할 일의 제안 소스(github·memory)는 지금 보고 있는 레포를 따른다 — 위에서 레포를 바꾸면
  // 아래 목록도 같은 맥락이 된다. GitHub 레포가 하나도 없으면 종전대로 최근 작업 레포.
  // '전체'는 어느 레포도 가리키지 않으므로 '오늘'까지 비우지 않고 최근 작업 레포로 둔다.
  const planRepo = (activeRepo === ALL_REPOS ? undefined : activeRepo) ?? latestRepo;

  return (
    <div className="flex-1 overflow-auto p-7">
      <div className="max-w-5xl mx-auto">
        <h1 className="text-xl font-medium mb-5 flex items-center gap-2">
          <span className="text-status-awaiting">
            <Icon name="sparkle" size={20} />
          </span>
          새 작업을 시작할까요?
        </h1>

        {/* 현황 — 아래 섹션들이 목록으로 말하는 것을 숫자 한 줄로 먼저 세운다. */}
        <HomeStatusStrip tasks={tasks} />
        {onOpenProject && <HomeProjectEditor localRoots={tasks.filter((task) => task.host === "local").map((task) => task.repo)} onOpen={onOpenProject} />}

        {/* 계획 — 오늘 하려던 것과 그 재료(이슈)를 붙여 둔다. 레포 선택이 둘 다를 움직인다. */}
        <TodaySection tasks={tasks} repo={planRepo} />

        {/* 오늘이 아닌 것. 오늘 목록에서 민 항목이 여기 쌓이고 여기서 다시 당긴다 —
            두 섹션이 떨어지면 밀기·당기기가 한 동작으로 읽히지 않는다. 비어 있으면
            통째로 사라진다. */}
        <BacklogSection repo={planRepo} />

        <GithubIssuesSection
          repos={ghRepos}
          repo={activeRepo}
          onSelectRepo={setChosenRepo}
          onOpenTask={onOpenTask}
          onRefresh={onRefresh}
        />

        {/* 개입 — 내가 손대야 진행되는 것. */}
        <PendingApprovalSection tasks={tasks} onRefresh={onRefresh} />

        {/* 복습 — 쌓인 문제를 푸는 자리. 낼 것이 없으면 통째로 사라진다. */}
        <HomeQuizCard />

        <div className="text-xs uppercase tracking-wide text-text-muted mb-2">최근</div>
        {recent.length === 0 ? (
          <div className="text-text-muted text-sm border border-border rounded-md p-4">
            표시할 작업이 없습니다. 아래 입력창에서 새 작업을 시작하세요.
          </div>
        ) : (
          <div className="border border-border rounded-md overflow-hidden">
            {recent.map((item) =>
              item.kind === "ensemble" ? (
                <button key={item.key} onClick={() => onOpenEnsemble(item.id)} className={ROW}>
                  <span className="text-primary-bright shrink-0">
                    <Icon name="scale" size={16} />
                  </span>
                  <span className="text-sm text-text truncate">
                    {item.tasks[0]?.instruction}
                  </span>
                  <span className="flex gap-1 shrink-0">
                    {item.tasks.map((t) => (
                      <span
                        key={t.id}
                        className="text-[11px] px-1.5 py-0.5 rounded bg-raised text-text-secondary font-code"
                      >
                        {badgeLabelFor(t.agent) ?? "?"}
                      </span>
                    ))}
                  </span>
                  <span className="ml-auto text-xs text-text-muted font-code shrink-0">
                    {item.ready}/{item.tasks.length} 완료 · {ago(item.at)}
                  </span>
                </button>
              ) : (
                <div key={item.key} className="flex items-center border-b border-border last:border-b-0">
                <button onClick={() => onOpenTask(item.task)} className={`${ROW} flex-1 min-w-0 border-b-0`}>
                  <span
                    className="w-2 h-2 rounded-full shrink-0"
                    style={{ background: taskDotColor(item.task) }}
                  />
                  <span className="text-sm text-text truncate">{item.task.instruction}</span>
                  {item.task.agent && (
                    <span className="text-[11px] px-1.5 py-0.5 rounded bg-raised text-text-secondary font-code shrink-0">
                      {badgeLabelFor(item.task.agent)}
                    </span>
                  )}
                  <span className="ml-auto text-xs text-text-muted font-code shrink-0">
                    {repoBase(item.task.repo)} · {ago(item.at)}
                  </span>
                </button>
                {onArchiveTask && canArchiveTask(item.task) && (
                  <button className="shrink-0 px-3 py-2 text-xs text-text-muted hover:text-text" title="기록·폴더·브랜치를 유지하고 보관합니다" onClick={() => onArchiveTask(item.task)}>보관</button>
                )}
                </div>
              ),
            )}
          </div>
        )}
        {onArchiveTask && <CompletedTaskArchiveList tasks={tasks} onOpenTask={onOpenTask} onArchiveTask={onArchiveTask} />}
      </div>
    </div>
  );
}
