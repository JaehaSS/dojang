import { useMemo } from "react";
import type { TablePreview } from "../../lib/ipc";
import { Icon } from "./icons";

/** 백엔드 JSON을 풀어 준다. 형태가 어긋나면(옛 백엔드·손상) 오류만 담은 빈 표로 돌려
 *  탭 자체는 열리게 한다 — 파싱 실패로 에디터 전체가 죽을 이유는 없다. */
export function parseTablePreview(content: string): TablePreview {
  try {
    const parsed = JSON.parse(content) as Partial<TablePreview>;
    return {
      format: typeof parsed.format === "string" ? parsed.format : "table",
      columns: Array.isArray(parsed.columns) ? parsed.columns : [],
      rows: Array.isArray(parsed.rows) ? parsed.rows : [],
      total_rows: typeof parsed.total_rows === "number" ? parsed.total_rows : -1,
      total_columns: typeof parsed.total_columns === "number" ? parsed.total_columns : 0,
      row_groups: typeof parsed.row_groups === "number" ? parsed.row_groups : 0,
      shown_rows: typeof parsed.shown_rows === "number" ? parsed.shown_rows : 0,
      shown_columns: typeof parsed.shown_columns === "number" ? parsed.shown_columns : 0,
      error: typeof parsed.error === "string" ? parsed.error : null,
    };
  } catch (e) {
    return {
      format: "table",
      columns: [],
      rows: [],
      total_rows: -1,
      total_columns: 0,
      row_groups: 0,
      shown_rows: 0,
      shown_columns: 0,
      error: `미리보기를 읽지 못했습니다: ${String(e)}`,
    };
  }
}

/** "총 N행 중 처음 K행 · 컬럼 M개 중 J개" — 잘린 것이 없으면 잘렸다는 말을 하지 않는다. */
export function tableSummary(t: TablePreview): string {
  const rows =
    t.total_rows < 0
      ? `${t.shown_rows.toLocaleString()}행`
      : t.shown_rows < t.total_rows
        ? `총 ${t.total_rows.toLocaleString()}행 중 처음 ${t.shown_rows.toLocaleString()}행`
        : `${t.total_rows.toLocaleString()}행`;
  const cols =
    t.shown_columns < t.total_columns
      ? `컬럼 ${t.total_columns.toLocaleString()}개 중 ${t.shown_columns.toLocaleString()}개`
      : `컬럼 ${t.total_columns.toLocaleString()}개`;
  const groups = t.row_groups > 0 ? ` · row group ${t.row_groups}` : "";
  return `${rows} · ${cols}${groups}`;
}

/** 셀 하나가 차지할 수 있는 최대 길이 — 긴 JSON·문자열 컬럼 하나가 표 전체를 늘어뜨리지 않게. */
const CELL_MAX = 200;

const cellText = (v: string | null): string =>
  v == null ? "" : v.length > CELL_MAX ? `${v.slice(0, CELL_MAX)}…` : v;

/** 표 파일(parquet) 미리보기 — 처음 N행·M열을 읽기 전용 그리드로 보여 준다.
 *  편집은 없다. 더 보려면 Python 콘솔에서 `pd.read_parquet`으로 여는 버튼을 둔다(로컬 전용). */
export function TableView({
  content,
  name,
  onOpenInRepl,
}: {
  /** `FileContent.content` — TablePreview JSON 문자열. */
  content: string;
  /** 파일 이름 — 헤더 표기. */
  name: string;
  /** Python 콘솔로 열기. 없으면(원격·콘솔 미지원) 버튼을 그리지 않는다. */
  onOpenInRepl?: () => void;
}) {
  const table = useMemo(() => parseTablePreview(content), [content]);
  const summary = tableSummary(table);

  return (
    <div className="flex-1 min-h-0 flex flex-col" data-testid="table-view">
      <div className="h-8 shrink-0 flex items-center gap-2 px-3 border-b border-border text-xs text-text-muted">
        <Icon name="chart" size={14} />
        <span className="truncate text-text-secondary">{name}</span>
        <span className="uppercase tracking-wide opacity-70">{table.format}</span>
        <span className="truncate">· {summary}</span>
        {onOpenInRepl && (
          <button
            className="ml-auto h-6 px-2 rounded border border-border text-text-secondary hover:text-text hover:border-border-strong flex items-center gap-1"
            onClick={onOpenInRepl}
            title="Python 콘솔에서 pandas로 연다 (⌃`)"
          >
            <Icon name="terminal" size={12} />
            IPython으로 열기
          </button>
        )}
      </div>
      {table.error && (
        <div className="shrink-0 px-3 py-2 text-xs text-danger border-b border-border" role="alert">
          {table.error}
        </div>
      )}
      <div className="flex-1 min-h-0 overflow-auto">
        <table className="text-xs font-mono border-collapse min-w-full">
          <thead className="sticky top-0 bg-surface z-10">
            <tr>
              <th className="px-2 py-1 text-right text-text-muted border-b border-r border-border select-none">
                #
              </th>
              {table.columns.map((c, i) => (
                <th
                  key={`${c.name}-${i}`}
                  className="px-2 py-1 text-left border-b border-r border-border whitespace-nowrap align-top"
                  title={c.type}
                >
                  <div className="text-text">{c.name}</div>
                  <div className="text-[11px] font-normal text-text-muted">{c.type}</div>
                </th>
              ))}
            </tr>
          </thead>
          <tbody>
            {table.rows.map((row, r) => (
              <tr key={r} className="hover:bg-raised">
                <td className="px-2 py-0.5 text-right text-text-muted border-r border-b border-border select-none">
                  {r}
                </td>
                {table.columns.map((_, c) => {
                  const v = row[c] ?? null;
                  return (
                    <td
                      key={c}
                      className={`px-2 py-0.5 border-r border-b border-border whitespace-pre max-w-[32rem] truncate ${
                        v == null ? "italic text-text-muted" : ""
                      }`}
                      title={v ?? "null"}
                    >
                      {v == null ? "null" : cellText(v)}
                    </td>
                  );
                })}
              </tr>
            ))}
          </tbody>
        </table>
        {table.rows.length === 0 && !table.error && (
          <div className="px-3 py-4 text-xs text-text-muted">행이 없습니다.</div>
        )}
      </div>
    </div>
  );
}
