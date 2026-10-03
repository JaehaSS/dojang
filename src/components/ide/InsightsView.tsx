import { useCallback, useEffect, useMemo, useRef, useState, type ReactElement } from "react";
import {
  insightCards,
  insightsCompute,
  lessonThemes,
  taskPatterns,
  type Insights,
  type RepoLessons,
  type SignalCard,
  type InsightsRange,
  type Task,
  type TaskPatterns,
} from "../../lib/ipc";
import { markSeen } from "../../lib/insight-seen";
import { modelCost, totalCost, fmtUsd } from "../../lib/pricing";
import { useExperimentalFeatures } from "../../lib/experimental-features";
import { cacheReadRate } from "./insights/cache";
import { CacheObservability } from "./insights/CacheObservability";
import { AgentSkillPanel } from "./insights/AgentSkillPanel";
import { AreaChart } from "./insights/AreaChart";
import { DiscoveryCards } from "./insights/DiscoveryCards";
import { Heatmap } from "./insights/Heatmap";
import { Icon } from "./icons";
import { LessonThemes } from "./insights/LessonThemes";
import { Punchcard } from "./insights/Punchcard";
import { PlanCalendar } from "./insights/PlanCalendar";
import { StackedBar } from "./insights/StackedBar";
import { TaskPatternPanel } from "./insights/TaskPatternPanel";
import { DeltaChip, Metric, RankBar, SectionHeader } from "./insights/parts";
import { OutcomeInsightsPanel } from "./OutcomeInsightsPanel";
import {
  deltaPct,
  fmtHour,
  fmtInt,
  fmtPct,
  fmtTokens,
  prettyModel,
  relativeDay,
} from "./insights/format";

const RANGES: { v: InsightsRange; label: string }[] = [
  { v: "all", label: "전체" },
  { v: "30d", label: "30d" },
  { v: "7d", label: "7d" },
];

/**
 * 발견 공간 3섹션 (설계 2026-09-28 §4, 슬라이스 T5).
 *
 * 이전에는 질문 축 4섹션(요약/지출/작업/방식, 설계 0054 DR-1)이었다. 요약 레인(`SummaryLanes`)을
 * 발견 섹션(신호 카드)이 대체하면서 그 자리가 사라졌고, 남은 지출·작업·방식은 "근거 자료" 하나로
 * 묶여 기본 접힌다 — 카드·교훈 주제가 먼저 오고, 손으로 다시 세고 싶을 때만 편다.
 */
const SECTIONS = [
  { id: "discovery", label: "발견" },
  { id: "lessons", label: "교훈 주제" },
  { id: "evidence", label: "근거 자료" },
];

/** 근거 자료 안에 있어 펼쳐야만 스크롤 대상이 보이는 하위 섹션 id. */
const EVIDENCE_SUBSECTIONS = new Set(["cost", "tasks", "how"]);

interface InsightsViewProps {
  /** 계획 섹션에서 정본 편집처(Home)로 보내는 링크. 없으면 링크를 숨긴다. */
  onOpenHome?: () => void;
  /** 발견 카드의 `Tasks` 근거가 가리키는 작업을 찾는 대상(§8, 슬라이스 T4). */
  tasks?: Task[];
  /** 발견 카드에서 작업 근거 행을 눌렀을 때 그 작업을 연다. */
  onOpenTask?: (task: Task) => void;
  /** 발견 카드의 "조사 작업 시작" — 작성기를 문장으로 미리 채우되 전송하지 않는다. */
  onStartResearch?: (instruction: string, repo: string | null) => void;
}

/** 사용량 인사이트 — 발견 → 교훈 주제 → 근거 자료 순의 단일 스크롤 리포트(ADR 0033). */
export function InsightsView({
  onOpenHome,
  tasks = [],
  onOpenTask,
  onStartResearch,
}: InsightsViewProps = {}): ReactElement {
  const advancedFeatures = useExperimentalFeatures();
  const [range, setRange] = useState<InsightsRange>("all");
  const [data, setData] = useState<Insights | null>(null);
  const [patterns, setPatterns] = useState<TaskPatterns | null>(null);
  const [cards, setCards] = useState<SignalCard[]>([]);
  const [lessons, setLessons] = useState<RepoLessons[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [active, setActive] = useState(SECTIONS[0].id);
  // 근거 자료 — 기본 접힘(설계 §4). 발견·교훈 주제가 먼저 읽히게 두고, 다시 세고 싶을 때만 편다.
  const [evidenceOpen, setEvidenceOpen] = useState(false);
  const scrollRef = useRef<HTMLDivElement>(null);
  // 근거 자료 하위 섹션으로 점프했는데 아직 접혀 있으면, 펼침이 반영된 다음 프레임에
  // 스크롤해야 한다 — 접힌 채로는 대상 엘리먼트가 화면에 없다(§4 "펼치고 그 위치로 스크롤").
  const pendingJumpRef = useRef<string | null>(null);

  useEffect(() => {
    let alive = true;
    setErr(null);
    insightsCompute(range)
      .then((d) => alive && setData(d))
      .catch((e) => alive && setErr(String(e)));
    return () => {
      alive = false;
    };
  }, [range]);

  // 작업 패턴은 트랜스크립트가 아니라 작업 DB를 읽는다 — 위 집계가 실패해도 살아 있다.
  useEffect(() => {
    let alive = true;
    taskPatterns(range)
      .then((p) => alive && setPatterns(p))
      .catch(() => alive && setPatterns(null));
    return () => {
      alive = false;
    };
  }, [range]);

  // 발견 공간 신호 카드 — 독립적으로 로드한다. 실패하거나 비어 있으면 섹션 자체가
  // 사라질 뿐, 다른 패널을 막지 않는다.
  useEffect(() => {
    let alive = true;
    // 이전 구간의 카드가 새 구간 문장처럼 남지 않게 먼저 비운다 — S6이 파일 집계를 기다린다.
    setCards([]);
    insightCards(range, -new Date().getTimezoneOffset() * 60)
      .then((c) => {
        if (!alive) return;
        setCards(c);
        // 이 화면을 열어 카드를 봤다는 사실 자체가 "읽음"이다(§4) — 사이드바 점은
        // 이 (signal, repo) 쌍을 더는 새 발견으로 세지 않는다.
        markSeen(c);
      })
      .catch(() => alive && setCards([]));
    return () => {
      alive = false;
    };
  }, [range]);

  // 교훈 주제 — `docs/lessons.json` 투영이 있는 저장소만 온다(§6.3). 신호 카드와 같은 원칙으로
  // 독립 로드하고, 실패·공백이면 섹션 자체가 사라질 뿐 다른 패널을 막지 않는다.
  useEffect(() => {
    let alive = true;
    setLessons([]);
    lessonThemes(range, -new Date().getTimezoneOffset() * 60)
      .then((r) => alive && setLessons(r))
      .catch(() => alive && setLessons([]));
    return () => {
      alive = false;
    };
  }, [range]);

  // 스크롤 스파이 — 단일 스크롤에서도 "지금 어느 섹션인지"를 헤더가 알려준다.
  useEffect(() => {
    const root = scrollRef.current;
    if (!root) return;
    const obs = new IntersectionObserver(
      (entries) => {
        const shown = entries
          .filter((e) => e.isIntersecting)
          .sort((a, b) => a.boundingClientRect.top - b.boundingClientRect.top);
        if (shown[0]) setActive(shown[0].target.id);
      },
      { root, rootMargin: "-72px 0px -55% 0px", threshold: 0 },
    );
    for (const s of SECTIONS) {
      const el = root.querySelector(`#${s.id}`);
      if (el) obs.observe(el);
    }
    return () => obs.disconnect();
  }, [data, patterns]);

  const scrollToId = useCallback((id: string) => {
    scrollRef.current?.querySelector(`#${id}`)?.scrollIntoView({ behavior: "smooth", block: "start" });
  }, []);

  /** 근거 자료 하위 섹션이면 먼저 펼치고, 펼침이 DOM에 반영된 뒤 스크롤한다(§4). */
  const jump = useCallback(
    (id: string) => {
      if (EVIDENCE_SUBSECTIONS.has(id) && !evidenceOpen) {
        pendingJumpRef.current = id;
        setEvidenceOpen(true);
        return;
      }
      scrollToId(id);
    },
    [evidenceOpen, scrollToId],
  );

  useEffect(() => {
    if (!evidenceOpen || pendingJumpRef.current == null) return;
    const id = pendingJumpRef.current;
    pendingJumpRef.current = null;
    // 펼침 직후 한 프레임은 아직 레이아웃이 반영되지 않아 대상이 화면 밖에 있을 수 있다.
    requestAnimationFrame(() => scrollToId(id));
  }, [evidenceOpen, scrollToId]);

  const cost = useMemo(() => (data ? totalCost(data.models) : null), [data]);
  const maxProjectTokens = useMemo(
    () => (data ? Math.max(1, ...data.projects.map((p) => p.total_tokens)) : 1),
    [data],
  );
  const weekendPct = useMemo(() => {
    if (!data || data.weekday_hours.length < 168) return "—";
    const total = data.weekday_hours.reduce((sum, c) => sum + c, 0);
    const weekend = data.weekday_hours
      .slice(0, 24)
      .concat(data.weekday_hours.slice(6 * 24, 7 * 24))
      .reduce((sum, c) => sum + c, 0);
    return fmtPct(weekend, total);
  }, [data]);

  const chip = (on: boolean) =>
    `h-7 px-3 rounded-md text-sm transition-colors ${
      on ? "bg-raised text-primary-bright" : "text-text-secondary hover:text-text"
    }`;

  /** 트랜스크립트 기반 섹션(지출·방식)이 쓸 수 있는 상태인가. */
  const hasUsage = data != null && data.messages > 0;

  return (
    <div ref={scrollRef} className="flex-1 overflow-auto">
      {/* 반투명 + blur는 표 숫자가 비쳐 읽기를 방해한다 — 데이터 화면에선 불투명이 낫다. */}
      <div className="sticky top-0 z-10 bg-bg border-b border-border">
        <div className="max-w-5xl mx-auto px-7 h-14 flex items-center justify-between gap-4">
          <span className="text-sm text-text-secondary shrink-0 hidden sm:block">인사이트</span>
          <div className="flex items-center gap-1">
            {SECTIONS.map((s) => (
              <button key={s.id} className={chip(active === s.id)} onClick={() => jump(s.id)}>
                {s.label}
              </button>
            ))}
          </div>
          <div className="flex items-center gap-1 bg-surface border border-border rounded-md p-0.5 shrink-0">
            {RANGES.map((r) => (
              <button
                key={r.v}
                onClick={() => setRange(r.v)}
                className={`h-6 px-2.5 rounded text-sm ${
                  range === r.v ? "bg-raised text-text" : "text-text-secondary hover:text-text"
                }`}
              >
                {r.label}
              </button>
            ))}
          </div>
        </div>
      </div>

      <div className="max-w-5xl mx-auto px-7 pb-20">
        {/* ── 발견 ────────────────────────────── */}
        <div id="discovery" className="scroll-mt-20">
          <DiscoveryCards
            cards={cards}
            onDismissed={(key) => setCards((cs) => cs.filter((c) => c.key !== key))}
            tasks={tasks}
            onOpenTask={onOpenTask ?? (() => {})}
            onJumpToSpend={() => jump("cost")}
            onStartResearch={onStartResearch ?? (() => {})}
          />
        </div>

        {/* ── 교훈 주제 ────────────────────────── */}
        <div id="lessons" className="scroll-mt-20">
          <LessonThemes repos={lessons} />
        </div>

        {err && <div className="text-status-failed text-sm mt-6 font-code">{err}</div>}

        {/* ── 근거 자료 ────────────────────────────
            현행 지출 · 작업 · 방식 리포트를 한 곳에 묶는다. 발견 · 교훈 주제가 먼저 읽히도록
            기본 접힘이고, `jump()`가 하위 섹션(§ cost·tasks·how)으로 보낼 때만 스스로 편다(§4). */}
        <section className="pt-12">
          <button
            type="button"
            id="evidence"
            data-testid="evidence-toggle"
            onClick={() => setEvidenceOpen((v) => !v)}
            aria-expanded={evidenceOpen}
            className="w-full flex items-center justify-between gap-2 mb-3 scroll-mt-20 text-left"
          >
            <h2 className="text-lg font-semibold">근거 자료</h2>
            <Icon name={evidenceOpen ? "chevronDown" : "chevronRight"} size={14} />
          </button>

          {evidenceOpen && <div data-testid="evidence-body">

        {/* ── 지출 ────────────────────────────── */}
        <section className="pt-0">
          <SectionHeader
            id="cost"
            title="지출"
            hint={
              hasUsage
                ? `입력 ${fmtTokens(data.input_tokens)} · 출력 ${fmtTokens(data.output_tokens)}`
                : undefined
            }
          />
          {data && advancedFeatures && <CacheObservability data={data} />}
          {!hasUsage ? (
            <div className="text-text-muted text-sm border border-border rounded-lg p-8 text-center">
              이 기간에 사용 기록이 없습니다.
            </div>
          ) : (
            <>
              <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
                <div className="flex items-start justify-between gap-4 mb-3">
                  <div>
                    <div className="text-xs text-text-secondary">총 토큰</div>
                    <div className="flex items-baseline gap-2">
                      <span className="text-2xl font-bold" style={{ color: "var(--c-primary-bright)" }}>
                        {fmtTokens(data.total_tokens)}
                      </span>
                      <DeltaChip delta={deltaPct(data.total_tokens, data.prev?.total_tokens)} />
                    </div>
                  </div>
                  <div className="flex items-center gap-3 text-xs text-text-muted pt-1">
                    <span className="flex items-center gap-1.5">
                      <span
                        className="w-2.5 h-2.5 rounded-sm"
                        style={{ background: "var(--c-primary-bright)" }}
                        aria-hidden
                      />
                      토큰
                    </span>
                    <span className="flex items-center gap-1.5">
                      <span className="w-2.5 border-t border-dashed border-text-muted" aria-hidden />
                      메시지
                    </span>
                  </div>
                </div>
                <AreaChart days={data.days} />
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5 mb-2.5">
                <Metric
                  label="세션"
                  value={fmtInt(data.sessions)}
                  delta={deltaPct(data.sessions, data.prev?.sessions)}
                />
                <Metric
                  label="메시지"
                  value={fmtInt(data.messages)}
                  delta={deltaPct(data.messages, data.prev?.messages)}
                />
                <Metric
                  label="활성 일수"
                  value={fmtInt(data.active_days)}
                  delta={deltaPct(data.active_days, data.prev?.active_days)}
                />
                <Metric
                  label={cost?.partial ? "예상 비용 (일부 미산정)" : "예상 비용"}
                  value={cost ? fmtUsd(cost.usd) : "—"}
                />
              </div>

              <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
                <div className="text-xs text-text-secondary mb-2">모델 점유율 (토큰)</div>
                <StackedBar
                  segments={data.models.map((m) => ({
                    key: `${m.provider}:${m.model}`,
                    label: prettyModel(m.model),
                    value: m.total_tokens,
                  }))}
                />
              </div>

              <div className="bg-surface border border-border rounded-lg overflow-x-auto mb-2.5">
                <table className="w-full text-sm min-w-[520px]">
                  <thead>
                    <tr className="text-xs text-text-secondary border-b border-border">
                      <th className="text-left font-medium px-3 py-2">모델</th>
                      <th className="text-right font-medium px-3 py-2">토큰</th>
                      <th className="text-right font-medium px-3 py-2">캐시 읽기 비율</th>
                      <th className="text-right font-medium px-3 py-2">세션</th>
                      <th className="text-right font-medium px-3 py-2">메시지</th>
                      <th className="text-right font-medium px-3 py-2">비용</th>
                    </tr>
                  </thead>
                  <tbody>
                    {data.models.map((m) => {
                      const c = modelCost(m);
                      return (
                        <tr key={`${m.provider}:${m.model}`} className="border-b border-border last:border-0">
                          <td className="px-3 py-2 truncate max-w-[180px]" title={m.model}>
                            {prettyModel(m.model)} <span className="text-xs text-text-muted">{m.provider}</span>
                          </td>
                          <td className="px-3 py-2 text-right font-code">
                            {fmtTokens(m.total_tokens)}
                          </td>
                          <td className="px-3 py-2 text-right font-code text-text-secondary">
                            <span title="관측된 입력 토큰 중 캐시에서 읽은 토큰의 비율">
                              {cacheReadRate(m)}
                            </span>
                            <span className="block text-xs text-text-muted">
                              관측 {fmtInt(m.cache_observed_messages ?? 0)} · 미관측 {fmtInt(m.cache_unknown_messages ?? 0)}
                            </span>
                          </td>
                          <td className="px-3 py-2 text-right font-code text-text-secondary">
                            {fmtInt(m.sessions)}
                          </td>
                          <td className="px-3 py-2 text-right font-code text-text-secondary">
                            {fmtInt(m.messages)}
                          </td>
                          <td className="px-3 py-2 text-right font-code">
                            {c == null ? "—" : fmtUsd(c)}
                          </td>
                        </tr>
                      );
                    })}
                  </tbody>
                </table>
              </div>

              <div className="bg-surface border border-border rounded-lg">
                <div className="text-xs text-text-secondary px-3 pt-3 pb-1">프로젝트</div>
                {data.projects.length === 0 ? (
                  <div className="text-text-muted text-sm px-3 pb-3">프로젝트 정보가 없습니다.</div>
                ) : (
                  data.projects.map((p) => (
                    <div key={p.path} className="px-3 py-2 border-t border-border">
                      <div className="flex items-baseline justify-between gap-3 mb-1.5">
                        <span className="truncate" title={p.path}>
                          {p.name}
                        </span>
                        <span className="text-xs font-code text-text-secondary shrink-0">
                          {fmtTokens(p.total_tokens)} · {fmtInt(p.sessions)}세션 ·{" "}
                          {relativeDay(p.last_active)}
                        </span>
                      </div>
                      <RankBar value={p.total_tokens} max={maxProjectTokens} />
                    </div>
                  ))
                )}
              </div>
            </>
          )}
        </section>

        {/* ── 작업 ──────────────────────────────
            작업 DB만 읽는다. 위 사용량 리포트가 비어 있어도 여기는 채워진다. */}
        <section className="pt-12">
          <SectionHeader id="tasks" title="작업" hint="작업 DB · 트랜스크립트 비의존" />
          <TaskPatternPanel data={patterns} />

          {/* 계획은 사용량이 아니라 day_items를 읽으므로 range 칩과 독립이다(설계 0023).
              작업 섹션 안에 두는 이유는 "무엇을 했나"의 다른 얼굴이기 때문이다.
              백로그(적체)는 홈이 정본이라 여기서 뺐다 — 편집 가능한 화면과 읽기 전용 화면에
              같은 목록이 둘 있으면 어느 쪽을 봐야 하는지 갈린다(1.0 화면 정리). */}
          <div className="pt-8">
            <div className="text-xs text-text-secondary mb-3">계획 · 읽기 전용</div>
            <PlanCalendar onOpenHome={onOpenHome} />
          </div>
        </section>

        {/* ── 방식 ────────────────────────────── */}
        <section className="pt-12">
          <SectionHeader id="how" title="방식" hint="스킬 · 리듬" />
          <AgentSkillPanel range={range} />

          {hasUsage && (
            <div className="pt-8">
              <div className="bg-surface border border-border rounded-lg p-4 mb-2.5">
                <div className="text-xs text-text-secondary mb-3">요일 × 시간</div>
                <Punchcard weekdayHours={data.weekday_hours} />
              </div>
              <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5 mb-2.5">
                <Metric label="현재 연속" value={`${data.current_streak}일`} />
                <Metric label="최장 연속" value={`${data.longest_streak}일`} />
                <Metric label="최다 사용 시간" value={fmtHour(data.peak_hour)} />
                <Metric label="주말 비중" value={weekendPct} />
              </div>
              <div className="bg-surface border border-border rounded-lg p-4">
                <div className="text-xs text-text-secondary mb-3">일별 활동</div>
                <Heatmap days={data.days} range={range} />
              </div>
            </div>
          )}

          {/* AX 결과는 접어둔다. goal_contract를 쓴 작업이 사실상 없어 지표 다수가 0에
              수렴하지만, 지표가 0인 것과 기능이 필요 없는 것은 다르다(설계 0054 DR-8).
              개발자용 실험 지표라 "고급·실험 기능 표시"가 꺼져 있으면 통째로 숨긴다. */}
          {advancedFeatures && (
            <details className="mt-8 border border-border rounded-lg bg-surface">
              <summary className="cursor-pointer select-none px-4 py-2.5 text-sm text-text-secondary">
                AX 결과 — 작업 DB 기반 채택·성과 지표
              </summary>
              <div className="px-4 pb-4">
                <OutcomeInsightsPanel range={range} />
              </div>
            </details>
          )}
        </section>
          </div>}
        </section>
      </div>
    </div>
  );
}
