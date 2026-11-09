import { useEffect, type ButtonHTMLAttributes, type ReactNode } from "react";
import { useTooltip } from "./Tooltip";

/**
 * 공용 버튼 (DESIGN.md `components.Button`).
 *
 * hover 배경을 `bg-text/[.08]`처럼 글자색의 반투명으로 칠하는 이유: 버튼이 놓이는 면이
 * `bg`·`surface`·`raised` 중 무엇이든 한 단계 들뜬 면으로 보인다. `hover:bg-raised`는
 * raised 면(메뉴·다이얼로그) 위에서 아무 변화도 보이지 않는다.
 */
export type ButtonVariant = "primary" | "secondary" | "ghost" | "danger";
export type ButtonSize = "sm" | "md";

const FOCUS =
  "focus-visible:outline focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-primary";

const VARIANT: Record<ButtonVariant, string> = {
  primary:
    "bg-primary text-bg hover:bg-primary-hover disabled:bg-border disabled:text-text-muted",
  secondary:
    "border border-border text-text hover:border-border-strong hover:bg-text/[.06] disabled:text-text-muted disabled:hover:bg-transparent",
  ghost: "text-text-secondary hover:bg-text/[.08] hover:text-text disabled:text-text-muted disabled:hover:bg-transparent",
  danger:
    "border border-dangerborder bg-dangerbg text-status-failed hover:border-status-failed disabled:opacity-50",
};

const SIZE: Record<ButtonSize, string> = {
  sm: "h-7 px-2.5 gap-1",
  md: "h-8 px-3 gap-1.5",
};

export function buttonClass(variant: ButtonVariant = "secondary", size: ButtonSize = "md"): string {
  return `inline-flex shrink-0 items-center justify-center rounded-md text-sm font-medium transition-colors disabled:cursor-default ${FOCUS} ${SIZE[size]} ${VARIANT[variant]}`;
}

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: ButtonVariant;
  size?: ButtonSize;
}

export function Button({ variant = "secondary", size = "md", className, type = "button", ...rest }: ButtonProps) {
  return <button type={type} className={className ? `${buttonClass(variant, size)} ${className}` : buttonClass(variant, size)} {...rest} />;
}

interface IconButtonProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "aria-label" | "children"> {
  /** 보이는 글자가 없으므로 이름이 필수다. aria-label과 툴팁에 같이 쓴다. `title`을 주면 툴팁만 그것으로 바뀐다. */
  label: string;
  /** 토글이면 넘긴다 — aria-pressed와 눌린 배경면이 함께 붙는다. */
  pressed?: boolean;
  /** `accent`는 눌림이 아닌 "주목할 상태"(저장 안 된 변경 등)를 글자색으로만 알린다. `danger`는 삭제·중단 — hover에서만 빨강이다. */
  tone?: "default" | "accent" | "danger";
  /**
   * 메뉴·팝오버를 여는 버튼이면 넘긴다 — 열린 동안 눌린 것과 같은 배경면을 쓰되 aria-pressed가 아니라
   * aria-expanded를 단다. 메뉴 버튼은 토글이 아니다.
   */
  expanded?: boolean;
  size?: ButtonSize;
  children: ReactNode;
}

/** 아이콘 단독 버튼 (DESIGN.md `components.IconButton`). 기본 글자색은 secondary — muted는 disabled 전용이다. */
export function IconButton({
  label,
  pressed,
  expanded,
  tone = "default",
  size = "md",
  className,
  type = "button",
  title,
  disabled,
  onMouseEnter,
  onMouseLeave,
  onMouseDown,
  onFocus,
  onBlur,
  ...rest
}: IconButtonProps) {
  const box = size === "sm" ? "h-6 w-6" : "h-7 w-7";
  const on = pressed || expanded;
  const ink =
    tone === "accent"
      ? "text-primary-bright"
      : tone === "danger"
        ? "text-text-secondary hover:text-status-failed"
        : on
          ? "text-text"
          : "text-text-secondary hover:text-text";
  const state = on ? `bg-text/[.10] ${ink}` : `${ink} hover:bg-text/[.08]`;
  const tip = title ?? label;
  const { ref, triggerProps: t, node, hide } = useTooltip<HTMLButtonElement>(tip, { describe: tip !== label });
  // 떠 있는 동안 비활성으로 바뀌면 mouseleave가 오지 않을 수 있다 — 여기서 닫는다.
  useEffect(() => {
    if (disabled) hide();
  }, [disabled, hide]);
  return (
    <>
      <button
        ref={ref}
        type={type}
        aria-label={label}
        aria-pressed={pressed}
        aria-expanded={expanded}
        aria-describedby={t["aria-describedby"]}
        // 비활성 버튼은 브라우저가 마우스 이벤트를 막는 경우가 있어 네이티브 title로 남긴다.
        title={disabled ? tip : undefined}
        disabled={disabled}
        className={`inline-flex shrink-0 items-center justify-center rounded-md transition-colors disabled:text-text-muted disabled:hover:bg-transparent ${FOCUS} ${box} ${state}${className ? ` ${className}` : ""}`}
        onMouseEnter={(e) => {
          t.onMouseEnter();
          onMouseEnter?.(e);
        }}
        onMouseLeave={(e) => {
          t.onMouseLeave();
          onMouseLeave?.(e);
        }}
        onMouseDown={(e) => {
          t.onMouseDown();
          onMouseDown?.(e);
        }}
        onFocus={(e) => {
          t.onFocus();
          onFocus?.(e);
        }}
        onBlur={(e) => {
          t.onBlur();
          onBlur?.(e);
        }}
        {...rest}
      />
      {node}
    </>
  );
}

/** 단축키 표기. 버튼 옆·메뉴 우측·툴팁 안에서 같은 모양으로 쓴다. */
export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded border border-border px-1 font-code text-[11px] leading-none text-text-muted">
      {children}
    </kbd>
  );
}
