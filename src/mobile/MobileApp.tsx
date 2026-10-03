import { useCallback, useEffect, useState } from "react";
import { hrefFor, type Route } from "./routes";
import { linkProps, useRoute } from "./router";
import { blocksApp, useSession } from "./session";
import { api, probeConnection } from "./api";
import { StatusBanner } from "./StatusBanner";
import { TaskListScreen } from "./TaskListScreen";
import { TaskDetailScreen } from "./TaskDetailScreen";
import { NewTaskScreen } from "./NewTaskScreen";
import { SchedulesScreen } from "./SchedulesScreen";
import { SettingsScreen } from "./SettingsScreen";
import { Empty } from "./primitives";
import { actionableCount, type ConnectionState } from "./status";

// 모바일 셸 — 단일 컬럼 + 상태 배너 + 하단 탭바. (설계 0013 §5.3)
// 배너는 항상 보인다. Runner가 죽었는데 화면이 멀쩡해 보이는 것이 가장 나쁜 실패다.

type NavItem = { route: Route; label: string };

const NAV: NavItem[] = [
  { route: { name: "home" }, label: "작업" },
  { route: { name: "new" }, label: "새 작업" },
  { route: { name: "schedules" }, label: "스케줄" },
  { route: { name: "settings" }, label: "설정" },
];

function titleOf(route: Route): string {
  switch (route.name) {
    case "home":
      return "작업";
    case "task":
      return `작업 #${route.id}`;
    case "new":
      return "새 작업";
    case "schedules":
      return "스케줄";
    case "settings":
      return "설정";
    case "notfound":
      return "찾을 수 없음";
  }
}

function Screen({ route, revision }: { route: Route; revision: number }) {
  switch (route.name) {
    case "home":
      return <TaskListScreen revision={revision} />;
    case "task":
      return <TaskDetailScreen id={route.id} tab={route.tab} />;
    case "new":
      return <NewTaskScreen />;
    case "schedules":
      return <SchedulesScreen />;
    case "settings":
      return <SettingsScreen />;
    case "notfound":
      return (
        <Empty>
          <div className="space-y-3">
            <p>없는 경로입니다.</p>
            <a
              {...linkProps(hrefFor({ name: "home" }))}
              className="inline-block rounded-md border border-border px-3 py-2 text-text"
            >
              작업 목록으로
            </a>
          </div>
        </Empty>
      );
  }
}

function BottomNav({ current, badge }: { current: Route; badge: number }) {
  return (
    <nav
      className="flex shrink-0 border-t border-border bg-surface"
      style={{ paddingBottom: "env(safe-area-inset-bottom)" }}
    >
      {NAV.map((item) => {
        const active = item.route.name === current.name;
        const showBadge = item.route.name === "home" && badge > 0;
        return (
          <a
            key={item.route.name}
            {...linkProps(hrefFor(item.route))}
            aria-current={active ? "page" : undefined}
            className={`relative flex-1 py-3 text-center text-xs ${
              active ? "text-primary-bright" : "text-text-muted"
            }`}
          >
            {item.label}
            {showBadge ? (
              <span className="ml-1 rounded-full bg-status-awaiting px-1.5 text-[10px] text-black">
                {badge}
              </span>
            ) : null}
          </a>
        );
      })}
    </nav>
  );
}

export default function MobileApp() {
  const route = useRoute();
  const { state: session, recheck } = useSession();
  const [connection, setConnection] = useState<ConnectionState | null>(null);
  const [revision, setRevision] = useState(0);
  const [pending, setPending] = useState(0);

  const refresh = useCallback(() => {
    recheck();
    setRevision((value) => value + 1);
    void probeConnection().then(setConnection);
  }, [recheck]);

  // 세션이 붙은 뒤에만 상태를 조회한다. 미페어링 상태에서 401을 배너로 띄우면
  // "세션 만료"와 "아직 연결 안 함"이 섞여 보인다.
  useEffect(() => {
    if (session.kind !== "ready") return;
    let cancelled = false;
    void probeConnection().then((result) => {
      if (!cancelled) setConnection(result);
    });
    return () => {
      cancelled = true;
    };
  }, [session.kind, revision]);

  // 검토 대기 배지는 목록과 같은 소스를 봐야 어긋나지 않는다.
  useEffect(() => {
    if (session.kind !== "ready") return;
    let cancelled = false;
    api
      .taskList()
      .then((tasks) => {
        if (!cancelled) setPending(actionableCount(tasks));
      })
      .catch(() => {
        if (!cancelled) setPending(0);
      });
    return () => {
      cancelled = true;
    };
  }, [session.kind, revision]);

  // 포그라운드 복귀·네트워크 복구 시 최신 상태를 다시 본다 — 폰은 오래 잠들어 있다가 돌아온다.
  useEffect(() => {
    const wake = () => {
      if (document.visibilityState === "visible") refresh();
    };
    document.addEventListener("visibilitychange", wake);
    window.addEventListener("online", wake);
    return () => {
      document.removeEventListener("visibilitychange", wake);
      window.removeEventListener("online", wake);
    };
  }, [refresh]);

  // 닿지 못하는 동안에는 스스로 다시 시도한다. 사용자가 버튼을 눌러야만 회복되면,
  // 잠깐 끊긴 것과 정말 죽은 것을 구분할 방법이 없다.
  useEffect(() => {
    if (session.kind !== "unreachable") return;
    const timer = setInterval(() => {
      if (document.visibilityState === "visible") recheck();
    }, 15_000);
    return () => clearInterval(timer);
  }, [session.kind, recheck]);

  // 전에 붙은 적 있는 기기의 일시적 도달 실패로는 화면을 가리지 않는다. 폰이 깨어나는
  // 중이면 첫 요청이 흔히 실패하는데, 그걸로 재페어링을 요구하면 멀쩡한 세션을 버리게 된다.
  const blocked = blocksApp(session);
  const banner: ConnectionState | null =
    session.kind === "ready"
      ? connection
      : session.kind === "unreachable" && !blocked
        ? session.status
          ? { kind: "runner-down", status: session.status }
          : { kind: "offline" }
        : null;

  return (
    <div className="flex h-[100dvh] flex-col bg-bg font-ui text-text">
      <header
        className="flex shrink-0 items-center gap-2 border-b border-border bg-surface px-4 py-3"
        style={{ paddingTop: "calc(env(safe-area-inset-top) + 0.75rem)" }}
      >
        <span className="text-md font-medium">{titleOf(route)}</span>
        <button
          type="button"
          onClick={refresh}
          aria-label="새로고침"
          className="ml-auto min-h-[36px] rounded-md border border-border px-3 text-xs text-text-muted"
        >
          새로고침
        </button>
      </header>

      {/* 화면을 가리지 않는 도달 실패는 배너로 반드시 드러낸다 — 안 그러면 낡은 목록을
          정상으로 오해한다. 이것이 이 앱에서 가장 나쁜 실패다. */}
      {banner ? <StatusBanner state={banner} onRetry={refresh} /> : null}

      <main className="min-h-0 flex-1 overflow-y-auto">
        {session.kind === "checking" ? (
          <Empty>연결을 확인하고 있습니다.</Empty>
        ) : blocked ? (
          <Empty>
            <div className="space-y-4">
              <p>
                {session.kind === "unpaired"
                  ? "이 기기는 아직 연결되지 않았습니다. 데스크톱 Dojang 설정에서 QR을 띄워 스캔하세요."
                  : "Runner에 닿지 못했습니다. Tailscale이 켜져 있는지 확인하세요."}
              </p>
              <button
                type="button"
                onClick={refresh}
                className="rounded-md border border-border px-3 py-2 text-text"
              >
                다시 시도
              </button>
            </div>
          </Empty>
        ) : (
          <Screen route={route} revision={revision} />
        )}
      </main>

      <BottomNav current={route} badge={pending} />
    </div>
  );
}
