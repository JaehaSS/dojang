import { useCallback, useEffect, useState } from "react";
import {
  knowledgeVaultsGet,
  knowledgeVaultsSet,
  knowledgeSync,
  type KnowledgeVaultEntry,
  type KnowledgeSyncResult,
} from "../lib/ipc";
import { useTheme } from "../lib/use-theme";
import { inputCls } from "./ide/formStyles";
import { DirectoryPickerModal } from "./ide/DirectoryPickerModal";
import { GmailSourceSection } from "./GmailSourceSection";
import { WikiDocumentsPanel } from "./WikiDocumentsPanel";
import { InsightSection } from "./ide/settings/InsightSection";

/**
 * 새 vault의 기본 임베딩 제외.
 *
 * 실측한 vault는 청크의 98%가 `Claude Code/` 세션 로그였고, 전량 임베딩에 ~6시간이 든다.
 * 기본값이 없으면 사용자는 첫 동기화에서 그 비용을 그대로 맞는다.
 * **색인은 그대로 되므로 어휘 검색으로는 계속 찾힌다** — 의미 검색에서만 빠진다.
 */
const DEFAULT_EMBED_EXCLUDE = ["Claude Code/**"];

/** Wiki › 연결 — Obsidian vault 연결과 동기화. 원래 설정 › 지식 그래프였다(1.0 화면 정리). */
export function KnowledgeView() {
  const [vaults, setVaults] = useState<KnowledgeVaultEntry[]>([]);
  // 구 위키의 "문서 자료" — Wiki 채널에서 내려와 여기 붙었다(계획 2026-09-13). 펼칠 때 처음 붙인다:
  // 색인 문서가 수천 건이라 설정을 여는 것만으로 그 목록을 읽게 두지 않는다.
  const [browsing, setBrowsing] = useState(false);
  const dark = useTheme().kind !== "light";
  const [picking, setPicking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<KnowledgeSyncResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    knowledgeVaultsGet()
      .then((cfg) => setVaults(cfg.vaults ?? []))
      .catch(() => setVaults([]));
  }, []);

  const persist = useCallback(async (next: KnowledgeVaultEntry[]) => {
    setVaults(next);
    try {
      await knowledgeVaultsSet(next);
      setError(null);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  const addVault = (root: string) => {
    setPicking(false);
    // 중복은 UNIQUE 충돌이 아니라 무의미한 재스캔이다. 조용히 무시한다.
    if (vaults.some((v) => v.root === root)) return;
    void persist([
      ...vaults,
      { root, exclude: [], embed_exclude: [...DEFAULT_EMBED_EXCLUDE] },
    ]);
  };

  const patch = (i: number, field: "exclude" | "embed_exclude", raw: string) => {
    const globs = raw
      .split(",")
      .map((s) => s.trim())
      .filter(Boolean);
    void persist(vaults.map((v, n) => (n === i ? { ...v, [field]: globs } : v)));
  };

  const runSync = async () => {
    setBusy(true);
    setError(null);
    setResult(null);
    try {
      setResult(await knowledgeSync());
    } catch (e) {
      // 조용히 실패하면 사용자는 검색이 안 되는 이유를 알 수 없다.
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    // 설정 › 지식 탭이 바깥에서 스크롤·여백(overflow-auto p-4)을 맡는다 — 여기서
    // 다시 flex-1/overflow-auto/p-6을 쓰면 이중 여백이 된다.
    <div>
      <div className="max-w-3xl mx-auto">
        <h2 className="text-lg font-semibold mb-1">지식 그래프</h2>
        <p className="text-sm text-text-muted mb-4">
          Obsidian vault를 색인해 컴포저의 <span className="font-code">@</span>멘션에서 찾습니다.
          원문·색인·검색은 모두 이 기기 안에 머뭅니다.
        </p>

        {vaults.length === 0 && (
          <div className="text-sm text-text-secondary border border-border rounded-md px-3 py-6 text-center mb-3">
            연결된 vault가 없습니다.
          </div>
        )}

        {vaults.map((v, i) => (
          <div key={v.root} className="border border-border rounded-md p-3 mb-2">
            <div className="flex items-center justify-between gap-2 mb-2">
              <div className="text-sm font-code truncate" title={v.root}>
                {v.root}
              </div>
              <button
                className="shrink-0 text-xs text-text-muted hover:text-danger px-2 py-1"
                onClick={() => void persist(vaults.filter((_, n) => n !== i))}
              >
                제거
              </button>
            </div>
            <label className="flex items-center gap-2 py-1">
              <span className="w-28 shrink-0 text-xs text-text-secondary">색인 제외</span>
              <input
                className={inputCls}
                defaultValue={v.exclude.join(", ")}
                placeholder="Templates/**, *.excalidraw.md"
                onBlur={(e) => patch(i, "exclude", e.target.value)}
              />
            </label>
            <label className="flex items-center gap-2 py-1">
              <span className="w-28 shrink-0 text-xs text-text-secondary">임베딩만 제외</span>
              <input
                className={inputCls}
                defaultValue={v.embed_exclude.join(", ")}
                placeholder="Claude Code/**"
                onBlur={(e) => patch(i, "embed_exclude", e.target.value)}
              />
            </label>
            <p className="text-xs text-text-muted mt-1">
              임베딩만 제외한 폴더도 <b>어휘 검색으로는 계속 찾힙니다</b> — 의미 검색에서만
              빠집니다. 임베딩은 청크당 약 0.2초라 큰 폴더는 몇 시간이 걸립니다.
            </p>
          </div>
        ))}

        <div className="flex items-center gap-2 mt-3">
          <button
            className="h-8 px-3 rounded-md text-sm border border-border text-text-secondary hover:text-text"
            onClick={() => setPicking(true)}
          >
            폴더 추가
          </button>
          <button
            className="h-8 px-3 rounded-md text-sm bg-primary text-white disabled:opacity-40"
            disabled={busy || vaults.length === 0}
            onClick={() => void runSync()}
          >
            {busy ? "동기화 중…" : "동기화"}
          </button>
        </div>

        {result && (
          <div className="text-sm text-text-secondary mt-3">
            색인 {result.indexed} · 변경 없음 {result.skipped} · 삭제 {result.deleted} · 링크{" "}
            {result.edges} · 임베딩 {result.embedded}
          </div>
        )}
        {error && <div className="text-sm text-danger mt-3 whitespace-pre-wrap">{error}</div>}

        {/* 개인 지식창고 문서를 대기 카드로 — 창고 연결 자체는 WIKI 화면이 맡는다. */}
        <div className="mt-6 border-t border-border pt-4">
          <InsightSection />
        </div>

        <GmailSourceSection />

        <section className="mt-6 border-t border-border pt-4">
          <div className="flex items-center justify-between gap-2">
            <div>
              <h3 className="text-sm font-semibold">문서 자료</h3>
              <p className="text-xs text-text-muted mt-0.5">
                색인한 vault 문서를 폴더·검색으로 둘러봅니다.
              </p>
            </div>
            <button
              className="h-8 px-3 rounded-md text-sm border border-border text-text-secondary hover:text-text"
              aria-expanded={browsing}
              onClick={() => setBrowsing((v) => !v)}
            >
              {browsing ? "접기" : "둘러보기"}
            </button>
          </div>
          {browsing && (
            <div className="mt-3 h-[28rem] flex border border-border rounded-md overflow-hidden">
              <WikiDocumentsPanel canAttach={false} dark={dark} onAttach={async () => {}} />
            </div>
          )}
        </section>

        {picking && (
          <DirectoryPickerModal
            initialPath={vaults[0]?.root ?? ""}
            onPick={addVault}
            onClose={() => setPicking(false)}
          />
        )}
      </div>
    </div>
  );
}
