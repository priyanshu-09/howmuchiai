import { describe, expect, it } from "vitest";
import { evidenceTooltip, resolveTokenDisplay } from "./tokenDisplay";

describe("evidenceTooltip", () => {
  it("explains tier 3 when accurate count is not possible", () => {
    const tip = evidenceTooltip({
      tokens_estimate_method: "cl100k_base tokenizer; Tier-3 ...",
      tokens_estimate_evidence: {
        sources_checked: ["chrome_history", "chrome_local_storage"],
        winner_tier: 3,
        winner_reason: "tier3_hours_floor_exceeded_tier2",
        winner_reason_detail: "18 sidebar conversations with Tier-3 floor",
        provider_logged_usage_found: false,
        accurate_count_possible: false,
        winner_tokens: 5_820_000,
      },
    });
    expect(tip).toContain("No provider-logged usage on disk");
    expect(tip).toContain("Tier 3");
    expect(tip).toContain("chrome_local_storage");
  });
});

describe("resolveTokenDisplay", () => {
  it("marks cost as estimated when evidence says accurate_count_possible is false", () => {
    const display = resolveTokenDisplay(
      {
        tokens: {
          input_tokens: 0,
          output_tokens: 5_820_000,
          cache_read_tokens: 0,
          cache_creation_tokens: 0,
          total: 5_820_000,
        },
        tokens_estimated: true,
        tokens_estimate_method: "cl100k_base tokenizer; Tier-3",
        tokens_estimate_evidence: {
          sources_checked: ["chrome_history"],
          winner_tier: 3,
          winner_reason: "tier3_active_hours",
          winner_reason_detail: "4.8h active browser time",
          provider_logged_usage_found: false,
          accurate_count_possible: false,
          winner_tokens: 5_820_000,
        },
      },
      true,
    );
    expect(display.kind).toBe("estimated");
    expect(display.costEstimated).toBe(true);
    expect(display.label).toMatch(/^≈/);
  });

  it("marks cost as logged when tier 1 evidence is accurate", () => {
    const display = resolveTokenDisplay(
      {
        tokens: {
          input_tokens: 0,
          output_tokens: 50_000,
          cache_read_tokens: 0,
          cache_creation_tokens: 0,
          total: 50_000,
        },
        tokens_estimated: true,
        tokens_estimate_evidence: {
          sources_checked: ["chrome_indexeddb"],
          winner_tier: 1,
          winner_reason: "tier1_idb_transcripts",
          winner_reason_detail: "IndexedDB message JSON",
          provider_logged_usage_found: false,
          accurate_count_possible: true,
          winner_tokens: 50_000,
        },
      },
      true,
    );
    expect(display.costEstimated).toBe(false);
  });
});
