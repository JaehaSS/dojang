import type { CSSProperties, ReactElement } from "react";
import type { ActivityItem } from "../../lib/activity";
import { taskRef, type ConvoStatus, type Task } from "../../lib/ipc";
import { taskKey } from "../../lib/transport";
import { ActivityPanel } from "./ActivityPanel";
import { FLOATING_CHANNEL_WIDTH } from "./activity-rail";
import { ChannelCard } from "./ChannelCard";
import { ContextChannel, type ContextChannelBinding } from "./ContextChannel";
import { KnowCardsSection, type KnowCardsBinding } from "./KnowCardsSection";
import { TodaySection } from "./TodaySection";
import type { WorkContextDiff } from "./WorkContextPanel";

interface Props {
  task: Task;
  runtime: "local" | "remote";
  diff: WorkContextDiff;
  items: ActivityItem[];
  busy: boolean;
  activity: ConvoStatus | null;
  onRefreshDiff: () => void;
  onOpenSubagent: (toolId: string) => void;
  /** 최근 활동 진입점 — 긴 목록은 채널 대신 코드 열의 작업정보 탭이 맡는다. */
  onOpenRecentActivity: () => void;
  /** 할 일 채널의 Task 배지용 — App이 이미 들고 있는 목록을 그대로 받는다. */
  tasks?: Task[];
  /** 플로팅 채널 접기 — 환경 헤더의 ✕. 코드 열의 작업정보 탭에서는 전달하지 않는다(Don't #12). */
  onCollapse?: () => void;
  /** 알아둘 것 카드 — 채널 맨 아래. 활성 카드가 없으면 섹션째 없다. */
  know?: KnowCardsBinding;
  /** 카드가 지금 화면에 보이는가. 읽음 처리의 조건이다 — 가려진 탭의 카드는 읽은 것이 아니다. */
  knowVisible?: boolean;
  context?: ContextChannelBinding;
  contextVisible?: boolean;
}

/**
 * 세션 우상단의 채널 스택: 작업정보, 컨텍스트 요약, 오늘 할 일, 하단 알아둘 것 카드.
 *
 * **세션 열 안에** 마운트된다. 헤더·파일 트리·코드 열은 채널의 영역이 아니고, 세션은 채널이
 * 떠 있는 동안 `FLOATING_CHANNEL_RESERVED`만큼 오른쪽을 비워 겹침을 없앤다 — absolute 배치의
 * 기준은 padding box라 이 예약은 카드 위치를 밀지 않는다.
 *
 * 컨테이너는 아래까지 내려오지만 배경도 클릭도 받지 않는다(`pointer-events-none`) — 카드만
 * 콘텐츠 높이로 떠 있고, 그 사이 공백과 아래 여백은 세션 그대로다.
 *
 * 공간이 모자라면 작업정보가 먼저 줄어들고 내부에서 스크롤된다. 컨텍스트가 있으면 작업정보에
 * 최소 96px을 남기고 할 일과 알아둘 것의 높이 제한을 낮춘다. 더 낮은 창에서는 스택 전체를
 * 스크롤할 수 있어 요약과 하단 카드의 버튼까지 도달할 수 있다.
 */
export function ActivityRail({ tasks, onCollapse, know, knowVisible = true, context, contextVisible = true, ...panel }: Props): ReactElement {
  return (
    <div
      className="pointer-events-none absolute top-4 right-4 bottom-4 z-30 flex flex-col gap-3 overflow-y-auto"
      style={{
        width: FLOATING_CHANNEL_WIDTH,
        // Leave room for the context summary, work-info header and inter-card gaps.
        "--channel-today-max-height": context ? "min(45%, max(80px, calc(50% - 180px)))" : "45%",
        "--channel-know-max-height": "min(30%, max(96px, calc(50% - 180px)))",
      } as CSSProperties}
    >
      <ChannelCard label="플로팅 작업 정보" className={context ? "min-h-[96px]" : "min-h-0"}>
        <ActivityPanel {...panel} density="rail" onCollapse={onCollapse} />
      </ChannelCard>

      {context && <ContextChannel key={`context:${taskKey(taskRef(panel.task))}`} context={context} density="rail" active={contextVisible} />}

      {/* 비면 카드째 사라진다 — 세션 화면에 빈 껍데기를 매일 띄우지 않는다(설계 0021 §12). */}
      <TodaySection density="channel" tasks={tasks} repo={panel.task.repo} />

      {/* 세션이 바뀌면 1페이지부터 — 앞 세션에서 넘기던 자리를 새 세션에 가져가지 않는다. */}
      {know && (
        <KnowCardsSection
          key={taskKey(taskRef(panel.task))}
          {...know}
          visible={knowVisible}
          className={context ? "mt-auto shrink-0 max-h-[var(--channel-know-max-height)]" : "mt-auto shrink-0 max-h-[30%]"}
        />
      )}
    </div>
  );
}

/**
 * 코드 열의 `작업정보` 탭 — 파일·Diff를 부른 동안 채널이 흡수되는 자리다(ADR 0066 결정 3).
 *
 * 플로팅과 같은 카드를 쓰며 컨텍스트 요약을 맨 위에 고정한다. 열 전체를 쓰는 `panel` 밀도에서
 * 컨텍스트 상세와 최근 활동을 펼칠 수 있다.
 */
export function ActivityColumnTab({ tasks, onCollapse, know, knowVisible = false, context, contextVisible = false, ...panel }: Props): ReactElement {
  // 코드 열의 작업정보 탭은 닫히지 않는다(Don't #12) — 접기 손잡이는 받아도 버린다.
  void onCollapse;
  // `knowVisible` 기본값이 false인 이유: 탭은 가려진 채 마운트돼 있을 때가 많다.
  // 보인다는 신호가 없으면 카드를 읽음 처리하지 않는다. 카드는 스크롤 영역 아래에 붙인다(`sticky`).
  // 작업정보가 길면 카드가 화면 밖으로 밀려나, 탭이 보인다는 신호만으로는 카드를 봤다고 할 수 없다.
  return (
    <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto p-3">
      {context && <ContextChannel key={`context:${taskKey(taskRef(panel.task))}`} context={context} density="panel" active={contextVisible} />}
      <ChannelCard label="작업 정보" className="shrink-0">
        <ActivityPanel {...panel} />
      </ChannelCard>

      <TodaySection density="channel" tasks={tasks} repo={panel.task.repo} />

      {know && (
        <KnowCardsSection
          key={taskKey(taskRef(panel.task))}
          {...know}
          visible={knowVisible}
          className="sticky bottom-0 mt-auto shrink-0"
        />
      )}
    </div>
  );
}
