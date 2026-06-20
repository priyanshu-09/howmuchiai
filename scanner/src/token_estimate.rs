use crate::types::TokenUsage;
use tiktoken_rs::cl100k_base;

/// Prefix stamped on every locally-derived token count so dashboards never
/// treat estimates as provider-logged telemetry.
pub const ESTIMATE_METHOD_PREFIX: &str = "cl100k_base tokenizer";

/// Count tokens for a single text blob using the shared cl100k_base encoder.
pub fn count_tokens(text: &str) -> u64 {
    if text.trim().is_empty() {
        return 0;
    }
    match cl100k_base() {
        Ok(bpe) => bpe.encode_with_special_tokens(text).len() as u64,
        Err(_) => 0,
    }
}

/// Tokenize user vs assistant text segments into a `TokenUsage` aggregate.
pub fn token_usage_from_segments(input_text: &str, output_text: &str) -> TokenUsage {
    let input_tokens = count_tokens(input_text);
    let output_tokens = count_tokens(output_text);
    let mut usage = TokenUsage {
        input_tokens,
        output_tokens,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        total: 0,
    };
    usage.compute_total();
    usage
}

/// Merge many text blobs into one estimate (all counted as output-side text).
pub fn token_usage_from_texts(texts: &[String]) -> TokenUsage {
    let joined = texts.join("\n");
    let mut usage = TokenUsage {
        input_tokens: 0,
        output_tokens: count_tokens(&joined),
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        total: 0,
    };
    usage.compute_total();
    usage
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn count_tokens_empty_is_zero() {
        assert_eq!(count_tokens(""), 0);
        assert_eq!(count_tokens("   "), 0);
    }

    #[test]
    fn count_tokens_nonempty_is_positive() {
        assert!(count_tokens("hello world") > 0);
    }

    #[test]
    fn token_usage_from_segments_splits_input_output() {
        let usage = token_usage_from_segments("user says hi", "assistant says hello");
        assert!(usage.input_tokens > 0);
        assert!(usage.output_tokens > 0);
        assert_eq!(usage.total, usage.input_tokens + usage.output_tokens);
    }
}
