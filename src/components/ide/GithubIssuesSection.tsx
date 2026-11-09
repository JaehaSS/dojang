import { useCallback, useEffect, useMemo, useState } from "react";
import { useHostScope } from "../../lib/host-scope";
import type { HostId } from "../../lib/transport";
import { openUrl } from "@tauri-apps/plugin-opener";
import { ago } from "../../lib/fmt";
import {
  githubCreateTaskFromIssue,
  githubIssueDelete,
  githubIssuesList,
  type GhRepo,
  type GithubIssuesResult,
  type Task,
} from "../../lib/ipc";
import { Icon } from "./icons";
import { IconButton } from "../ui";
import {
  ALL_REPOS,
  hasNotice,
  issueRows,
  issueUrl,
  mergedStatus,
  repoLabels,
  reposWithIssues,
  toRepoIssues,
  withoutIssue,
  type IssueRow,
  type RepoIssues,
  type SectionStatus,
} from "./github-repos";

/** IPC 경계를 주입 가능하게 열어 둔다 — 테스트가 Tauri 런타임 없이 동작을 검증한다. */
export interface GithubIssuesApi {
  list: (repo: string) => Promise<GithubIssuesResult>;
  create: (repo: string, issueNumber: number, agent: string) => Promise<Task>;
  remove: (repo: string, issueNumber: number) => Promise<void>;
  open: (url: string) => void;
}

/** 호스트에 묶인 기본 API. 모듈 상수로 둘 수 없다 — 호스트는 렌더 시점에 정해진다. */
const defaultApi = (host: HostId): GithubIssuesApi => ({
  list: (repo) => githubIssuesList(host, repo),
  create: (repo, issueNumber, agent) => githubCreateTaskFromIssue(host, repo, issueNumber, agent),
  remove: (repo, issueNumber) => githubIssueDelete(host, repo, issueNumber),
  open: (url) => void openUrl(url).catch(() => {}),
});

interface Props {
  /** 이슈를 볼 수 있는 레포 후보(홈이 최근 작업에서 뽑아 백엔드가 걸러낸 것). */
  repos: GhRepo[];
  /** 지금 보고 있는 레포 경로, 또는 `ALL_REPOS`. 후보가 없으면 undefined이고 섹션은 그려지지 않는다. */
  repo: string | undefined;
  onSelectRepo: (path: string) => void;
  onOpenTask: (task: Task) => void;
  onRefresh: () => void;
  api?: GithubIssuesApi;
}

/**
 * 확정된 조회 결과 한 벌. **어느 후보 목록에 대한 것인지**(`key`)를 함께 들고 있어야 후보가
 * 바뀐 직후에 낡은 결과로 버튼을 그리지 않는다.
 */
interface Snapshot {
  key: string;
  byRepo: Map<string, RepoIssues>;
}

/** 판정 전의 빈 결과. 모듈 상수로 둬야 매 렌더 새 Map이 참조를 흔들지 않는다. */
const NO_RESULTS: Map<string, RepoIssues> = new Map();

/** 행을 가리키는 키 — 전체 보기에서는 레포가 달라도 번호가 겹치므로 경로까지 묶는다. */
const rowKey = (row: IssueRow): string => `${row.repoPath}#${row.issue.number}`;

/**
 * GitHub 이슈 → 태스크 홈 섹션(C-2). gh 미설치/미인증은 안내 카드, GitHub 레포가 하나도
 * 없으면 섹션 자체를 숨긴다(PRD F-07 "조용한 비활성").
 *
 * 레포를 여럿 오가는 것이 기본 사용이라, 목록이 **어느 레포의 것인지**를 헤더에 `owner/repo`로
 * 밝히고 헤더 오른쪽에 레포 전환 버튼을 둔다. 경로가 아니라 레포 이름으로 고르게 하는 이유는
 * 워크트리·동명 디렉터리가 경로만으로 구분되지 않기 때문이다.
 *
 * 버튼은 후보 전부가 아니라 **열린 이슈가 있는 레포만** 세운다. 그래서 활성 레포 하나가 아니라
 * 후보 전부를 한 번에 조회한다 — 어차피 필요한 판정이고, 결과를 레포별로 들고 있으면 전환이
 * 재조회 없이 즉시 끝난다. 후보 전부를 이미 들고 있으므로 '전체' 보기도 추가 호출 없이 선다.
 */
export function GithubIssuesSection({
  repos,
  repo,
  onSelectRepo,
  onOpenTask,
  onRefresh,
  api: injectedApi,
}: Props) {
  // GitHub 이슈는 레포가 사는 머신의 것이다 — 스코프가 그 머신을 정한다 (ADR 0133).
  const host = useHostScope();
  // host가 바뀔 때만 다시 짓는다. 매 렌더 새 객체를 만들면 이 참조에 매달린 load가 매번
  // 새 함수가 되고, 그것을 보는 아래 effect가 렌더마다 다시 조회한다 — 응답이 snapshot을
  // 갈아끼우면 또 렌더가 돌아 목록이 끝없이 다시 그려진다. 모듈 상수였을 때는 참조가
  // 저절로 안정적이라 드러나지 않던 함정이다.
  const api = useMemo(() => injectedApi ?? defaultApi(host), [injectedApi, host]);
  const [snapshot, setSnapshot] = useState<Snapshot | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  // 진행 중 표시는 번호가 아니라 행 키로 잡는다 — 전체 보기에서는 레포가 달라도 번호가 겹친다.
  const [creating, setCreating] = useState<string | null>(null);
  // 삭제는 되돌릴 수 없어 행 안에서 한 번 더 확인받는다. 확인·진행·실패를 행 키로 들고 있어야
  // 목록이 다시 그려져도 엉뚱한 행이 확인 상태로 남지 않는다.
  const [confirming, setConfirming] = useState<string | null>(null);
  const [deleting, setDeleting] = useState<string | null>(null);
  const [failure, setFailure] = useState<{ key: string; message: string } | null>(null);

  // repos는 매 렌더 새 배열일 수 있어 내용 기준 키로 의존한다(HomeView의 repoPaths와 같은 이유).
  const repoKey = repos.map((candidate) => candidate.path).join("\n");

  const load = useCallback(() => {
    if (repos.length === 0) return;
    setRefreshing(true);
    // 한 레포의 실패가 나머지를 못 보게 하면 안 된다 — 개별 catch로 error 상태만 남기고 간다.
    void Promise.all(
      repos.map((candidate) =>
        api
          .list(candidate.path)
          .then((result): [string, RepoIssues] => [candidate.path, toRepoIssues(result)])
          .catch((): [string, RepoIssues] => [
            candidate.path,
            { status: "error", issues: [], ownerRepo: null },
          ]),
      ),
    ).then((entries) => {
      // 결과는 통째로 갈아끼운다 — 레포별로 도착하는 대로 반영하면 버튼이 하나씩 늘거나 준다.
      setSnapshot({ key: repoKey, byRepo: new Map(entries) });
      setRefreshing(false);
    });
    // repoKey가 repos의 내용을 대신한다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [api, repoKey]);

  useEffect(() => {
    load();
  }, [load]);

  // 지금 후보에 대한 결과일 때만 쓴다. 후보가 바뀌면 다시 판정 전으로 돌아간다.
  const settled = snapshot !== null && snapshot.key === repoKey;
  const byRepo = settled ? snapshot.byRepo : NO_RESULTS;
  const showingAll = repo === ALL_REPOS;

  // 판정 전에는 **아무 버튼도 세우지 않는다.** 후보 전부를 먼저 그렸다가 응답이 오는 대로
  // 지우면, 사용자는 자기가 고를 수 있었던 레포가 사라지는 장면을 보게 된다. 없던 것이
  // 나타나는 쪽이 있던 것이 사라지는 쪽보다 낫다.
  const visibleRepos = useMemo(
    () => (settled ? reposWithIssues(repos, byRepo) : []),
    // repoKey가 repos의 내용을 대신한다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
    [repoKey, byRepo, settled],
  );
  const visibleKey = visibleRepos.map((candidate) => candidate.path).join("\n");

  // 활성 레포가 걸러졌으면 남은 첫 레포로 옮긴다. 옮기지 않으면 버튼에 없는 레포의 빈 목록을
  // 보고 있게 된다 — 이슈가 있는 레포를 눈앞에 두고도. '전체'는 어느 레포도 가리키지 않으므로
  // 여기서 건드리지 않는다.
  useEffect(() => {
    if (!settled || visibleRepos.length === 0 || repo === ALL_REPOS) return;
    if (repo && visibleRepos.some((candidate) => candidate.path === repo)) return;
    onSelectRepo(visibleRepos[0].path);
    // visibleKey가 visibleRepos의 내용을 대신한다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [settled, visibleKey, repo, onSelectRepo]);

  if (repos.length === 0) return null;

  // 보여줄 레포가 하나도 남지 않으면 섹션을 통째로 접는다 — 홈 위쪽 자리를 "이슈가 없다"는
  // 말 한 줄로 지키는 것은 값이 비싸다(PRD F-07 "조용한 비활성"). 다만 gh 부재·조회 실패처럼
  // 사용자가 고칠 수 있는 사정이 있으면 남긴다. 그때 접으면 왜 이슈가 안 보이는지 물어볼
  // 자리조차 없어진다 — 이유를 말하는 카드가 이 섹션 안에 있다.
  if (settled && visibleRepos.length === 0 && !hasNotice(byRepo)) return null;

  const createTask = async (row: IssueRow) => {
    setCreating(rowKey(row));
    try {
      const task = await api.create(row.repoPath, row.issue.number, "claude");
      onRefresh();
      onOpenTask(task);
    } finally {
      setCreating(null);
    }
  };

  const deleteIssue = async (row: IssueRow) => {
    const key = rowKey(row);
    setDeleting(key);
    setFailure(null);
    try {
      await api.remove(row.repoPath, row.issue.number);
      // 지운 한 건만 빼고 나머지는 그대로 둔다 — 재조회하면 레포 수만큼 gh를 다시 부른다.
      setSnapshot((current) =>
        current === null
          ? current
          : { ...current, byRepo: withoutIssue(current.byRepo, row.repoPath, row.issue.number) },
      );
      setConfirming(null);
    } catch (error) {
      // 삭제 권한이 없으면(triage 이하) gh가 그 사유를 준다. 사유를 지우면 사용자는 무엇을
      // 고쳐야 하는지 알 수 없다 — "실패했습니다"로 뭉개지 않는다.
      setFailure({ key, message: error instanceof Error ? error.message : String(error) });
    } finally {
      setDeleting(null);
    }
  };

  const active = repo && !showingAll ? byRepo.get(repo) : undefined;
  // 판정이 끝났는데도 활성 레포의 결과가 없다면 위 effect가 방금 레포를 옮긴 직후다 — 다음
  // 렌더에서 채워지므로 그때까지는 로딩으로 둔다.
  let status: SectionStatus = "loading";
  if (settled) status = showingAll ? mergedStatus(repos, byRepo) : (active?.status ?? "loading");
  const rows = issueRows(repos, byRepo, repo);

  const labels = repoLabels(visibleRepos);
  // 판정 전에도 헤더는 레포 이름을 유지한다 — 전환할 때마다 이름이 사라지면 어디를 보고
  // 있는지 놓친다. 확정된 응답이 오면 그 값으로 대체된다. 남은 레포가 하나도 없을 때만
  // 이름을 접는다 — 그때 헤더가 가리킬 레포는 없다.
  const shownRepo =
    settled && visibleRepos.length === 0
      ? undefined
      : (active?.ownerRepo ?? repos.find((r) => r.path === repo)?.owner_repo);

  // 빈 목록의 이유를 구분해 말한다 — 후보가 통째로 걸러진 것과 보고 있는 레포만 빈 것은 다르다.
  let emptyNote = "열린 이슈가 없습니다.";
  if (visibleRepos.length === 0) emptyNote = "열린 이슈가 있는 레포가 없습니다.";
  else if (showingAll) emptyNote = "모든 레포에 열린 이슈가 없습니다.";
  else if (shownRepo) emptyNote = `${shownRepo}에 열린 이슈가 없습니다.`;

  return (
    <>
      <div className="flex items-center gap-2 mb-2">
        <div className="text-xs uppercase tracking-wide text-text-muted">GitHub 이슈</div>
        {showingAll ? (
          <div className="text-xs text-text-secondary">레포 {visibleRepos.length}개 전체</div>
        ) : (
          shownRepo && (
            <div className="text-xs text-text-secondary font-code truncate" title={repo}>
              {shownRepo}
            </div>
          )
        )}
        {/* 무슨 목록을 보고 있는지 — gh 기본값인 '열린 이슈'를 건수와 함께 밝힌다. */}
        {status === "ready" && (
          <div className="text-xs text-text-muted shrink-0">열린 이슈 {rows.length}건</div>
        )}
        <div className="ml-auto flex items-center gap-1 min-w-0">
          {visibleRepos.length > 1 && (
            <div
              className="flex items-center gap-1 overflow-x-auto"
              role="group"
              aria-label="레포 선택"
            >
              {/* 레포를 오가며 훑는 대신 한눈에 보고 싶을 때 — 이미 캐시된 결과를 합칠 뿐이라
                  누르는 순간 조회 없이 그려진다. */}
              <button
                type="button"
                aria-pressed={showingAll}
                title="모든 레포의 열린 이슈를 최근 갱신순으로"
                className={`h-6 px-2 rounded text-xs shrink-0 ${
                  showingAll ? "bg-raised text-text" : "text-text-muted hover:text-text hover:bg-surface"
                }`}
                onClick={() => onSelectRepo(ALL_REPOS)}
              >
                전체
              </button>
              {visibleRepos.map((candidate) => {
                const active = candidate.path === repo;
                return (
                  <button
                    key={candidate.path}
                    type="button"
                    aria-pressed={active}
                    title={`${candidate.owner_repo} · ${candidate.path}`}
                    className={`h-6 px-2 rounded text-xs font-code shrink-0 ${
                      active
                        ? "bg-raised text-text"
                        : "text-text-muted hover:text-text hover:bg-surface"
                    }`}
                    onClick={() => onSelectRepo(candidate.path)}
                  >
                    {labels.get(candidate.path) ?? candidate.owner_repo}
                  </button>
                );
              })}
            </div>
          )}
          {/* 새로고침 중에는 목록을 비우지 않고 버튼만 진행 중임을 알린다 — 있던 것을 지웠다
              다시 그리면 방금 고친 그 깜빡임이 새로고침마다 돌아온다. */}
          <IconButton size="sm" label="새로고침" onClick={load} disabled={refreshing} aria-busy={refreshing}>
            <Icon name="refresh" size={14} />
          </IconButton>
        </div>
      </div>
      {status === "loading" && (
        <div className="text-text-muted text-sm border border-border rounded-md p-4 mb-6">
          이슈를 불러오는 중…
        </div>
      )}
      {status === "not_github_repo" && (
        <div className="text-text-muted text-sm border border-border rounded-md p-4 mb-6">
          이 레포에는 GitHub 원격이 없습니다.
        </div>
      )}
      {status === "unavailable" && (
        <div className="text-text-muted text-sm border border-border rounded-md p-4 mb-6">
          gh CLI가 설치/인증되어 있지 않습니다. <code className="font-code">gh auth login</code> 실행 후
          새로고침하세요.
        </div>
      )}
      {status === "error" && (
        <div className="text-status-failed text-sm border border-border rounded-md p-4 mb-6">
          이슈 목록을 불러오지 못했습니다.
        </div>
      )}
      {status === "ready" && (
        <div className="border border-border rounded-md overflow-hidden mb-6">
          {rows.length === 0 ? (
            <div className="text-text-muted text-sm p-4">{emptyNote}</div>
          ) : (
            rows.map((row) => {
              const key = rowKey(row);
              const { issue, ownerRepo } = row;
              return (
                <div key={key} className="border-b border-border last:border-b-0">
                  <div className="flex items-center gap-2.5 px-3 py-2.5">
                    <span className="text-primary-bright shrink-0">
                      <Icon name="branch" size={14} />
                    </span>
                    {/* 전체 보기에서는 행마다 레포가 다르다 — 어느 레포의 이슈인지 번호 앞에 밝힌다. */}
                    {showingAll && (
                      <span
                        className="text-[11px] text-text-muted font-code shrink-0"
                        title={row.repoPath}
                      >
                        {labels.get(row.repoPath) ?? ownerRepo}
                      </span>
                    )}
                    <button
                      type="button"
                      className="text-xs text-text-muted hover:text-text font-code shrink-0"
                      title={ownerRepo ? `${ownerRepo}#${issue.number} 열기` : "GitHub에서 열기"}
                      disabled={!ownerRepo}
                      onClick={() => ownerRepo && api.open(issueUrl(ownerRepo, issue.number))}
                    >
                      #{issue.number}
                    </button>
                    <span className="text-sm text-text truncate">{issue.title}</span>
                    {issue.labels.length > 0 && (
                      <span className="flex gap-1 shrink-0">
                        {issue.labels.map((label) => (
                          <span
                            key={label.name}
                            className="text-[11px] px-1.5 py-0.5 rounded bg-raised text-text-secondary"
                          >
                            {label.name}
                          </span>
                        ))}
                      </span>
                    )}
                    <span className="ml-auto text-xs text-text-muted font-code shrink-0">
                      {ago(Math.floor(Date.parse(issue.updated_at) / 1000))}
                    </span>
                    <button
                      className="h-7 px-2.5 rounded text-xs font-medium bg-surface hover:bg-raised disabled:opacity-40 shrink-0"
                      disabled={creating === key}
                      onClick={() => void createTask(row)}
                    >
                      태스크 생성
                    </button>
                    <IconButton
                      size="sm"
                      tone="danger"
                      label={`#${issue.number} 삭제`}
                      disabled={deleting === key}
                      onClick={() => {
                        setFailure(null);
                        setConfirming(key);
                      }}
                    >
                      <Icon name="x" size={14} />
                    </IconButton>
                  </div>
                  {/* 확인은 행 아래 줄로 편다 — 한 줄에 밀어 넣으면 제목이 밀려 무엇을 지우는지 흐려진다. */}
                  {confirming === key && (
                    <div className="flex items-center gap-2 px-3 pb-2.5">
                      <span className="text-[11px] text-status-failed">
                        GitHub에서 완전히 삭제됩니다 — 되돌릴 수 없습니다.
                      </span>
                      <button
                        type="button"
                        disabled={deleting === key}
                        onClick={() => void deleteIssue(row)}
                        className="rounded border border-dangerborder px-1.5 py-0.5 text-[11px] text-status-failed disabled:opacity-40"
                      >
                        삭제
                      </button>
                      <button
                        type="button"
                        onClick={() => setConfirming(null)}
                        className="rounded border border-border px-1.5 py-0.5 text-[11px] text-text-muted"
                      >
                        취소
                      </button>
                    </div>
                  )}
                  {failure?.key === key && (
                    <div className="px-3 pb-2.5 text-[11px] text-status-failed">
                      삭제하지 못했습니다 — {failure.message}
                    </div>
                  )}
                </div>
              );
            })
          )}
        </div>
      )}
    </>
  );
}
