export interface TokenUsage {
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  total: number;
}

export interface Tier1Candidate {
  tokens: number;
  message_chars: number;
  rejected?: string;
}

export interface Tier1_5Candidate {
  tokens: number;
  snippet_chars: number;
  snippet_count: number;
}

export interface Tier2Candidate {
  tokens: number;
  sessions?: number;
  session_kind?: string;
}

export interface Tier3Candidate {
  tokens: number;
  hours: number;
  benchmark_tph: number;
}

export interface TierCandidates {
  "1": Tier1Candidate;
  "1.5": Tier1_5Candidate;
  "2": Tier2Candidate;
  "3": Tier3Candidate;
}

export interface TokenEstimateEvidence {
  sources_checked: string[];
  winner_tier: number;
  winner_reason: string;
  winner_reason_detail: string;
  provider_logged_usage_found: boolean;
  accurate_count_possible: boolean;
  winner_tokens: number;
  tier_candidates?: TierCandidates;
}

export interface DomainEntry {
  visits?: number;
  hours?: number;
  tokens?: TokenUsage;
  tokens_estimated?: boolean;
  tokens_estimate_method?: string;
  tokens_unavailable?: boolean;
  tokens_unavailable_reason?: string;
  tokens_estimate_evidence?: TokenEstimateEvidence;
}

export interface ProviderResult {
  provider: string;
  hours?: number;
  tokens?: TokenUsage;
  tokens_estimated?: boolean;
  tokens_estimate_method?: string;
  metadata?: {
    domains?: Record<string, DomainEntry>;
    skipped?: string;
  };
}

export interface ScanResult {
  schema_version: number;
  sources: Record<string, ProviderResult>;
  totals: {
    hours: number;
    tokens: number;
    estimated_tokens?: number;
    sessions: number;
    visits: number;
    invocations: number;
  };
}

export type TokenDisplayKind = "logged" | "estimated" | "unavailable" | "none";

export interface TokenDisplay {
  kind: TokenDisplayKind;
  label: string;
  tooltip?: string;
  /** When true, dashboard should prefix cost (Rs) with ≈ */
  costEstimated?: boolean;
}
