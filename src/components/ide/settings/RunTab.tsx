import { useEffect, useState } from "react";
import {
  captureProfileGet,
  captureProfileSet,
  type CaptureProfile,
  blockUnverifiedGet,
  blockUnverifiedSet,
  lspAutoinjectGet,
  lspAutoinjectSet,
  useWorktreeGet,
  useWorktreeSet,
  refreshBaseGet,
  refreshBaseSet,
  maxConcurrentGet,
  maxConcurrentSet,
  defaultShell,
  agentModelsGet,
  agentModelSet,
  debateRoundCapGet,
  debateRoundCapSet,
  type ShellSpec,
  type ConcurrencyLimit,
} from "../../../lib/ipc";
import { modelsForAgentWithObserved, AGENT_MODEL_CATALOG } from "../../../lib/models";
import { useObservedModels } from "../../../lib/use-observed-models";
import { AGENT_PRESETS } from "../../../lib/agents";
import { SettingRow, SettingSection, SettingsTabShell, Switch, TabSummary } from "./SettingRow";
import { setSessionStyle, useSessionStyle, type SessionStyle } from "../../../lib/session-style";
import { TranslateSection } from "./TranslateSection";

/** 토론 라운드 상한 — 백엔드 `debate::DEFAULT_ROUND_CAP`·`ROUND_CAP_RANGE`와 같은 값이다. */
const DEBATE_ROUND_CAP_DEFAULT = 3;
const DEBATE_ROUND_CAP_MIN = 2;
const DEBATE_ROUND_CAP_MAX = 5;

/** 캡처 프로파일 입력의 후보 — 작업 실행과 같은 카탈로그를 쓴다. 목록 밖 값도 허용하되
 *  경고만 띄운다(자유입력이 폴백이라는 카탈로그의 전제를 그대로 따른다). */
const CLAUDE_MODEL_IDS = new Set((AGENT_MODEL_CATALOG.claude ?? []).map((m) => m.id));
/** claude CLI `--effort`는 모델과 무관하게 전역 검증된다. 최종 검증은 백엔드가 한다. */
const CAPTURE_EFFORTS =
  (AGENT_MODEL_CATALOG.claude ?? []).find((m) => m.reasoningEfforts?.length)?.reasoningEfforts ?? [];

const numberInput =
  "w-16 bg-bg border border-border rounded px-2 py-1 text-sm text-text outline-none focus:border-primary disabled:opacity-50";

interface Props {
  onUseWorktreeChange: (on: boolean) => void;
}

/** 작업 실행 — 모델·동시 실행·작업 생성 기본값처럼 "작업이 어떻게 도는가"를 정하는 설정. */
export function RunTab({ onUseWorktreeChange }: Props) {
  const observedModelList = useObservedModels();
  const [captureProfile, setCaptureProfile] = useState<CaptureProfile | null>(null);
  const style = useSessionStyle();
  const [blockUnverified, setBlockUnverified] = useState(false);
  const [lspAutoinject, setLspAutoinject] = useState(true);
  const [useWorktree, setUseWorktree] = useState(true);
  const [refreshBase, setRefreshBase] = useState(true);
  // 동시 실행 상한 — 값·경계는 백엔드가 준다. 입력 중에는 문자열로 들고 있어야
  // 지웠다 다시 치는 중간 상태("")가 0으로 튀지 않는다. 확정은 blur/Enter.
  const [concurrency, setConcurrency] = useState<ConcurrencyLimit | null>(null);
  const [concurrencyInput, setConcurrencyInput] = useState("");
  const [shell, setShell] = useState<ShellSpec | null>(null);
  const [agentModels, setAgentModels] = useState<Record<string, string>>({});
  // 라운드 상한 — 백엔드가 범위 밖을 거부하므로 여기서도 클램프하지 않는다. 밖이면 보내지 않고 되돌린다.
  const [roundCap, setRoundCap] = useState(DEBATE_ROUND_CAP_DEFAULT);
  const [roundCapInput, setRoundCapInput] = useState(String(DEBATE_ROUND_CAP_DEFAULT));
  const [roundCapStatus, setRoundCapStatus] = useState<"saved" | "error" | "range" | undefined>(undefined);
  const [modelStatus, setModelStatus] = useState<Record<string, "saved" | "error" | undefined>>({});

  useEffect(() => {
    captureProfileGet().then(setCaptureProfile).catch(() => {});
    blockUnverifiedGet().then(setBlockUnverified).catch(() => {});
    lspAutoinjectGet().then(setLspAutoinject).catch(() => {});
    refreshBaseGet().then(setRefreshBase).catch(() => {});
    useWorktreeGet()
      .then((on) => {
        setUseWorktree(on);
        onUseWorktreeChange(on);
      })
      .catch(() => {});
    maxConcurrentGet()
      .then((limit) => {
        setConcurrency(limit);
        setConcurrencyInput(String(limit.value));
      })
      .catch(() => {});
    defaultShell().then(setShell).catch(() => {});
    debateRoundCapGet()
      .then((cap) => {
        setRoundCap(cap);
        setRoundCapInput(String(cap));
      })
      .catch(() => {});
    agentModelsGet().then(setAgentModels).catch(() => {});
  }, []);

  /** 프로파일 저장 — 실패하면 서버 상태를 다시 읽어 화면과 어긋난 채 두지 않는다. */
  const saveCaptureProfile = async (patch: Partial<CaptureProfile>) => {
    if (!captureProfile) return;
    const next = { ...captureProfile, ...patch };
    setCaptureProfile(next);
    try {
      await captureProfileSet(next.model_raw, next.effort_raw, next.lean);
      setCaptureProfile(await captureProfileGet());
    } catch {
      captureProfileGet().then(setCaptureProfile).catch(() => {});
    }
  };

  /** 켜고 끄는 설정은 실패하면 되돌린다 — 화면이 서버보다 앞서 있으면 안 된다. */
  const toggle = async (
    current: boolean,
    apply: (next: boolean) => Promise<unknown>,
    set: (next: boolean) => void,
  ) => {
    const next = !current;
    set(next);
    try {
      await apply(next);
    } catch {
      set(!next);
    }
  };

  const toggleUseWorktree = async () => {
    const next = !useWorktree;
    setUseWorktree(next);
    try {
      await useWorktreeSet(next);
      onUseWorktreeChange(next);
    } catch {
      setUseWorktree(!next);
    }
  };

  // 동시 실행 상한 확정(blur/Enter). 빈 입력·비정수는 저장하지 않고 직전 값으로 되돌린다.
  // 범위는 백엔드가 준 경계로 보내기 전에 접는다 — 상한은 `usize`라 음수를 그대로 보내면
  // 역직렬화 에러로 떨어질 뿐 사용자에게 아무것도 알려주지 못한다.
  // 화면은 요청값이 아니라 백엔드가 돌려준 값으로 갱신해 입력칸과 실제 상한을 일치시킨다.
  const commitConcurrency = async () => {
    if (!concurrency) return;
    const raw = concurrencyInput.trim();
    const parsed = Number(raw);
    if (raw === "" || !Number.isInteger(parsed)) {
      setConcurrencyInput(String(concurrency.value));
      return;
    }
    const next = Math.min(concurrency.max, Math.max(concurrency.min, parsed));
    if (next === concurrency.value) {
      setConcurrencyInput(String(concurrency.value));
      return;
    }
    try {
      const applied = await maxConcurrentSet(next);
      setConcurrency(applied);
      setConcurrencyInput(String(applied.value));
    } catch {
      setConcurrencyInput(String(concurrency.value));
    }
  };

  const saveRoundCap = async () => {
    const parsed = Number(roundCapInput.trim());
    if (parsed === roundCap) return setRoundCapInput(String(roundCap));
    if (!Number.isInteger(parsed) || parsed < DEBATE_ROUND_CAP_MIN || parsed > DEBATE_ROUND_CAP_MAX) {
      // 되돌리기만 하면 왜 안 먹었는지 말하지 않는다 — 클램프해 삼키지도 않는다.
      setRoundCapInput(String(roundCap));
      setRoundCapStatus("range");
      setTimeout(() => setRoundCapStatus(undefined), 1500);
      return;
    }
    try {
      await debateRoundCapSet(parsed);
      setRoundCap(parsed);
      setRoundCapStatus("saved");
      setTimeout(() => setRoundCapStatus(undefined), 1500);
    } catch {
      setRoundCapInput(String(roundCap));
      setRoundCapStatus("error");
    }
  };

  const saveAgentModel = async (agent: string, model: string) => {
    const trimmed = model.trim();
    if ((agentModels[agent] ?? "") === trimmed) return;
    try {
      await agentModelSet(agent, trimmed);
      setAgentModels((prev) => ({ ...prev, [agent]: trimmed }));
      setModelStatus((prev) => ({ ...prev, [agent]: "saved" }));
      setTimeout(() => setModelStatus((prev) => ({ ...prev, [agent]: undefined })), 1500);
    } catch {
      setModelStatus((prev) => ({ ...prev, [agent]: "error" }));
    }
  };

  const onOff = (on: boolean) => (on ? "켜짐" : "꺼짐");

  return (
    <SettingsTabShell>
      <TabSummary
        items={[
          `동시 실행 ${concurrency ? concurrency.value : "…"}`,
          `워크트리 격리 ${onOff(useWorktree)}`,
          `base 최신화 ${onOff(refreshBase)}`,
          `Approve 차단 ${onOff(blockUnverified)}`,
          `LSP 자동 연결 ${onOff(lspAutoinject)}`,
          `토론 라운드 ${roundCap}`,
        ]}
      />

      <SettingSection
        id="agent-models"
        title="에이전트 모델"
        risk="cost"
        hint="작업/대화 실행 시 해당 CLI에 --model/-m 플래그로 전달됩니다. 비워두면 CLI 기본 모델을 사용합니다."
      >
        {AGENT_PRESETS.map((p) => (
          <SettingRow key={p.key} title={p.label}>
            {modelStatus[p.key] === "saved" && <span className="text-xs text-primary-bright">저장됨</span>}
            {modelStatus[p.key] === "error" && <span className="text-xs text-status-failed">저장 실패</span>}
            <input
              aria-label={`${p.label} 모델`}
              className="w-48 bg-bg border border-border rounded px-2 py-1 text-sm text-text outline-none focus:border-primary placeholder:text-text-muted font-code"
              placeholder="CLI 기본값 사용"
              list={`praxis-models-${p.key}`}
              defaultValue={agentModels[p.key] ?? ""}
              key={agentModels[p.key] ?? ""}
              onBlur={(e) => saveAgentModel(p.key, e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
              }}
            />
            <datalist id={`praxis-models-${p.key}`}>
              {modelsForAgentWithObserved(p.key, observedModelList).map((m) => (
                <option key={m.id} value={m.id} label={m.label} />
              ))}
            </datalist>
          </SettingRow>
        ))}

        <SettingRow
          id="debate-round-cap"
          title="토론 라운드 상한"
          risk="cost"
          hint={`한 발화가 여는 라운드 수의 상한(${DEBATE_ROUND_CAP_MIN}~${DEBATE_ROUND_CAP_MAX}, 기본 ${DEBATE_ROUND_CAP_DEFAULT}). 라운드마다 두 에이전트가 한 번씩 말하므로 벤더 호출은 그 두 배입니다.`}
        >
          {roundCapStatus === "saved" && <span className="text-xs text-primary-bright">저장됨</span>}
          {roundCapStatus === "error" && <span className="text-xs text-status-failed">저장 실패</span>}
          {roundCapStatus === "range" && (
            <span className="text-xs text-status-failed">
              {DEBATE_ROUND_CAP_MIN}~{DEBATE_ROUND_CAP_MAX}만 저장합니다
            </span>
          )}
          <input
            type="number"
            aria-label="토론 라운드 상한"
            min={DEBATE_ROUND_CAP_MIN}
            max={DEBATE_ROUND_CAP_MAX}
            className={numberInput}
            value={roundCapInput}
            onChange={(e) => setRoundCapInput(e.target.value)}
            onBlur={() => void saveRoundCap()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        </SettingRow>
      </SettingSection>

      <SettingSection
        id="task-defaults"
        title="작업 생성 기본값"
        hint="새 작업을 만들 때 적용되는 값입니다. 작업마다 따로 덮어쓸 수 있는 것도 있습니다."
      >
        <SettingRow
          id="session-style"
          title="세션 방식"
          hint="대화: 턴마다 CLI를 실행해 말풍선으로 보여 주고 앱 컴포저로 입력합니다. 터미널: 실제 CLI를 앱 터미널에 띄우고 터미널 안에서만 입력합니다(작업 중 입력·Esc 중단·Shift+Tab 권한 모드·슬래시 명령). 터미널에서는 @파일 멘션·캡처 첨부·질문 카드를 쓸 수 없습니다. 로컬에서 Claude나 Codex 하나로 새로 시작하는 작업에만 적용되고, 이미 만든 작업은 만들 때의 방식을 유지합니다."
        >
          <select
            aria-label="세션 방식"
            className="bg-bg border border-border rounded px-2 py-1 text-xs text-text"
            value={style}
            onChange={(e) => setSessionStyle(e.target.value as SessionStyle)}
          >
            <option value="conversation">대화</option>
            <option value="terminal">터미널 (CLI 그대로)</option>
          </select>
        </SettingRow>

        <SettingRow
          id="use-worktree"
          title="워크트리 격리"
          risk="safety"
          hint="끄면 새 작업이 메인 체크아웃에서 직접 실행됨 — 비교 실행/외부 작업은 항상 격리. 프로젝트별로 다르게 쓰려면 홈 컴포저의 격리 칩을 눌러 이 기본값을 덮어쓰세요."
        >
          <Switch on={useWorktree} onClick={() => void toggleUseWorktree()} label="워크트리 격리" />
        </SettingRow>

        <SettingRow
          id="refresh-base"
          title="base 브랜치 최신화"
          hint="워크트리를 만들기 전에 고른 base를 원격 최신으로 맞춥니다(fast-forward만). 갈라져 있으면 손대지 않고, 원격에 못 닿아도 작업 생성은 그대로 진행됩니다."
        >
          <Switch
            on={refreshBase}
            onClick={() => void toggle(refreshBase, refreshBaseSet, setRefreshBase)}
            label="base 브랜치 최신화"
          />
        </SettingRow>

        <SettingRow
          id="lsp-autoinject"
          title="LSP 자동 연결"
          hint="작업 생성 시 워크트리 언어(Cargo.toml/tsconfig/pyproject)를 감지해 해당 LSP-MCP 브리지를 에이전트 .mcp.json에 자동 추가합니다. language server(rust-analyzer 등)가 설치돼 있어야 동작."
        >
          <Switch
            on={lspAutoinject}
            onClick={() => void toggle(lspAutoinject, lspAutoinjectSet, setLspAutoinject)}
            label="LSP 자동 연결"
          />
        </SettingRow>

        <SettingRow
          id="block-unverified"
          title="검증 실패 시 Approve 차단 (opt-in)"
          risk="safety"
          hint="켜면 Verify 통과(ready) 전에는 Approve & Merge가 막힙니다 (기본 OFF=자문)"
        >
          <Switch
            on={blockUnverified}
            onClick={() => void toggle(blockUnverified, blockUnverifiedSet, setBlockUnverified)}
            label="검증 실패 시 Approve 차단"
          />
        </SettingRow>
      </SettingSection>

      <SettingSection
        id="run-environment"
        title="실행 환경"
        hint="에이전트 프로세스가 도는 조건입니다."
      >
        <SettingRow
          id="max-concurrent"
          title="동시 실행 세션 수"
          risk="cost"
          hint={`한 번에 진행할 수 있는 작업 수 (기본 8, ${concurrency?.min ?? 1}~${concurrency?.max ?? 64}). 작업마다 CLI 프로세스와 워크트리가 따로 뜨므로 머신 사양과 API 사용량을 보고 올리세요. 줄여도 실행 중인 작업은 그대로 두고 새 작업만 막힙니다.`}
        >
          <input
            type="number"
            aria-label="동시 실행 세션 수"
            min={concurrency?.min ?? 1}
            max={concurrency?.max ?? 64}
            disabled={!concurrency}
            className={numberInput}
            value={concurrencyInput}
            onChange={(e) => setConcurrencyInput(e.target.value)}
            onBlur={() => void commitConcurrency()}
            onKeyDown={(e) => e.key === "Enter" && e.currentTarget.blur()}
          />
        </SettingRow>

        <SettingRow id="default-shell" title="기본 셸" hint="에이전트 PTY 실행 셸">
          <span className="font-code text-sm text-text-secondary">
            {shell ? `${shell.cmd} ${shell.args.join(" ")}` : "…"}
          </span>
        </SettingRow>
      </SettingSection>

      {captureProfile && (
        <SettingSection
          id="capture-profile"
          title="캡처·회고 실행 프로파일"
          risk="cost"
          hint="이 호출들은 도구 없이 돈다. 지정하지 않으면 아래 실효값이 쓰인다 — CLI 기본값을 상속하지 않는다."
        >
          <div className="flex items-center gap-2">
            <label className="text-xs text-text-muted w-16 shrink-0" htmlFor="capture-model">
              모델
            </label>
            <input
              id="capture-model"
              className="flex-1 min-w-0 bg-bg border border-border rounded px-2 py-1 text-xs text-text"
              placeholder={`미지정 — ${captureProfile.model}`}
              value={captureProfile.model_raw}
              onChange={(e) => setCaptureProfile({ ...captureProfile, model_raw: e.target.value })}
              onBlur={(e) => saveCaptureProfile({ model_raw: e.target.value })}
            />
            <span className="text-xs text-text-muted shrink-0">실효: {captureProfile.model}</span>
          </div>
          {captureProfile.model_raw.trim() !== "" && !CLAUDE_MODEL_IDS.has(captureProfile.model_raw.trim()) && (
            <div className="text-xs text-status-failed">
              확인되지 않은 모델입니다 — 오타면 호출이 조용히 실패합니다.
            </div>
          )}

          <div className="flex items-center gap-2">
            <label className="text-xs text-text-muted w-16 shrink-0" htmlFor="capture-effort">
              effort
            </label>
            <select
              id="capture-effort"
              className="flex-1 min-w-0 bg-bg border border-border rounded px-2 py-1 text-xs text-text"
              value={captureProfile.effort_raw}
              onChange={(e) => saveCaptureProfile({ effort_raw: e.target.value })}
            >
              <option value="">미지정 ({captureProfile.effort})</option>
              {CAPTURE_EFFORTS.map((e) => (
                <option key={e} value={e}>
                  {e}
                </option>
              ))}
            </select>
          </div>

          <SettingRow
            title="린 인보케이션"
            hint="끄면 시스템 프롬프트·출력 형식·설정 소스를 되돌린다. 비용이 오르고 측정도 멈춘다 — 문제가 있을 때만"
          >
            <Switch
              on={captureProfile.lean}
              onClick={() => saveCaptureProfile({ lean: !captureProfile.lean })}
              label="린 인보케이션"
            />
          </SettingRow>
        </SettingSection>
      )}

      <TranslateSection />
    </SettingsTabShell>
  );
}
