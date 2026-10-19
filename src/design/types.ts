/** Canvas screens receive sample data through props, without app IPC. */
export interface ScreenMeta<Props extends object = Record<string, unknown>> {
  title: string;
  viewport: { width: number; height: number };
  states: Record<string, Props>;
}
