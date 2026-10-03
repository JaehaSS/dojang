import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import {
  skillsList,
  skillsRead,
  type SkillMeta,
} from "../lib/ipc";
import { LOCAL_HOST } from "../lib/transport";
import { FilterSegment, type FilterSegmentItem } from "./FilterSegment";
import { MetaTag } from "./MetaTag";
import { HarnessExperiencePanel } from "./HarnessExperiencePanel";
import { useExperimentalFeatures } from "../lib/experimental-features";

/** 에이전트 렌즈 — 값은 벤더 문자열과 같다(`all`만 예외). */
const LENSES = [
  { value: "claude", label: "Claude" },
  { value: "codex", label: "Codex" },
  { value: "antigravity", label: "Agy" },
] as const;

const VENDOR_LABEL: Record<string, string> = {
  claude: "Claude",
  codex: "Codex",
  antigravity: "Agy",
};

type SortKey = "name" | "size";

const SORTS: { value: SortKey; label: string }[] = [
  { value: "name", label: "이름순" },
  { value: "size", label: "큰 순" },
];

/** 확장 시 프롬프트에 들어가는 양. 1k 미만은 그대로 센다. */
function formatBytes(bytes: number): string {
  if (bytes < 1024) return `~${bytes}`;
  return `~${(bytes / 1024).toFixed(1)}k`;
}

/**
 * 값 하나를 고르는 칩+팝오버 (DESIGN.md `components.SelectChip`).
 * 네이티브 `<select>`는 쓰지 않는다 — OS 팝업은 토큰 밖이다.
 */
function SortChip({
  value,
  onChange,
}: {
  value: SortKey;
  onChange: (v: SortKey) => void;
}) {
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const current = SORTS.find((s) => s.value === value) ?? SORTS[0];

  return (
    <div className="relative shrink-0" ref={ref}>
      <button
        className="h-7 px-2 rounded border border-border text-xs text-text-secondary hover:border-border-strong hover:text-text flex items-center gap-1"
        aria-haspopup="listbox"
        aria-expanded={open}
        onClick={() => setOpen((v) => !v)}
      >
        {current.label}
        <span aria-hidden="true">▾</span>
      </button>
      {open && (
        <div
          role="listbox"
          className="absolute right-0 top-full mt-1 z-20 min-w-32 rounded-lg border border-border-strong bg-raised py-1 shadow-xl"
        >
          {SORTS.map((s) => (
            <button
              key={s.value}
              role="option"
              aria-selected={s.value === value}
              className={`w-full text-left px-3 py-1.5 text-xs flex items-center gap-2 ${
                s.value === value ? "text-text" : "text-text-secondary hover:text-text"
              }`}
              onClick={() => {
                onChange(s.value);
                setOpen(false);
              }}
            >
              <span className="w-3 shrink-0">{s.value === value ? "✓" : ""}</span>
              {s.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** 고아 배너 — 옮기는 것은 사용자가 한다. 화면은 경로를 알려 주고 복사해 줄 뿐이다. */
function OrphanBanner({ orphans }: { orphans: SkillMeta[] }) {
  const [copied, setCopied] = useState(false);
  if (orphans.length === 0) return null;

  const names = orphans.map((o) => o.name).join(", ");
  const paths = orphans
    .map((o) => `~/.claude/skills/${o.name}/SKILL.md`)
    .join("\n");

  return (
    <div className="mb-3 rounded-lg border border-border bg-surface p-3 text-xs">
      <div className="text-text">
        <code className="font-code">~/.praxis/skills/</code> 에만 있는 스킬 {orphans.length}개 — {names}
      </div>
      <div className="mt-1 flex items-center justify-between gap-3">
        <span className="text-text-muted">
          거처가 없어 다음 릴리스부터 발동하지 않습니다.{" "}
          <code className="font-code">~/.claude/skills/&lt;이름&gt;/SKILL.md</code> 로 옮기세요.
        </span>
        <button
          className="shrink-0 h-7 px-2 rounded border border-border text-text-secondary hover:text-text hover:border-border-strong"
          onClick={() => {
            void navigator.clipboard?.writeText(paths);
            setCopied(true);
            window.setTimeout(() => setCopied(false), 1500);
          }}
        >
          {copied ? "복사됨" : "경로 복사"}
        </button>
      </div>
    </div>
  );
}

function SkillBody({ repo, name }: { repo: string; name: string }) {
  const [body, setBody] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  useEffect(() => {
    skillsRead(repo, name)
      .then(setBody)
      .catch((e) => setErr(String(e)));
  }, [repo, name]);

  if (err) return <div className="text-status-failed text-xs mt-2">{err}</div>;
  if (body === null) return <div className="text-text-muted text-xs mt-2">불러오는 중…</div>;
  return (
    <pre className="mt-2 pt-2 border-t border-border text-xs font-code text-text-secondary whitespace-pre-wrap break-words">
      {body}
    </pre>
  );
}

interface Props {
  repo: string;
  onOpenMemory?: () => void;
}

/**
 * 스킬 탭 — **뷰어**. 스킬의 정본은 각 에이전트 디렉터리이고 Praxis는 저장하지 않는다.
 * 스킬 목록은 본문 펼침만 제공한다 (설계 0052 §C). 별도 경험 패널은 조회·초안 복사를 제공한다.
 */
export function SkillsView({ repo, onOpenMemory = () => {} }: Props) {
  const advancedFeatures = useExperimentalFeatures();
  const [skills, setSkills] = useState<SkillMeta[]>([]);
  const [err, setErr] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [lens, setLens] = useState("all");
  const [sort, setSort] = useState<SortKey>("name");

  const refresh = useCallback(async () => {
    try {
      // 설정 화면의 스킬 관리는 이 PC 고정 — 미리보기(skillsRead)가 로컬 파일만 읽는다.
      setSkills(await skillsList(LOCAL_HOST, repo));
    } catch (e) {
      setErr(String(e));
    }
  }, [repo]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const orphans = useMemo(() => skills.filter((s) => s.orphan), [skills]);

  // 렌즈 개수는 **소속 기준**이다 — 그 벤더에 거처가 있는 스킬 수.
  const lensCount = useCallback(
    (vendor: string) => skills.filter((s) => s.homes.some((h) => h.vendor === vendor)).length,
    [skills],
  );

  const segments: FilterSegmentItem[] = [
    { value: "all", label: "전체", count: skills.length },
    ...LENSES.map((l) => ({ value: l.value, label: l.label, count: lensCount(l.value) })),
  ];

  // 고른 렌즈가 사라지면(프로젝트 전환) 전체로 되돌린다.
  useEffect(() => {
    if (lens !== "all" && lensCount(lens) === 0) setLens("all");
  }, [lens, lensCount]);

  const visible = useMemo(() => {
    const q = query.trim().toLowerCase();
    let list = skills;
    if (lens !== "all") list = list.filter((s) => s.homes.some((h) => h.vendor === lens));
    if (q) {
      list = list.filter(
        (s) =>
          s.name.toLowerCase().includes(q) || s.description.toLowerCase().includes(q),
      );
    }
    return [...list].sort((a, b) => {
      if (sort === "size") return b.bytes - a.bytes || a.name.localeCompare(b.name);
      return a.name.localeCompare(b.name);
    });
  }, [skills, lens, query, sort]);

  return (
    <div className="flex-1 overflow-auto p-4">
      {err && (
        <div className="text-status-failed text-sm font-code mb-2 max-w-3xl mx-auto">{err}</div>
      )}
      <div className="max-w-3xl mx-auto">
        <OrphanBanner orphans={orphans} />

        <div className="flex items-center gap-2 mb-3">
          <input
            className="flex-1 bg-bg border border-border rounded px-2 py-1 text-sm outline-none focus:border-primary text-text"
            placeholder="이름·설명 검색"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
          <span className="shrink-0 text-xs text-text-muted">{visible.length}개</span>
        </div>

        <div className="flex items-center justify-between gap-2 mb-1">
          <FilterSegment
            label="에이전트 렌즈"
            items={segments}
            value={lens}
            onChange={setLens}
            alwaysVisible={["all"]}
          />
          <SortChip value={sort} onChange={setSort} />
        </div>

        {visible.length === 0 ? (
          <div className="text-text-muted text-sm">보여 줄 스킬이 없습니다.</div>
        ) : (
          <div className="flex flex-col gap-2">
            {visible.map((sk) => {
              const isOpen = expanded === sk.name;
              // 렌즈를 고른 뒤에는 그 에이전트가 어떻게 발동하는지가 정보다.
              const native =
                lens !== "all" && sk.homes.some((h) => h.vendor === lens);
              return (
                <div key={sk.name} className="rounded-lg border border-border bg-surface p-3">
                  <button
                    className="w-full text-left"
                    onClick={() => setExpanded(isOpen ? null : sk.name)}
                  >
                    <div className="flex items-baseline gap-2 flex-wrap">
                      <span className="font-code text-sm text-text">
                        /{sk.name}
                        {sk.argumentHint && (
                          <span className="text-text-muted"> {sk.argumentHint}</span>
                        )}
                      </span>
                      <span className="flex-1" />
                      <MetaTag
                        tone={lens !== "all" && lens !== "claude" ? "accent" : "default"}
                        title="확장 시 프롬프트에 들어가는 양"
                      >
                        {formatBytes(sk.bytes)}
                      </MetaTag>
                      {lens !== "all" && (
                        <MetaTag tone={native ? "default" : "accent"}>
                          {native ? "네이티브" : "Dojang 확장"}
                        </MetaTag>
                      )}
                      {sk.orphan ? (
                        <MetaTag tone="accent">거처 없음</MetaTag>
                      ) : (
                        sk.homes.map((h) => (
                          <MetaTag
                            key={h.path}
                            tone={h.project ? "accent" : "default"}
                            title={h.path}
                          >
                            {VENDOR_LABEL[h.vendor] ?? h.vendor}
                            {h.project ? " · 프로젝트" : ""}
                          </MetaTag>
                        ))
                      )}
                    </div>
                    {sk.description && (
                      <div className="mt-1 text-xs text-text-muted line-clamp-2">
                        {sk.description}
                      </div>
                    )}
                  </button>
                  {isOpen && <SkillBody repo={repo} name={sk.name} />}
                </div>
              );
            })}
          </div>
        )}
        {advancedFeatures && <HarnessExperiencePanel repo={repo} onOpenMemory={onOpenMemory} />}
      </div>
    </div>
  );
}
