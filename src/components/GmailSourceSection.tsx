import { useCallback, useEffect, useState } from "react";
import {
  knowledgeGmailConfigSet,
  knowledgeGmailConnect,
  knowledgeGmailDisconnect,
  knowledgeGmailEstimate,
  knowledgeGmailStatus,
  knowledgeGmailSync,
  type GmailStatus,
} from "../lib/ipc";
import { inputCls } from "./ide/formStyles";

/**
 * Wiki › 연결 › Gmail (설계 0020 Phase 4).
 *
 * 클라이언트 출처는 둘로 갈린다 (ADR 0147). 빌드에 client가 박혀 있으면 사용자는
 * 아무것도 입력하지 않고 `연결`만 누르면 되고, 없으면 예전처럼 자기 것을 발급해
 * 넣는다(DR-10). 발급 절차를 모르면 후자는 연결 자체가 불가능한데 문서에만 두면
 * 아무도 읽지 않으므로, 화면 안에 접어서 둔다.
 *
 * **번들이 BYO를 밀어내지 않는다.** 번들된 client가 받아주지 않는 계정이 있기 때문이다 —
 * 그때 기댈 곳은 직접 넣는 값뿐이다.
 */
export function GmailSourceSection() {
  const [status, setStatus] = useState<GmailStatus | null>(null);
  const [clientId, setClientId] = useState("");
  const [clientSecret, setClientSecret] = useState("");
  const [query, setQuery] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [note, setNote] = useState<string | null>(null);
  const [estimate, setEstimate] = useState<number | null>(null);
  const [guideOpen, setGuideOpen] = useState(false);
  const [byoOpen, setByoOpen] = useState(false);

  const refresh = useCallback(async () => {
    try {
      const next = await knowledgeGmailStatus();
      setStatus(next);
      setClientId(next.client_id);
      setQuery(next.query);
    } catch (e) {
      setError(String(e));
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const run = async (label: string, task: () => Promise<void>) => {
    setBusy(label);
    setError(null);
    try {
      await task();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(null);
      void refresh();
    }
  };

  const save = () =>
    run("저장", async () => {
      // secret은 비워 두면 기존 값을 유지한다 — 필터만 고칠 때 연결이 끊기지 않도록.
      await knowledgeGmailConfigSet(clientId, query, clientSecret || undefined);
      setClientSecret("");
      setNote("저장했습니다.");
    });

  // client_id를 비우면 백엔드 해석이 번들로 되돌아간다. 옛 refresh token은 다른
  // client로 발급된 것이라 그대로 두면 `invalid_grant`가 되므로 백엔드가 폐기한다.
  const useBundled = () =>
    run("되돌리기", async () => {
      await knowledgeGmailConfigSet("", query);
      setClientSecret("");
      setByoOpen(false);
      setNote("기본 클라이언트로 되돌렸습니다. 다시 연결하세요.");
    });

  const connect = () =>
    run("연결", async () => {
      const address = await knowledgeGmailConnect();
      setNote(`${address} 계정을 연결했습니다.`);
    });

  const checkEstimate = () =>
    run("확인", async () => {
      setEstimate(await knowledgeGmailEstimate());
    });

  const sync = () =>
    run("동기화", async () => {
      const result = await knowledgeGmailSync(5);
      setNote(
        `색인 ${result.indexed} · 변경 없음 ${result.skipped} · 삭제 ${result.deleted} · 임베딩 ${result.embedded}` +
          (result.has_more ? " — 아직 남았습니다. 다시 눌러 이어가세요." : " — 최신입니다."),
      );
    });

  const disconnect = () =>
    run("해제", async () => {
      const removed = await knowledgeGmailDisconnect();
      setNote(`연결을 해제하고 메일 ${removed}건을 지웠습니다.`);
      setEstimate(null);
    });

  const connected = status?.connected ?? false;
  const bundledAvailable = status?.bundled_available ?? false;
  const usingBundled = status?.using_bundled ?? false;
  // 번들이 없으면 BYO 말고는 길이 없으므로 접지 않는다. 사용자가 이미 자기 값을
  // 넣어 둔 경우(=번들을 안 쓰는 중)도 펼친 채로 둔다 — 접으면 자기 설정이 사라진
  // 것처럼 보인다.
  const byoVisible = !bundledAvailable || !usingBundled || byoOpen;

  return (
    <section className="border border-border rounded-md p-3 mt-4">
      <div className="flex items-center justify-between gap-2 mb-1">
        <h3 className="text-sm font-semibold">Gmail</h3>
        <span className="text-xs text-text-muted">
          {status ? `${status.stage} · ${status.indexed}건` : "…"}
        </span>
      </div>
      <p className="text-xs text-text-muted mb-3">
        읽기 전용으로 흡수합니다. 메일을 수정하거나 보내지 않습니다.
      </p>

      {byoVisible ? (
        <>
          <button
            className="text-xs text-text-secondary hover:text-text underline mb-2"
            onClick={() => setGuideOpen((v) => !v)}
          >
            {guideOpen ? "설정 절차 접기" : "Google Cloud 설정 절차 보기"}
          </button>
          {guideOpen && (
            <ol className="text-xs text-text-secondary list-decimal ml-4 space-y-1 mb-3">
              <li>Google Cloud에서 프로젝트를 만들고 Gmail API를 사용 설정합니다.</li>
              <li>
                OAuth consent screen을 <b>External</b>로 만들고 범위에{" "}
                <span className="font-code">gmail.readonly</span>를 추가합니다.
              </li>
              <li>
                <b>Publish app</b>을 눌러 <span className="font-code">In production</span>으로
                바꿉니다. <b>Testing으로 두면 refresh token이 7일마다 만료</b>돼 매주 다시
                연결해야 합니다. 검증 제출은 하지 않아도 됩니다.
              </li>
              <li>
                사용자 인증 정보에서 <b>Desktop app</b> 클라이언트를 만들고 아래에 붙여넣습니다.
              </li>
            </ol>
          )}

          <label className="flex items-center gap-2 py-1">
            <span className="w-28 shrink-0 text-xs text-text-secondary">client_id</span>
            <input
              className={inputCls}
              value={clientId}
              placeholder="…apps.googleusercontent.com"
              onChange={(e) => setClientId(e.target.value)}
            />
          </label>
          <label className="flex items-center gap-2 py-1">
            <span className="w-28 shrink-0 text-xs text-text-secondary">client_secret</span>
            <input
              className={inputCls}
              type="password"
              value={clientSecret}
              placeholder={status?.client_secret_set ? "저장됨 — 바꿀 때만 입력" : "GOCSPX-…"}
              onChange={(e) => setClientSecret(e.target.value)}
            />
          </label>
          {bundledAvailable && (
            <button
              className="text-xs text-text-muted hover:text-text underline mt-1"
              disabled={busy !== null}
              onClick={() => void useBundled()}
            >
              기본 클라이언트로 되돌리기
            </button>
          )}
        </>
      ) : (
        <div className="text-xs text-text-secondary border border-border rounded-md px-3 py-2 mb-3">
          <b>따로 설정할 것이 없습니다.</b> 바로 연결하세요.
          <button
            className="block mt-1 text-text-muted hover:text-text underline"
            onClick={() => setByoOpen(true)}
          >
            다른 클라이언트 직접 입력하기
          </button>
        </div>
      )}
      <label className="flex items-center gap-2 py-1">
        <span className="w-28 shrink-0 text-xs text-text-secondary">흡수 범위</span>
        <input
          className={inputCls}
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />
      </label>
      <p className="text-xs text-text-muted mt-1 mb-3">
        프로모션·소셜은 지식 가치가 낮은데 볼륨이 흔히 절반을 넘습니다. 좁게 시작해 넓히면
        부족분만 채워지지만, 넓게 시작해 좁히면 이미 쓴 시간은 돌아오지 않습니다.
      </p>

      <div className="flex flex-wrap items-center gap-2">
        <button
          className="h-8 px-3 rounded-md text-sm border border-border text-text-secondary hover:text-text disabled:opacity-40"
          disabled={busy !== null}
          onClick={() => void save()}
        >
          저장
        </button>
        <button
          className="h-8 px-3 rounded-md text-sm bg-primary text-white disabled:opacity-40"
          disabled={busy !== null || (!clientId && !bundledAvailable)}
          onClick={() => void connect()}
        >
          {connected ? "다시 연결" : "연결"}
        </button>
        {connected && (
          <>
            <button
              className="h-8 px-3 rounded-md text-sm border border-border text-text-secondary hover:text-text disabled:opacity-40"
              disabled={busy !== null}
              onClick={() => void checkEstimate()}
            >
              규모 확인
            </button>
            <button
              className="h-8 px-3 rounded-md text-sm border border-border text-text-secondary hover:text-text disabled:opacity-40"
              disabled={busy !== null}
              onClick={() => void sync()}
            >
              {busy === "동기화" ? "동기화 중…" : "동기화"}
            </button>
            <button
              className="h-8 px-3 rounded-md text-sm text-text-muted hover:text-danger disabled:opacity-40"
              disabled={busy !== null}
              onClick={() => {
                // 파괴적이다 — 지워질 개수를 먼저 보여주고 확인을 받는다.
                const count = status?.indexed ?? 0;
                if (window.confirm(`메일 ${count}건을 지우고 연결을 해제합니다. 계속할까요?`)) {
                  void disconnect();
                }
              }}
            >
              연결 해제
            </button>
          </>
        )}
      </div>

      {!clientId && !bundledAvailable && (
        <div className="text-xs text-text-muted mt-2">
          연결하려면 위에 <span className="font-code">client_id</span>와{" "}
          <span className="font-code">client_secret</span>을 입력하고 저장하세요.
        </div>
      )}
      {estimate !== null && (
        <div className="text-sm text-text-secondary mt-3">
          현재 범위에 약 <b>{estimate.toLocaleString()}</b>건이 걸립니다 (Gmail이 주는
          어림수). 동기화는 나눠서 진행되며 중단해도 이어집니다.
        </div>
      )}
      {note && <div className="text-sm text-text-secondary mt-3">{note}</div>}
      {(error ?? status?.last_error) && (
        <div className="text-sm text-danger mt-3 whitespace-pre-wrap">
          {error ?? status?.last_error}
        </div>
      )}
    </section>
  );
}
