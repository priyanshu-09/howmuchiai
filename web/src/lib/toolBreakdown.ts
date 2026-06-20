import { resolveTokenDisplay } from "./tokenDisplay";
import type { DomainEntry, ProviderResult, ScanResult, TokenDisplay } from "./types";

/** Scanner source key → dashboard tool key */
export const PROVIDER_TOOL_KEYS: Record<string, string> = {
  claude_code: "claude",
  chatgpt_desktop: "chatgpt",
  cursor: "cursor",
  codex: "codex",
  gemini_cli: "gemini",
  aider: "aider",
  continue_dev: "continue",
  continue: "continue",
  windsurf: "windsurf",
  zed: "zed",
  v0: "v0",
  perplexity: "perplexity",
  ollama: "ollama",
  warp: "warp",
  jetbrains: "jetbrains",
  qwen_cli: "qwen",
  kimi_cli: "kimi",
  opencode: "opencode",
  amp: "opencode",
  droid: "opencode",
  openclaw: "openclaw",
};

/** Browser metadata.domains display name → dashboard tool key */
export const DOMAIN_TOOL_KEYS: Record<string, string> = {
  "Claude.ai": "claude",
  ChatGPT: "chatgpt",
  Gemini: "gemini",
  Perplexity: "perplexity",
  Copilot: "copilot",
  v0: "v0",
  "Cursor Agents": "cursor",
  "AI Studio": "ai-studio",
  "Google Labs": "ai-studio",
  NotebookLM: "notebooklm",
  Poe: "poe",
  Phind: "phind",
  DeepSeek: "deepseek",
  Groq: "groq",
  HuggingFace: "huggingface",
  "You.com": "you",
  Lovable: "lovable",
  Devin: "devin",
  Bolt: "bolt",
  Grok: "grok",
  "Meta AI": "meta",
  "Character.AI": "claude",
  "Le Chat (Mistral)": "le-chat",
  Qwen: "qwen",
  Kimi: "kimi",
};

export interface ToolRow {
  name: string;
  provider: string;
  hours: number;
  tokens: number;
  tokenDisplay: TokenDisplay;
  pct: number;
}

function isBrowserSource(key: string): boolean {
  return key.endsWith("_browser");
}

function addRow(
  rows: Map<string, ToolRow>,
  toolKey: string,
  hours: number,
  tokens: number,
  tokenDisplay: TokenDisplay,
  provider: string,
) {
  const existing = rows.get(toolKey);
  if (existing) {
    existing.hours += hours;
    if (tokenDisplay.kind === "logged") {
      existing.tokens += tokens;
      existing.tokenDisplay = tokenDisplay;
    } else if (
      existing.tokenDisplay.kind !== "logged" &&
      tokenDisplay.kind !== "none"
    ) {
      existing.tokenDisplay = tokenDisplay;
    }
  } else {
    rows.set(toolKey, {
      name: toolKey,
      provider,
      hours,
      tokens,
      tokenDisplay,
      pct: 0,
    });
  }
}

/** Patched breakdown builder — never shows bare 0 tokens when hours exist but telemetry is missing. */
export function buildToolBreakdown(scan: ScanResult, limit = Infinity): ToolRow[] {
  const totalHours = Math.max(scan.totals.hours, 1);
  const rows = new Map<string, ToolRow>();

  for (const [sourceKey, provider] of Object.entries(scan.sources)) {
    if (isBrowserSource(sourceKey)) {
      const domains = provider.metadata?.domains ?? {};
      for (const [domainName, domain] of Object.entries(domains)) {
        const toolKey = DOMAIN_TOOL_KEYS[domainName];
        if (!toolKey) continue;
        const hours = domain.hours ?? 0;
        const display = resolveTokenDisplay(domain, hours > 0);
        const tokens =
          display.kind === "logged" ? (domain.tokens?.total ?? 0) : 0;
        addRow(rows, toolKey, hours, tokens, display, sourceKey);
      }
      continue;
    }

    const toolKey = PROVIDER_TOOL_KEYS[sourceKey];
    if (!toolKey) continue;

    const hours = provider.hours ?? 0;
    const display = resolveTokenDisplay(provider, hours > 0);
    const tokens =
      display.kind === "logged" || display.kind === "estimated"
        ? (provider.tokens?.total ?? 0)
        : 0;
    addRow(rows, toolKey, hours, tokens, display, sourceKey);
  }

  return Array.from(rows.values())
    .map((row) => ({
      ...row,
      pct: Math.round((row.hours / totalHours) * 100),
    }))
    .filter((row) => row.hours > 0 || row.tokens > 0)
    .sort((a, b) => b.hours - a.hours)
    .slice(0, limit);
}
