import { useState } from "react";
import {
  loadRemoteRunnerProfile,
  probeRemoteRunner,
  saveRemoteRunnerProfile,
} from "../../../lib/transport/remote-host";
import type { RunnerHealth } from "../../../lib/transport/runner";
import { AgentHealthSection } from "../AgentHealthSection";
import { UsageTokenSection } from "../UsageTokenSection";
import { SettingSection, SettingsTabShell } from "./SettingRow";

function RemoteRunnerSection() {
  const initial = loadRemoteRunnerProfile();
  const [hostId, setHostId] = useState(initial.hostId);
  const [endpoint, setEndpoint] = useState(initial.endpoint);
  const [pairingToken, setPairingToken] = useState(initial.pairingToken);
  const [enabled, setEnabled] = useState(initial.enabled);
  const [testing, setTesting] = useState(false);
  const [health, setHealth] = useState<RunnerHealth | null>(null);
  const [message, setMessage] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  const handleTest = async () => {
    setTesting(true);
    setError(null);
    setMessage(null);
    try {
      const result = await probeRemoteRunner(endpoint, pairingToken);
      setHealth(result);
      setMessage(
        `연결 성공 (버전 ${result.version ?? "확인됨"} · 동시 실행 상한 ${result.max_concurrent_tasks})`,
      );
    } catch (err) {
      setHealth(null);
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setTesting(false);
    }
  };

  const handleSave = (nextEnabled: boolean) => {
    setError(null);
    const saved = saveRemoteRunnerProfile({
      hostId,
      endpoint,
      pairingToken,
      enabled: nextEnabled,
    });
    setHostId(saved.hostId);
    setEndpoint(saved.endpoint);
    setPairingToken(saved.pairingToken);
    setEnabled(saved.enabled);
    setMessage(
      saved.enabled
        ? `상시 러너(${saved.hostId})가 연결되었습니다.`
        : "상시 러너 연결이 비활성화되었습니다.",
    );
  };

  return (
    <div className="space-y-3 rounded-lg border border-border bg-surface p-3.5 text-xs">
      <div className="grid grid-cols-1 gap-2.5 sm:grid-cols-2">
        <label className="space-y-1">
          <span className="block text-text-secondary">호스트 식별자</span>
          <input
            type="text"
            value={hostId}
            onChange={(e) => setHostId(e.target.value)}
            placeholder="mini"
            className="w-full rounded-md border border-border bg-bg px-2.5 py-1.5 text-text outline-none focus:border-primary"
          />
        </label>
        <label className="space-y-1">
          <span className="block text-text-secondary">엔드포인트 (Tailscale HTTPS 또는 로컬)</span>
          <input
            type="text"
            value={endpoint}
            onChange={(e) => setEndpoint(e.target.value)}
            placeholder="https://mini.tailnet.ts.net 또는 http://127.0.0.1:47831"
            className="w-full rounded-md border border-border bg-bg px-2.5 py-1.5 font-code text-text outline-none focus:border-primary"
          />
        </label>
      </div>

      <label className="block space-y-1">
        <span className="block text-text-secondary">페어링 토큰 (64자 hex)</span>
        <input
          type="password"
          value={pairingToken}
          onChange={(e) => setPairingToken(e.target.value)}
          placeholder="praxis-runner pairing_token_file 값"
          className="w-full rounded-md border border-border bg-bg px-2.5 py-1.5 font-code text-text outline-none focus:border-primary"
        />
      </label>

      <div className="flex flex-wrap items-center justify-between gap-2 pt-1">
        <label className="flex items-center gap-2 text-text-secondary">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => handleSave(e.target.checked)}
          />
          <span>앱 시작 시 상시 러너 자동 연결</span>
        </label>

        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={testing || !endpoint.trim()}
            onClick={() => void handleTest()}
            className="rounded-md border border-border bg-raised px-3 py-1.5 text-text-secondary hover:text-text disabled:opacity-40"
          >
            {testing ? "확인 중…" : "연결 테스트"}
          </button>
          <button
            type="button"
            disabled={!endpoint.trim() || !pairingToken.trim()}
            onClick={() => handleSave(true)}
            className="rounded-md bg-primary px-3 py-1.5 font-medium text-white disabled:opacity-40"
          >
            저장 및 연결
          </button>
        </div>
      </div>

      {message ? (
        <p className="text-xs text-status-ready">
          {message}
          {health ? ` · 실행 중 ${health.running_tasks ?? 0}건 / 대기 ${health.queued_tasks ?? 0}건` : ""}
        </p>
      ) : null}
      {error ? <p className="text-xs text-status-failed">{error}</p> : null}
    </div>
  );
}

/** 연결 — 상시 러너(미니 PC) 및 에이전트 CLI 상태. 폰 연결은 관리 묶음의 "모바일" 탭이 맡는다. */
export function ConnectionTab() {
  return (
    <SettingsTabShell>
      <SettingSection
        id="remote-runner"
        title="상시 러너 (미니 PC)"
        hint="항시 켜져 있는 미니 PC의 praxis-runner에 연결해 상시 비서 세션과 원격 코딩 작업 큐를 데스크톱·모바일에서 함께 관제한다."
      >
        <RemoteRunnerSection />
      </SettingSection>
      <SettingSection
        id="agent-cli"
        title="에이전트 CLI"
        hint="CLI에 직접 물어 읽는다 — 자격증명 파일 포맷은 벤더가 예고 없이 바꾸지만 CLI 계약은 안정적이다. 인증·버전은 열 때마다 다시 확인하고, 최신 버전 조회만 6시간 캐시한다."
      >
        <AgentHealthSection />
        <UsageTokenSection />
      </SettingSection>
    </SettingsTabShell>
  );
}
