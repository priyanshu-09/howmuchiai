use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use clap::Parser;

#[derive(Parser)]
#[command(name = "howmuchiai", about = "Scan your machine for AI tool usage")]
struct Cli {
    /// Output format: json, pretty, or card-url
    #[arg(short, long, default_value = "pretty")]
    format: String,
}

fn main() {
    let cli = Cli::parse();

    eprintln!("\n\x1b[1m🔍 Scanning for AI usage...\x1b[0m\n");

    let result = howmuchiai::run_scan();

    eprintln!();

    match cli.format.as_str() {
        "json" => {
            println!("{}", serde_json::to_string(&result).unwrap());
        }
        "card-url" => {
            let compact = serde_json::json!({
                "t": result.totals,
                "s": result.sources.keys().collect::<Vec<_>>(),
                "tier": result.tier,
                "at": result.scanned_at,
            });
            let encoded = URL_SAFE_NO_PAD.encode(serde_json::to_string(&compact).unwrap());
            println!("howmuchiai.xyz/c/{}", encoded);
        }
        _ => {
            // Pretty print
            println!("\x1b[1m═══════════════════════════════════════\x1b[0m");
            println!("\x1b[1m  How Much I AI'd\x1b[0m");
            println!("\x1b[1m═══════════════════════════════════════\x1b[0m\n");

            println!("  \x1b[1mTier:\x1b[0m {}", result.tier);
            println!("  \x1b[1mTotal Hours:\x1b[0m {:.1}", result.totals.hours);
            println!(
                "  \x1b[1mTotal Tokens:\x1b[0m {}",
                format_number(result.totals.tokens)
            );
            println!("  \x1b[1mSessions:\x1b[0m {}", result.totals.sessions);
            if result.totals.visits > 0 {
                println!(
                    "  \x1b[1mBrowser Visits:\x1b[0m {}",
                    format_number(result.totals.visits)
                );
            }
            if result.totals.invocations > 0 {
                println!(
                    "  \x1b[1mCLI Invocations:\x1b[0m {}",
                    result.totals.invocations
                );
            }

            println!("\n  \x1b[1m--- By Source ---\x1b[0m\n");

            let mut sorted_sources: Vec<_> = result.sources.iter().collect();
            sorted_sources.sort_by(|a, b| {
                let hours_a = a.1.hours.unwrap_or(0.0);
                let hours_b = b.1.hours.unwrap_or(0.0);
                hours_b
                    .partial_cmp(&hours_a)
                    .unwrap_or(std::cmp::Ordering::Equal)
            });

            for (name, source) in &sorted_sources {
                println!("  \x1b[36m{}\x1b[0m", name);
                if let Some(h) = source.hours {
                    println!("    Hours: {:.1}", h);
                }
                if let Some(ref t) = source.tokens {
                    if t.total > 0 {
                        println!("    Tokens: {}", format_number(t.total));
                    }
                }
                if let Some(s) = source.sessions {
                    println!("    Sessions: {}", s);
                }
                if let Some(v) = source.visits {
                    println!("    Visits: {}", v);
                }
                if let Some(i) = source.invocations {
                    println!("    Invocations: {}", i);
                }
                println!();
            }

            if !result.detected_tools.is_empty() {
                println!("  \x1b[1m--- Also Detected ---\x1b[0m\n");
                for tool in &result.detected_tools {
                    println!("  \x1b[90m• {}\x1b[0m", tool);
                }
                println!();
            }

            println!(
                "  Scanned in {}ms on {}\n",
                result.scan_duration_ms, result.platform
            );
        }
    }
}

fn format_number(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        n.to_string()
    }
}
