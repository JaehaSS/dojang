import type { ToolOutputContent } from "./ipc";

/** Tool output is persisted in the transcript, so only self-contained media may be embedded.
 * Remote media would turn opening an old conversation into an implicit network request. */
export function normalizeToolOutputContents(value: unknown): ToolOutputContent[] {
  if (!Array.isArray(value)) return [];
  return value.flatMap((content): ToolOutputContent[] => {
    if (typeof content !== "object" || content == null) return [];
    const item = content as Record<string, unknown>;
    if (item.type === "text" && typeof item.text === "string") return [{ type: "text", text: item.text }];
    if (item.type === "image" && typeof item.url === "string") return [{ type: "image", url: item.url }];
    if (item.type === "audio" && typeof item.url === "string") return [{ type: "audio", url: item.url }];
    if (item.type === "resource" && typeof item.uri === "string" && typeof item.title === "string") return [{ type: "resource", uri: item.uri, title: item.title }];
    return [];
  });
}

const DATA_URL = /^data:(image\/(?:png|jpeg|webp|gif)|audio\/(?:wav|mpeg|ogg|webm));base64,([A-Za-z0-9+/]+={0,2})$/;

/** Returns the advertised MIME only for a syntactically complete, allowlisted base64 data URL. */
export function safeToolMediaUrl(value: string, expected: "image" | "audio"): string | null {
  const match = DATA_URL.exec(value);
  if (!match || !match[1].startsWith(`${expected}/`)) return null;
  const payload = match[2];
  if (payload.length % 4 !== 0 || (payload.includes("=") && !/={1,2}$/.test(payload))) return null;
  return value;
}

/** Resource activation stays in the established app link handler; reject executable web schemes first. */
export function safeToolResourceUri(value: string): string | null {
  const uri = value.trim();
  if (!uri || /[\u0000-\u001f\u007f]/.test(uri)) return null;
  if (/^[A-Za-z][A-Za-z0-9+.-]*:/.test(uri)) {
    try {
      const protocol = new URL(uri).protocol;
      return protocol === "https:" || protocol === "http:" || protocol === "file:" ? uri : null;
    } catch {
      return null;
    }
  }
  return uri;
}

/** Native URL requests can open only an explicit web URL, and only after the user presses a button. */
export function safeExternalHttpUrl(value: string | null | undefined): string | null {
  if (!value) return null;
  try {
    const parsed = new URL(value);
    return parsed.protocol === "https:" || parsed.protocol === "http:" ? parsed.href : null;
  } catch {
    return null;
  }
}
