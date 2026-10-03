import { useCallback, useEffect, useState } from "react";
import { useHostScope } from "../lib/host-scope";
import {
  scheduleList,
  scheduleAdd,
  scheduleRemove,
  scheduleSetEnabled,
  reminderAdd,
  type Schedule,
} from "../lib/ipc";
import { ago, until } from "../lib/fmt";
import { inputCls } from "./ide/formStyles";
import { ToggleBadge, ResourceRow, DeleteButton } from "./ide/ResourceRow";
import { CronBuilder } from "./CronBuilder";
import { TaskPayloadTemplate } from "./TaskPayloadTemplate";
import { CronPreview } from "./CronPreview";
import { getTransport } from "../lib/transport";

const KIND_LABEL: Record<string, string> = { task: "작업", reminder: "리마인더" };

/** 빠른 리마인더 버튼의 분 단위 프리셋. */
const QUICK_DELAYS = [
  { label: "5분 후", minutes: 5 },
  { label: "10분 후", minutes: 10 },
  { label: "30분 후", minutes: 30 },
  { label: "1시간 후", minutes: 60 },
];

/** 스케줄 한 건의 시각 표시 — 1회성(run_at != null)이면 남은시간/완료, 반복이면 cron 원문. */
function ScheduleTiming({ s }: { s: Schedule }) {
  if (s.run_at == null) {
    return <span className="font-code text-text-secondary text-xs w-40 truncate">{s.cron}</span>;
  }
  const nowSec = Math.floor(Date.now() / 1000);
  const fired = !s.enabled || s.run_at <= nowSec;
  const label = fired ? "완료" : `⏰ ${until(s.run_at)} 후`;
  return (
    <span
      className={`font-code text-xs w-40 truncate ${fired ? "text-text-muted" : "text-status-done"}`}
      title={new Date(s.run_at * 1000).toLocaleString()}
    >
      {label}
    </span>
  );
}

/** 텍스트 입력 + 빠른 분 버튼 + 커스텀 분 입력으로 1회성 리마인더 추가. */
function QuickReminder({ onAdded }: { onAdded: () => void }) {
  // 리마인더는 스케줄러가 도는 머신의 것이다 — 스코프가 그 머신을 정한다 (ADR 0133).
  const host = useHostScope();
  const [text, setText] = useState("");
  const [customMinutes, setCustomMinutes] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const canSubmit = text.trim() !== "";

  const submit = async (minutes: number) => {
    if (!canSubmit || minutes <= 0) return;
    setErr(null);
    try {
      await reminderAdd(host, text.trim(), minutes);
      setText("");
      setCustomMinutes("");
      onAdded();
    } catch (e) {
      setErr(String(e));
    }
  };

  return (
    <div className="bg-surface border border-border rounded-lg p-3 mb-4 flex flex-col gap-2">
      <div className="text-sm font-medium">빠른 리마인더</div>
      {err && <div className="text-status-failed text-sm font-code">{err}</div>}
      <input
        className={`${inputCls} w-full`}
        placeholder="리마인더 내용"
        value={text}
        onChange={(e) => setText(e.target.value)}
      />
      <div className="flex flex-wrap gap-2 items-center">
        {QUICK_DELAYS.map(({ label, minutes }) => (
          <button
            key={minutes}
            className="h-8 px-3 rounded-md bg-raised text-text-secondary text-sm hover:text-text disabled:opacity-50"
            disabled={!canSubmit}
            onClick={() => submit(minutes)}
          >
            {label}
          </button>
        ))}
        <input
          className={`${inputCls} w-20`}
          type="number"
          min={1}
          placeholder="분"
          value={customMinutes}
          onChange={(e) => setCustomMinutes(e.target.value)}
        />
        <button
          className="h-8 px-3 rounded-md bg-primary text-bg text-sm font-medium disabled:bg-border disabled:text-text-muted"
          disabled={!canSubmit || Number(customMinutes) <= 0}
          onClick={() => submit(Number(customMinutes))}
        >
          추가
        </button>
      </div>
    </div>
  );
}

/** Phase 3.2 — 크론 예약(스케줄) CRUD. McpServersView 패턴 복제. */
export function SchedulesView() {
  // 스케줄은 그것을 돌리는 머신의 것이다 — 스코프가 그 머신을 정한다 (ADR 0133).
  const host = useHostScope();
  const [schedules, setSchedules] = useState<Schedule[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [label, setLabel] = useState("");
  const [cron, setCron] = useState("");
  const [kind, setKind] = useState<"task" | "reminder">("task");
  const [repo, setRepo] = useState("");
  const [instruction, setInstruction] = useState("");
  const [agent, setAgent] = useState("");
  const [text, setText] = useState("");
  const remote = getTransport(host).kind === "remote";

  // 브라우저 timezone offset (초) — 클라이언트 브라우저 기준으로 계산.
  const tzOffsetSecs = -new Date().getTimezoneOffset() * 60;

  const refresh = useCallback(async () => {
    try {
      setSchedules(await scheduleList(host));
    } catch (e) {
      setErr(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const add = async () => {
    setErr(null);
    try {
      const payload =
        kind === "reminder"
          ? JSON.stringify({ text: text.trim() })
          : JSON.stringify({ repo: repo.trim(), instruction: instruction.trim(), agent: agent.trim() });
      await scheduleAdd(host, label.trim(), cron.trim(), kind, payload, tzOffsetSecs);
      setLabel("");
      setCron("");
      setRepo("");
      setInstruction("");
      setAgent("");
      setText("");
      await refresh();
    } catch (e) {
      setErr(String(e));
    }
  };

  const payloadReady =
    kind === "reminder" ? text.trim() !== "" : repo.trim() !== "" && instruction.trim() !== "";
  const canSubmit = label.trim() !== "" && cron.trim() !== "" && payloadReady;

  return (
    <div className="flex-1 overflow-auto p-4">
      {err && <div className="text-status-failed text-sm font-code mb-2">{err}</div>}
      <div className="max-w-3xl mx-auto">
        <div className="text-text-muted text-xs mb-3">
          빠른 리마인더로 N분 후 1회성 알림을 등록하거나, 아래 고급 설정에서 cron 반복 예약(작업 자동 실행/텔레그램 리마인더)을 등록합니다.
        </div>
        {remote && (
          <div className="text-text-muted text-xs mb-3">
            원격 Runner 스케줄 · 출력/이벤트 보관은 60일이며, 삭제는 Runner에 즉시 반영됩니다.
          </div>
        )}

        <QuickReminder onAdded={refresh} />

        <div className="text-text-muted text-xs mb-2">고급 · 반복 예약</div>
        <div className="bg-surface border border-border rounded-lg p-4 mb-4 flex flex-col gap-4">
          <div className="flex flex-col gap-2">
            <label className="text-xs text-text-muted">이름</label>
            <input
              className={`${inputCls}`}
              placeholder="예: 일일 테스트"
              value={label}
              onChange={(e) => setLabel(e.target.value)}
            />
          </div>

          <div className="flex flex-col gap-2">
            <label className="text-xs text-text-muted">반복 패턴 (Cron)</label>
            <CronBuilder cron={cron} onCronChange={setCron} tz_offset_secs={tzOffsetSecs} />
          </div>

          <CronPreview cron={cron} tz_offset_secs={tzOffsetSecs} />

          <div className="flex flex-col gap-2">
            <label className="text-xs text-text-muted">실행 종류</label>
            <select
              className={inputCls}
              value={kind}
              onChange={(e) => setKind(e.target.value as "task" | "reminder")}
            >
              <option value="task">자동 작업 실행</option>
              <option value="reminder">리마인더 알림</option>
            </select>
          </div>

          <div className="flex flex-col gap-2">
            <label className="text-xs text-text-muted">
              {kind === "reminder" ? "알림 내용" : "작업 설정"}
            </label>
            <TaskPayloadTemplate
              kind={kind}
              repo={repo}
              instruction={instruction}
              agent={agent}
              text={text}
              onApply={(payload) => {
                if ("text" in payload) {
                  setText(payload.text);
                } else {
                  setRepo(payload.repo);
                  setInstruction(payload.instruction);
                  setAgent(payload.agent);
                }
              }}
            />
          </div>

          <button
            className="h-9 px-4 rounded-md bg-primary text-bg text-sm font-medium disabled:bg-border disabled:text-text-muted"
            disabled={!canSubmit}
            onClick={add}
          >
            스케줄 추가
          </button>
        </div>

        {schedules.length === 0 ? (
          <div className="text-text-muted text-center py-8">등록된 스케줄이 없습니다.</div>
        ) : (
          <div className="flex flex-col gap-2">
            {schedules.map((s) => (
              <ResourceRow key={s.id}>
                <ToggleBadge enabled={s.enabled} onToggle={() => scheduleSetEnabled(host, s.id, !s.enabled).then(refresh)} />
                <span className="font-medium text-sm w-28 truncate">{s.label}</span>
                <ScheduleTiming s={s} />
                <span className="text-xs px-2 py-0.5 rounded bg-raised text-text-secondary shrink-0">
                  {KIND_LABEL[s.kind] ?? s.kind}
                </span>
                <span className="text-text-muted text-xs flex-1 truncate">
                  {s.last_run_at != null ? `마지막 실행 ${ago(s.last_run_at)} 전` : "미실행"}
                </span>
                <DeleteButton
                  onRemove={() => {
                    if (!window.confirm(`스케줄 “${s.label}”을 삭제할까요?`)) return;
                    void scheduleRemove(host, s.id).then(refresh).catch((error) => setErr(String(error)));
                  }}
                />
              </ResourceRow>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}
