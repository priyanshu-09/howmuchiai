use crate::types::Totals;

pub fn compute_tier(totals: &Totals) -> String {
    let hours = totals.hours;
    let tokens = totals.tokens;

    if hours >= 1000.0 || tokens >= 10_000_000 {
        "The Singularity".to_string()
    } else if hours >= 500.0 || tokens >= 5_000_000 {
        "Neural Link".to_string()
    } else if hours >= 200.0 || tokens >= 2_000_000 {
        "The Architect".to_string()
    } else if hours >= 100.0 || tokens >= 1_000_000 {
        "Prompt Native".to_string()
    } else if hours >= 50.0 {
        "Vibe Coder".to_string()
    } else if hours >= 10.0 {
        "The Explorer".to_string()
    } else {
        "The Purist".to_string()
    }
}
