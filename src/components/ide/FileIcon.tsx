import { FILE_GLYPHS, type FileGlyphName } from "./file-glyphs";
import { Icon, type IconName } from "./icons";

/**
 * 파일 트리의 파일 종류 아이콘 — UI 아이콘(`Icon`)과 **다른 register**다.
 *
 * `Icon`은 `currentColor`를 상속하는 스트로크 글리프이고, 이쪽은 **자기 색을 가진 이미지**다.
 * 한 컴포넌트에 섞지 않는 이유가 그것이다: 렌더 모델이 다르고(stroke vs fill), 색 정책도
 * 다르다(테마 상속 vs 고정 브랜드색).
 *
 * `DESIGN.md`의 근거는 Don't #5의 기존 예외다 — **"에이전트 브랜드색(Claude/Cursor 로고)은
 * 아이콘 이미지 내부에 한정"**. 언어 로고도 같은 범주이고, 액센트 teal이나 상태색으로
 * 새는 것이 아니라 아이콘 안에 갇혀 있다.
 *
 * 아는 글리프가 없으면 **기존 단색 아이콘 + 틴트로 후퇴한다.** 형태가 주 채널이라는 계약은
 * 그대로다 — 색을 못 알아봐도 실루엣으로 종류가 읽힌다.
 */
export function FileIcon({
  glyph,
  fallback,
  fallbackTint,
  size = 18,
}: {
  glyph: FileGlyphName | null;
  fallback: IconName;
  /** 후퇴 경로의 틴트 클래스 — 글리프가 있으면 쓰지 않는다(아이콘이 색을 갖는다). */
  fallbackTint?: string;
  /** 컬러 글리프의 크기. 트리 행은 18px(DESIGN.md 파일 타입 아이콘, 대비 실측 근거) — UI 아이콘 단계와 별개다. */
  size?: number;
}) {
  if (!glyph) {
    return (
      // 선 아이콘은 크기 단계(IconSize)를 지키고, 칸은 컬러 글리프와 같은 크기로 잡아 행 정렬을 맞춘다.
      // 컬러 글리프는 viewBox 안에 여백이 있어 16px 선 아이콘과 눈으로 보이는 크기가 비슷하다.
      <span className={`inline-flex shrink-0 items-center justify-center ${fallbackTint ?? ""}`} style={{ width: size, height: size }}>
        <Icon name={fallback} size={size >= 16 ? 16 : 14} />
      </span>
    );
  }
  const g = FILE_GLYPHS[glyph];
  return (
    <svg width={size} height={size} viewBox={g.viewBox} aria-hidden="true">
      {g.paths.map((p, i) => (
        <path key={i} d={p.d} fill={p.fill ?? "currentColor"} />
      ))}
    </svg>
  );
}
