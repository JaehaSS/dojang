import {
  getTransport,
  hasHost,
  listHosts,
  LOCAL_HOST,
  registerTransport,
  unregisterTransport,
  type HostId,
} from "../transport";
import { RunnerTransport, type RunnerHealth } from "./runner";

const STORAGE_KEY = "praxis.remoteRunner.v1";

export interface RemoteRunnerProfile {
  hostId: HostId;
  endpoint: string;
  pairingToken: string;
  enabled: boolean;
}

const DEFAULT_PROFILE: RemoteRunnerProfile = {
  hostId: "mini",
  endpoint: "",
  pairingToken: "",
  enabled: false,
};

let memoryProfile: RemoteRunnerProfile | null = null;

function getStorage(): Storage | null {
  try {
    return typeof globalThis !== "undefined" && "localStorage" in globalThis
      ? globalThis.localStorage
      : null;
  } catch {
    return null;
  }
}

export function loadRemoteRunnerProfile(): RemoteRunnerProfile {
  const storage = getStorage();
  if (!storage) {
    return memoryProfile ? { ...memoryProfile } : { ...DEFAULT_PROFILE };
  }
  try {
    const raw = storage.getItem(STORAGE_KEY);
    if (!raw) return memoryProfile ? { ...memoryProfile } : { ...DEFAULT_PROFILE };
    const parsed = JSON.parse(raw) as Partial<RemoteRunnerProfile>;
    return {
      hostId: (parsed.hostId ?? "mini").trim() || "mini",
      endpoint: (parsed.endpoint ?? "").trim(),
      pairingToken: (parsed.pairingToken ?? "").trim(),
      enabled: Boolean(parsed.enabled),
    };
  } catch {
    return memoryProfile ? { ...memoryProfile } : { ...DEFAULT_PROFILE };
  }
}

export function saveRemoteRunnerProfile(profile: RemoteRunnerProfile): RemoteRunnerProfile {
  const normalized: RemoteRunnerProfile = {
    hostId: profile.hostId.trim() || "mini",
    endpoint: profile.endpoint.trim().replace(/\/$/, ""),
    pairingToken: profile.pairingToken.trim(),
    enabled: Boolean(profile.enabled && profile.endpoint.trim() && profile.pairingToken.trim()),
  };
  memoryProfile = { ...normalized };
  const storage = getStorage();
  if (storage) {
    try {
      storage.setItem(STORAGE_KEY, JSON.stringify(normalized));
    } catch {
      // localStorage 쓰기 실패 시에도 메모리 등록은 수행한다.
    }
  }
  syncRemoteRunnerTransport(normalized);
  return normalized;
}

export function syncRemoteRunnerTransport(
  profile: RemoteRunnerProfile = loadRemoteRunnerProfile(),
): RunnerTransport | null {
  if (!profile.enabled || !profile.endpoint || !profile.pairingToken) {
    if (profile.hostId && profile.hostId !== LOCAL_HOST && hasHost(profile.hostId)) {
      const current = getTransport(profile.hostId);
      if (current instanceof RunnerTransport) {
        try {
          unregisterTransport(profile.hostId);
        } catch {
          // ignore
        }
      }
    }
    return null;
  }
  if (profile.hostId === LOCAL_HOST) {
    return null;
  }
  const transport = new RunnerTransport({
    profileName: profile.hostId,
    endpoint: profile.endpoint,
    pairingToken: profile.pairingToken,
  });
  registerTransport(transport);
  return transport;
}

export async function probeRemoteRunner(
  endpoint: string,
  pairingToken: string,
): Promise<RunnerHealth> {
  const transport = new RunnerTransport({
    profileName: "probe",
    endpoint: endpoint.trim().replace(/\/$/, ""),
    pairingToken: pairingToken.trim(),
  });
  return transport.health();
}

export function getRemoteRunnerTransport(): RunnerTransport | null {
  const profile = loadRemoteRunnerProfile();
  if (profile.enabled && hasHost(profile.hostId)) {
    const candidate = getTransport(profile.hostId);
    if (candidate instanceof RunnerTransport) {
      return candidate;
    }
  }
  for (const host of listHosts()) {
    if (host !== LOCAL_HOST) {
      const candidate = getTransport(host);
      if (candidate instanceof RunnerTransport) {
        return candidate;
      }
    }
  }
  return null;
}
