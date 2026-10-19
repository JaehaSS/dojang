import type { ReactElement } from "react";

import type { AppActivity } from "../../../lib/ipc";
import { Metric } from "./parts";
import { deltaPct, fmtDuration } from "./format";

interface Props {
  /** 상위가 로드한 집계. 아직 없으면 null. */
  data: AppActivity | null;
}

/**
 * 앱 사용 시간 — 실제로 조작한 시간을 앞세우고, 켜져 있던 시간과 창 포커스 시간을 함께 둔다.
 *
 * 조작 시간은 창 포커스 중 키보드·마우스 입력 사이가 60초 이하로 이어진 구간이다. 창을 띄워
 * 두기만 한 시간은 포커스 시간에는 들어가도 여기에는 들어가지 않는다.
 *
 * 기록은 이 기능이 들어온 뒤부터 쌓인다. 그 전 기간을 0시간으로 보여주면 "안 썼다"로 읽히므로
 * 기록이 하나도 없을 때는 빈 상태 문장으로 대신한다.
 */
export function AppActivityPanel({ data }: Props): ReactElement {
  if (data == null) {
    return <div className="text-text-muted text-sm">집계 중…</div>;
  }
  if (data.since == null) {
    return (
      <div className="text-text-muted text-sm border border-border rounded-lg p-8 text-center">
        아직 기록이 없습니다. 앱이 켜져 있는 동안 사용 시간이 쌓입니다.
      </div>
    );
  }
  const dailyInput = data.active_days > 0 ? Math.round(data.input_secs / data.active_days) : null;
  return (
    <div className="grid grid-cols-2 sm:grid-cols-4 gap-2.5">
      <Metric
        label="조작 시간"
        value={fmtDuration(data.input_secs)}
        delta={deltaPct(data.input_secs, data.prev?.input_secs)}
        accent
      />
      <Metric
        label="켜져 있던 시간"
        value={fmtDuration(data.open_secs)}
        delta={deltaPct(data.open_secs, data.prev?.open_secs)}
      />
      <Metric
        label="창 포커스 시간"
        value={fmtDuration(data.focus_secs)}
        delta={deltaPct(data.focus_secs, data.prev?.focus_secs)}
      />
      <Metric label="켜진 날 평균 조작" value={fmtDuration(dailyInput)} />
    </div>
  );
}
