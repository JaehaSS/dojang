import type { ScreenMeta } from "../types";

interface Props { hours: string; tasks: number; reviews: number; error?: string }

export const meta: ScreenMeta<Props> = {
  title: "주간 통계 카드",
  viewport: { width: 960, height: 640 },
  states: {
    정상: { hours: "12h 30m", tasks: 41, reviews: 9 },
    "빈 상태": { hours: "0h", tasks: 0, reviews: 0 },
    오류: { hours: "", tasks: 0, reviews: 0, error: "통계를 불러오지 못했습니다." },
  },
};

export default function WeeklySummary({ hours, tasks, reviews, error }: Props) {
  return <section className="min-h-screen bg-bg p-6 font-ui text-text">
    <h1 className="mb-4 text-xl font-semibold">이번 주 작업</h1>
    {error ? <p role="alert" className="rounded border border-dangerborder bg-dangerbg p-4 text-status-failed">{error}</p>
      : tasks === 0 ? <p className="rounded border border-border p-4 text-text-secondary">아직 기록된 작업이 없습니다.</p>
        : <div className="grid grid-cols-1 gap-3 sm:grid-cols-3">{[["사용 시간", hours], ["완료한 작업", tasks], ["리뷰", reviews]].map(([label, value]) =>
          <div key={label} className="rounded-md border border-border bg-surface p-4"><p className="text-sm text-text-secondary">{label}</p><p className="mt-2 text-2xl font-semibold">{value}</p></div>,
        )}</div>}
  </section>;
}
