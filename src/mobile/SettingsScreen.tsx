import { useCallback, useEffect, useState } from "react";
import { probeConnection } from "./api";
import { describeConnection, formatAge, type ConnectionState } from "./status";
import { Button, Spinner } from "./primitives";
import { PushCard } from "./PushCard";

// 설정 — 페어링 상태 · 연결 진단. (설계 0013 §5.3)
// 여기서 답해야 하는 질문은 하나다: "지금 안 되는 게 폰 탓인가, Runner 탓인가."
// 그래서 값을 나열하는 대신 판정(배너 문구)과 근거(원시 값)를 같이 보여준다.

function Field({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-start gap-3 px-4 py-2.5">
      <span className="w-28 shrink-0 text-xs text-text-muted">{label}</span>
      <span className="min-w-0 flex-1 break-all font-code text-xs text-text-secondary">
        {value}
      </span>
    </div>
  );
}

const POLICY_LABEL: Record<string, string> = {
  always_approve: "자동 승인",
  require_approval: "승인 필요",
};

export function SettingsScreen() {
  const [connection, setConnection] = useState<ConnectionState | null>(null);
  const [checking, setChecking] = useState(false);
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1000));

  const check = useCallback(() => {
    setChecking(true);
    void probeConnection()
      .then(setConnection)
      .finally(() => {
        setNow(Math.floor(Date.now() / 1000));
        setChecking(false);
      });
  }, []);

  useEffect(() => check(), [check]);

  if (!connection) return <Spinner label="연결을 확인하는 중" />;

  const view = describeConnection(connection, now);
  const health = connection.kind === "ok" ? connection.health : null;

  return (
    <div className="space-y-5 py-4">
      <section className="space-y-2">
        <div className="px-4 text-xs text-text-muted">알림</div>
        <PushCard />
      </section>

      <section className="space-y-2 px-4">
        <div className="text-xs text-text-muted">연결 진단</div>
        <div className="rounded-lg border border-border bg-surface p-3">
          <div
            className={`text-sm ${
              view.tone === "failed"
                ? "text-status-failed"
                : view.tone === "awaiting"
                  ? "text-status-awaiting"
                  : view.tone === "done"
                    ? "text-status-done"
                    : "text-text-muted"
            }`}
          >
            {view.title}
          </div>
          {view.detail ? (
            <div className="mt-1 text-xs text-text-muted">{view.detail}</div>
          ) : null}
        </div>
        <Button disabled={checking} onClick={check}>
          {checking ? "확인 중…" : "다시 확인"}
        </Button>
      </section>

      {health ? (
        <section>
          <div className="px-4 pb-1 text-xs text-text-muted">Runner</div>
          <div className="divide-y divide-border border-y border-border">
            <Field label="버전" value={health.version ?? "—"} />
            <Field label="주소" value={health.bind} />
            <Field
              label="승인 정책"
              value={POLICY_LABEL[health.execution_policy] ?? health.execution_policy}
            />
            <Field label="동시 실행" value={String(health.max_concurrent_tasks)} />
            {health.running_tasks != null ? (
              <Field label="실행 중" value={String(health.running_tasks)} />
            ) : null}
            {health.queued_tasks != null ? (
              <Field label="대기열" value={String(health.queued_tasks)} />
            ) : null}
            {health.uptime_secs != null ? (
              <Field label="가동" value={`${formatAge(health.uptime_secs)}`} />
            ) : null}
            <Field label="보존" value={`${health.retention_days}일`} />
          </div>
        </section>
      ) : null}

      <section>
        <div className="px-4 pb-1 text-xs text-text-muted">이 기기</div>
        <div className="divide-y divide-border border-y border-border">
          <Field label="접속 주소" value={window.location.origin} />
          {/* 세션 토큰은 HttpOnly 쿠키라 JS로 읽을 수 없다 — 그 사실 자체가 안전 신호다. */}
          <Field label="인증" value="HttpOnly 세션 쿠키 (JS에서 읽을 수 없음)" />
        </div>
        <div className="px-4 pt-3 text-xs text-text-muted">
          연결을 끊으려면 데스크톱 Dojang 설정의 <span className="text-text-secondary">모바일 기기</span>{" "}
          목록에서 이 기기를 지우세요.
        </div>
      </section>
    </div>
  );
}
