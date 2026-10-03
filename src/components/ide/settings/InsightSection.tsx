import { useCallback, useEffect, useState } from "react";

import {
  insightAvailability,
  insightEnabledGet,
  insightEnabledSet,
  insightWikiFoldersSet,
  type InsightAvailability,
  type InsightWikiFolder,
} from "../../../lib/ipc";
import { InsightCard } from "../../InsightCard";
import { SettingRow, SettingSection, Switch } from "./SettingRow";

/**
 * Wiki › 연결 › 대기 인사이트.
 *
 * 모양새 탭 → 설정 › 지식 그래프를 거쳐 여기로 옮겨 왔다(원장 #506, 1.0 화면 정리).
 * 카드의 내용은 개인 지식창고 문서에서 오므로 "지식"을 다루는 화면이 제자리다.
 *
 * **"지금 한 장"이 이 화면의 핵심이다.** 대기 카드는 25초 이상 기다려야 뜨므로 설정을
 * 켜 놓고도 몇 주를 한 번도 못 볼 수 있다. 여기서 바로 한 장 보여 주면 "내 글이 이렇게
 * 나온다"를 확인하고 나간다.
 *
 * **폴더 범위**는 창고의 최상위 폴더 체크박스다. 창고 전체가 리마인드 감은 아니라서
 * (작업일지의 README 절이 카드로 떴다) 어느 폴더를 카드로 쓸지 여기서 고른다. 체크 상태는
 * 백엔드가 계산한 `included`를 그대로 쓴다 — 손으로 넣은 하위 경로 범위도 거짓 없이 보인다.
 * 다 켜면 `null`(전체)로 되돌려 새 폴더가 생겨도 저절로 든다.
 *
 * 덱 파일 폴더를 여는 버튼은 뺐다. 규격 MD를 Finder에서 손으로 쓰는 길은 아무도 걷지
 * 않았고(원장 #509), 설정에 남겨 두면 "이게 뭐지"만 남는다. 로더는 그대로라 파일이 있으면 센다.
 */
export function InsightSection() {
  const [on, setOn] = useState(true);
  const [stats, setStats] = useState<InsightAvailability | null>(null);
  const [preview, setPreview] = useState(false);
  const [savingScope, setSavingScope] = useState(false);

  const refresh = useCallback(() => insightAvailability().then(setStats).catch(() => {}), []);

  useEffect(() => {
    insightEnabledGet().then(setOn).catch(() => {});
    void refresh();
  }, [refresh]);

  const toggle = async () => {
    const next = !on;
    setOn(next);
    try {
      await insightEnabledSet(next);
    } catch {
      setOn(!next);
    }
  };

  const toggleFolder = async (folder: InsightWikiFolder) => {
    if (!stats) return;
    const all = stats.wiki_folders.map((f) => f.path);
    const current = stats.wiki_scope ?? all;
    const next = folder.included
      ? current.filter((p) => p !== folder.path && !p.startsWith(`${folder.path}/`))
      : [...current, folder.path];
    // 전부 골랐으면 "전체"로 되돌린다 — 목록을 그대로 저장하면 새 폴더가 조용히 빠진다.
    const scope = all.every((p) => next.includes(p)) ? null : next;
    setSavingScope(true);
    try {
      await insightWikiFoldersSet(scope);
      await refresh();
    } catch {
      // 저장이 실패하면 화면은 백엔드가 준 상태 그대로다 — 되돌릴 낙관 갱신이 없다.
    } finally {
      setSavingScope(false);
    }
  };

  const vaultConnected = !!stats?.wiki_root;
  const folders = stats?.wiki_folders ?? [];
  const scopeEmpty = vaultConnected && stats?.wiki_scope !== null && stats?.wiki_scope?.length === 0;

  return (
    <SettingSection
      id="insight"
      title="대기 인사이트"
      hint="에이전트 응답을 25초 넘게 기다릴 때 개인 지식창고의 글 한 조각을 카드로 띄웁니다. 문서의 절(##) 하나가 카드 하나이고, 출처는 문서 경로입니다. 카드로 쓸 폴더는 아래에서 고릅니다."
    >
      <SettingRow
        title="대기 중 카드 표시"
        hint={
          <>
            {stats && (
              <span className="text-text-secondary">
                {vaultConnected
                  ? `지식창고 문서 ${stats.wiki_notes}편 · 카드 ${stats.wiki_cards}장`
                  : "지식창고가 연결돼 있지 않습니다 — WIKI 화면에서 창고를 연결하면 그 문서가 카드가 됩니다."}
                {stats.decks > 0 && ` · 덱 파일 ${stats.decks}개 · 카드 ${stats.deck_cards}장`}
                {vaultConnected && stats.cards === 0 && !scopeEmpty && " — 절(##)이 있는 문서가 아직 없습니다."}
              </span>
            )}
            {/* 버린 카드를 조용히 삼키지 않는다 — 왜 안 뜨는지 알 수 있어야 한다. */}
            {stats?.warnings.length ? (
              <ul className="mt-1 text-[11px] text-text-muted">
                {stats.warnings.slice(0, 4).map((w) => (
                  <li key={w}>· {w}</li>
                ))}
              </ul>
            ) : null}
            <span className="mt-1.5 flex flex-wrap gap-1.5">
              <button
                className="text-xs text-text-secondary border border-border rounded-md px-2 py-1 hover:border-border-strong disabled:opacity-40"
                disabled={!stats || stats.cards === 0}
                aria-expanded={preview}
                onClick={() => setPreview((v) => !v)}
              >
                {preview ? "미리보기 닫기" : "지금 한 장"}
              </button>
            </span>
          </>
        }
      >
        <Switch on={on} onClick={() => void toggle()} label="대기 인사이트" />
      </SettingRow>
      {preview && (
        <div className="mt-3 max-w-md">
          <InsightCard onClose={() => setPreview(false)} />
        </div>
      )}
      {vaultConnected && folders.length > 0 && (
        <SettingRow
          title="카드로 쓸 폴더"
          hint={
            <>
              <span>
                {stats?.wiki_scope === null
                  ? "지식창고의 모든 폴더를 씁니다. 끄고 싶은 폴더의 체크를 빼세요."
                  : scopeEmpty
                    ? "고른 폴더가 없어 지식창고 카드가 뜨지 않습니다."
                    : "체크한 폴더의 문서만 카드가 됩니다."}
              </span>
              <ul className="mt-1.5 space-y-1" aria-label="카드로 쓸 폴더">
                {folders.map((folder) => (
                  <li key={folder.path}>
                    <label className="flex items-center gap-2 text-xs text-text-secondary">
                      <input
                        type="checkbox"
                        checked={folder.included}
                        disabled={savingScope}
                        onChange={() => void toggleFolder(folder)}
                      />
                      <span className="text-text">{folder.path === "." ? "루트 문서" : folder.path}</span>
                      <span className="text-text-muted">
                        · 문서 {folder.notes}편{folder.included && ` · 카드 ${folder.cards}장`}
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            </>
          }
        />
      )}
    </SettingSection>
  );
}
