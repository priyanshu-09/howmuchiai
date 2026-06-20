import type { DomainEntry, ProviderResult, TokenDisplay } from "./types";

export function formatCompact(n: number): string {
  if (n >= 1_000_000_000) return `${(n / 1_000_000_000).toFixed(1)}B`;
  if (n >= 1_000_000) return `${(n / 1_000_000).toFixed(1)}M`;
  if (n >= 1_000) return `${(n / 1_000).toFixed(1)}K`;
  return String(n);
}

function tierLabel(tier: number): string {
  if (tier === 1) return "Tier 1 (IndexedDB transcripts)";
  if (tier === 1.5) return "Tier 1.5 (Local Storage snippets)";
  if (tier === 2) return "Tier 2 (session metadata)";
  if (tier === 3) return "Tier 3 (active-hours benchmark)";
  return `Tier ${tier}`;
}

/** Build tooltip from schema v6 evidence when present. */
export function evidenceTooltip(
  entry: Pick<DomainEntry, "tokens_estimate_evidence" | "tokens_estimate_method">,
): string | undefined {
  const evidence = entry.tokens_estimate_evidence;
  if (!evidence) {
    return entry.tokens_estimate_method;
  }

  const parts: string[] = [];
  if (evidence.accurate_count_possible) {
    parts.push("Counted from local message text on disk.");
  } else {
    parts.push(
      "No provider-logged usage on disk; closest local estimate.",
    );
  }
  if (evidence.winner_tier > 0) {
    parts.push(tierLabel(evidence.winner_tier));
  }
  if (evidence.winner_reason_detail) {
    parts.push(evidence.winner_reason_detail);
  } else if (entry.tokens_estimate_method) {
    parts.push(entry.tokens_estimate_method);
  }
  if (evidence.sources_checked?.length) {
    parts.push(`Sources checked: ${evidence.sources_checked.join(", ")}`);
  }
  return parts.join(" ");
}

/** Resolve how to render tokens for a provider or browser domain row. */
export function resolveTokenDisplay(
  entry: Pick<
    ProviderResult,
    "tokens" | "tokens_estimated" | "tokens_estimate_method"
  > &
    Pick<
      DomainEntry,
      | "tokens_unavailable"
      | "tokens_unavailable_reason"
      | "tokens_estimate_evidence"
    >,
  hasHours: boolean,
): TokenDisplay {
  if (entry.tokens_unavailable) {
    const auditDetail = entry.tokens_estimate_evidence?.winner_reason_detail;
    return {
      kind: "unavailable",
      label: "—",
      tooltip:
        entry.tokens_unavailable_reason ??
        auditDetail ??
        "Token usage not available from local data",
    };
  }

  const total = entry.tokens?.total ?? 0;
  if (total > 0 && entry.tokens_estimated) {
    const accurate = entry.tokens_estimate_evidence?.accurate_count_possible;
    const prefix = accurate ? "≈" : "≈";
    return {
      kind: "estimated",
      label: `${prefix}${formatCompact(total)}`,
      tooltip: evidenceTooltip(entry),
      costEstimated: accurate === false || accurate === undefined,
    };
  }

  if (total > 0) {
    return { kind: "logged", label: formatCompact(total), costEstimated: false };
  }

  if (hasHours) {
    return {
      kind: "unavailable",
      label: "—",
      tooltip: "Hours tracked from browser history; no local token telemetry",
    };
  }

  return { kind: "none", label: "" };
}
