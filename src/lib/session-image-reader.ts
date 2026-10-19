import { isAbsolutePath } from "./document-link";
import { readLocalFile } from "./ipc";
import { getTransport, type TaskRef } from "./transport";

/** Keep image reads on the session's host and its existing filesystem boundary. */
export function readSessionImage(ref: TaskRef, path: string) {
  const transport = getTransport(ref.host);
  return transport.kind === "local" && isAbsolutePath(path)
    ? readLocalFile(ref, path)
    : transport.fsRead(ref.id, path);
}
