// Where replays come from: the ones shipped with the page, and, when there
// is a server, the matches it has recorded.

import type { ReplayChoice } from "../ui/store";

interface Bundled {
  name: string;
  file: string;
}

interface Recorded {
  name: string;
  recorded_at: number;
}

async function json<T>(url: string): Promise<T[]> {
  try {
    const response = await fetch(url);
    return response.ok ? ((await response.json()) as T[]) : [];
  } catch {
    return [];
  }
}

export async function listReplays(withServer: boolean): Promise<ReplayChoice[]> {
  const bundled = await json<Bundled>("./replays/index.json");
  const recorded = withServer ? await json<Recorded>("./api/replays") : [];
  return [
    ...recorded
      .sort((a, b) => b.recorded_at - a.recorded_at)
      .slice(0, 20)
      .map((entry) => ({
        name: `Played here, ${new Date(entry.recorded_at * 1000).toLocaleString()}`,
        url: `./api/replays/${entry.name}`,
      })),
    ...bundled.map((entry) => ({ name: entry.name, url: `./replays/${entry.file}` })),
  ];
}

export async function loadReplay(url: string): Promise<Uint8Array> {
  const response = await fetch(url);
  if (!response.ok) {
    throw new Error(`The replay could not be loaded (${response.status}).`);
  }
  return new Uint8Array(await response.arrayBuffer());
}
