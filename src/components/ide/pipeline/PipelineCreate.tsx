import { useEffect, useState } from "react";
import { gitBranches, type BranchList } from "../../../lib/ipc";
import {
  MIN_VENDORS,
  availableVendors,
  canStartPipeline,
  pipelineStart,
  pipelineVendors,
  vendorLabel,
  type RunRow,
  type VendorAvailability,
} from "../../../lib/pipeline";
import { BranchPicker } from "../BranchPicker";
import { BTN_PRIMARY, ErrorLine, INPUT, Section } from "./parts";

/** 실행 생성 — 저장소·base 브랜치·목표를 받고 가용 벤더를 보인다. 벤더 2개 미만이면 시작할 수 없다. */
export function PipelineCreate({ repo, onStarted }: { repo: string; onStarted: (run: RunRow) => void }) {
  const [vendors, setVendors] = useState<VendorAvailability[] | null>(null);
  const [branches, setBranches] = useState<BranchList>({ current: "", branches: [] });
  const [base, setBase] = useState("");
  const [goal, setGoal] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    pipelineVendors().then(setVendors, (e: unknown) => {
      setVendors([]);
      setError(String(e));
    });
  }, []);
  useEffect(() => {
    if (!repo) return;
    gitBranches(repo).then((b) => {
      setBranches(b);
      setBase("");
    }, () => undefined);
  }, [repo]);

  const enough = vendors != null && canStartPipeline(vendors);
  const baseBranch = base || branches.current;
  const ready = enough && !!repo && !!baseBranch && !!goal.trim() && !busy;

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      onStarted(await pipelineStart(repo, baseBranch, goal.trim()));
      setGoal("");
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Section title="새 실행">
      <div className="mb-2 flex flex-wrap items-center gap-2 text-sm">
        <span className="text-text-muted">{repo || "프로젝트를 먼저 선택하세요."}</span>
        {repo && (
          <BranchPicker value={base} current={branches.current} branches={branches.branches} onPick={setBase} />
        )}
      </div>
      <p className="mb-2 text-xs text-text-muted" aria-label="가용 벤더">
        가용 벤더:{" "}
        {vendors == null
          ? "확인 중…"
          : vendors.map((v) => `${vendorLabel(v.vendor)} ${v.available ? "사용 가능" : "사용 불가"}`).join(" · ")}
      </p>
      {vendors != null && !enough && (
        <p role="status" className="mb-2 text-sm text-status-awaiting">
          교차 리뷰에는 서로 다른 벤더가 {MIN_VENDORS}개 이상 필요합니다(현재 {availableVendors(vendors).length}개). 설정에서 CLI 로그인을 확인하세요.
        </p>
      )}
      <label className="flex flex-col gap-1 text-xs">
        목표
        <textarea className={INPUT} rows={4} value={goal} onChange={(e) => setGoal(e.target.value)} placeholder="만들고 싶은 기능이나 바꾸고 싶은 동작을 적으세요." />
      </label>
      <ErrorLine message={error} />
      <div className="mt-2">
        <button type="button" className={BTN_PRIMARY} disabled={!ready} onClick={() => void start()}>
          실행 시작
        </button>
      </div>
    </Section>
  );
}
