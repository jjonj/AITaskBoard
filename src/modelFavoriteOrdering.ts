import type { TaskModelSelection } from "./types";

export type ModelFavorite = Pick<TaskModelSelection, "provider" | "model">;

function providerGroup(provider: string): number {
  if (provider === "codex" || provider.startsWith("codex_")) return 0;
  if (provider === "claudeAgent" || provider.startsWith("claudeAgent_")) return 1;
  if (provider === "opencode" || provider.startsWith("opencode_")) return 2;
  return 3;
}

function version(slug: string): number {
  const [, major, minor = "0"] = /(?:gpt-|claude-(?:opus|sonnet|fable|haiku)-)(\d+)(?:[.-](\d+))?/i.exec(slug) ?? [];
  return Number(major || 0) * 1000 + Number(minor || 0);
}

function modelTier(model: string, provider: number): number {
  if (provider === 0) {
    const tier = /-(sol|astra|terra|luna)(?:-|$)/i.exec(model)?.[1]?.toLowerCase();
    return ["sol", "astra", "terra", "luna"].indexOf(tier ?? "") < 0
      ? 4 : ["sol", "astra", "terra", "luna"].indexOf(tier!);
  }
  if (provider === 1) {
    const tier = /claude-(opus|sonnet|fable|haiku)/i.exec(model)?.[1]?.toLowerCase();
    return ["opus", "sonnet", "fable", "haiku"].indexOf(tier ?? "") < 0
      ? 4 : ["opus", "sonnet", "fable", "haiku"].indexOf(tier!);
  }
  return 0;
}

/** T3-style favorites grouping: Codex GPTs, Claude families, then OpenCode and
 * other providers. GPTs sort newest-first, Claude by Opus/Sonnet/Fable/Haiku.
 * Preserve T3's original order for other providers. ChatGPT is added by the
 * caller as an independent first row, never sorted among native favorites. */
export function orderTaskModelFavorites<T extends ModelFavorite>(favorites: readonly T[]): T[] {
  return favorites.map((entry, index) => ({ entry, index })).sort((a, b) => {
    const ap = providerGroup(a.entry.provider);
    const bp = providerGroup(b.entry.provider);
    if (ap !== bp) return ap - bp;
    if (ap === 0 || ap === 1) {
      if (ap === 0) {
        const versionOrder = version(b.entry.model) - version(a.entry.model);
        if (versionOrder) return versionOrder;
      }
      const tierOrder = modelTier(a.entry.model, ap) - modelTier(b.entry.model, bp);
      if (tierOrder) return tierOrder;
      if (ap === 1) {
        const versionOrder = version(b.entry.model) - version(a.entry.model);
        if (versionOrder) return versionOrder;
      }
    }
    return a.index - b.index;
  }).map(({ entry }) => entry);
}
