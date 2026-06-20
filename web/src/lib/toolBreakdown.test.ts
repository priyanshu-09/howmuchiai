import { describe, expect, it } from "vitest";
import { buildToolBreakdown } from "./toolBreakdown";
import type { ScanResult } from "./types";

const EIGHT_HOURS_ONLY_TOOLS = [
  "chatgpt",
  "notebooklm",
  "gemini",
  "perplexity",
  "grok",
  "lovable",
  "ai-studio",
] as const;

describe("buildToolBreakdown", () => {
  it("never shows bare 0 tokens for all 8 hours-only dashboard tools", () => {
    const browserDomains: ScanResult["sources"][string]["metadata"] = {
      domains: {
        ChatGPT: {
          hours: 4.8,
          visits: 100,
          tokens_unavailable: true,
          tokens_unavailable_reason: "No local transcript",
        },
        NotebookLM: {
          hours: 1.7,
          visits: 20,
          tokens_unavailable: true,
          tokens_unavailable_reason: "History only",
        },
        Gemini: {
          hours: 0.8,
          visits: 15,
          tokens_unavailable: true,
        },
        Perplexity: {
          hours: 0.6,
          visits: 10,
          tokens_unavailable: true,
        },
        Grok: {
          hours: 0.3,
          visits: 5,
          tokens_unavailable: true,
        },
        Lovable: {
          hours: 0.2,
          visits: 3,
          tokens_unavailable: true,
        },
        "AI Studio": {
          hours: 0.1,
          visits: 2,
          tokens_unavailable: true,
        },
      },
    };

    const scan: ScanResult = {
      schema_version: 5,
      sources: {
        chrome_browser: {
          provider: "Chrome",
          hours: 7.5,
          metadata: browserDomains,
        },
        cursor: {
          provider: "Cursor IDE",
          hours: 16,
          tokens: {
            input_tokens: 0,
            output_tokens: 82_000_000,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            total: 82_000_000,
          },
          tokens_estimated: true,
          tokens_estimate_method: "cl100k_base over Cursor bubble transcripts",
        },
      },
      totals: {
        hours: 23.5,
        tokens: 0,
        estimated_tokens: 82_000_000,
        sessions: 0,
        visits: 0,
        invocations: 0,
      },
    };

    const rows = buildToolBreakdown(scan);

    for (const tool of EIGHT_HOURS_ONLY_TOOLS) {
      const row = rows.find((r) => r.name === tool);
      expect(row, `${tool} row`).toBeDefined();
      expect(row!.hours).toBeGreaterThan(0);
      expect(row!.tokenDisplay.kind).not.toBe("none");
      if (row!.tokenDisplay.kind === "logged") {
        expect(row!.tokens).toBeGreaterThan(0);
      } else {
        expect(["estimated", "unavailable"]).toContain(row!.tokenDisplay.kind);
        expect(row!.tokenDisplay.label).not.toBe("0");
      }
    }

    const cursor = rows.find((r) => r.name === "cursor");
    expect(cursor?.tokenDisplay.kind).toBe("estimated");
    expect(cursor?.tokenDisplay.label).toMatch(/^≈/);
  });

  it("shows unavailable instead of 0 for browser domains with hours only", () => {
    const scan: ScanResult = {
      schema_version: 5,
      sources: {
        chrome_browser: {
          provider: "Chrome",
          hours: 4.8,
          metadata: {
            domains: {
              ChatGPT: {
                hours: 4.8,
                visits: 100,
                tokens_unavailable: true,
                tokens_unavailable_reason: "No local transcript",
              },
            },
          },
        },
        cursor: {
          provider: "Cursor IDE",
          hours: 16,
          tokens: {
            input_tokens: 1,
            output_tokens: 99,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            total: 100,
          },
          tokens_estimated: true,
        },
      },
      totals: {
        hours: 20.8,
        tokens: 0,
        estimated_tokens: 100,
        sessions: 0,
        visits: 0,
        invocations: 0,
      },
    };

    const rows = buildToolBreakdown(scan);
    const chatgpt = rows.find((r) => r.name === "chatgpt");
    const cursor = rows.find((r) => r.name === "cursor");

    expect(chatgpt?.tokenDisplay.kind).toBe("unavailable");
    expect(chatgpt?.tokenDisplay.label).toBe("—");
    expect(cursor?.tokenDisplay.kind).toBe("estimated");
    expect(cursor?.tokenDisplay.label).toBe("≈100");
  });
});
