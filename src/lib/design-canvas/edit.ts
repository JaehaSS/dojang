export interface DesignEditRequest {
  requestId: string;
  operation: "inspect" | "apply";
  source: string;
  instanceSource?: string | null;
  scope: "instance" | "component";
  property?: string;
  value?: string;
  expectedHash?: string;
  componentConfirmed?: boolean;
}

export interface DesignEditInfo {
  requestId: string;
  ok: true;
  file: string;
  line: number;
  column: number;
  hash: string;
  editable: boolean;
  reason?: string;
  shared: boolean;
  source: string;
  selectionSource: string;
  className: string;
  kind: "static" | "dynamic" | "none";
}
export interface DesignEditFailure { requestId: string; ok: false; reason: string; message: string }
export type DesignEditResult = DesignEditInfo | DesignEditFailure;
export interface DesignEditEvent { taskId: number; surface: string; request: DesignEditRequest }

export function isDesignEditRequest(value: unknown): value is DesignEditRequest {
  if (!value || typeof value !== "object") return false;
  const request = value as Record<string, unknown>;
  return typeof request.requestId === "string" && request.requestId.length > 0 && request.requestId.length <= 128
    && (request.operation === "inspect" || request.operation === "apply")
    && typeof request.source === "string" && request.source.length <= 1024
    && (request.instanceSource == null || typeof request.instanceSource === "string" && request.instanceSource.length <= 1024)
    && (request.scope === "instance" || request.scope === "component")
    && (request.componentConfirmed === undefined || typeof request.componentConfirmed === "boolean")
    && (request.operation !== "apply" || typeof request.expectedHash === "string" && /^[a-f0-9]{64}$/.test(request.expectedHash)
      && typeof request.property === "string" && request.property.length <= 64 && typeof request.value === "string" && request.value.length <= 256);
}

export function isDesignEditResult(value: unknown): value is DesignEditResult {
  if (!value || typeof value !== "object") return false;
  const result = value as Record<string, unknown>;
  if (typeof result.requestId !== "string") return false;
  if (result.ok === false) return typeof result.reason === "string" && typeof result.message === "string";
  return result.ok === true && typeof result.file === "string" && typeof result.source === "string"
    && (result.reason === undefined || typeof result.reason === "string")
    && typeof result.selectionSource === "string" && typeof result.hash === "string" && /^[a-f0-9]{64}$/.test(result.hash)
    && typeof result.editable === "boolean" && typeof result.shared === "boolean" && typeof result.className === "string"
    && ["static", "dynamic", "none"].includes(result.kind as string)
    && Number.isInteger(result.line) && Number.isInteger(result.column);
}
