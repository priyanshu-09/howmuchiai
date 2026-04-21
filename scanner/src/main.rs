use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use clap::Parser;

const WEBSITE_BASE: &str = "https://howmuchiai.xyz";

#[derive(Parser)]
#[command(name = "howmuchiai", about = "Scan your machine for AI tool usage")]
struct Cli {
    /// Output format: url (default) or json
    #[arg(short, long, default_value = "url")]
    format: String,

    /// Don't auto-open the browser
    #[arg(long)]
    no_open: bool,
}

fn main() {
    let cli = Cli::parse();

    eprintln!("\n\x1b[1m\u{1f50d} Scanning for AI usage...\x1b[0m\n");

    let result = howmuchiai::run_scan();

    eprintln!();

    match cli.format.as_str() {
        "json" => {
            // Raw JSON to stdout — for debugging, piping, or advanced use
            println!("{}", serde_json::to_string(&result).unwrap_or_default());
        }
        _ => {
            // Default: show compact summary on stderr, URL on stdout

            // Count successful sources
            let source_count = result.sources.len();

            // Show summary stats
            let t = &result.totals;
            let mut parts = Vec::new();
            if t.hours > 0.0 {
                parts.push(format!("{:.1}h", t.hours));
            }
            if t.tokens > 0 {
                parts.push(format!("{} tokens", format_number(t.tokens)));
            }
            if t.sessions > 0 {
                parts.push(format!("{} sessions", t.sessions));
            }
            if t.visits > 0 {
                parts.push(format!("{} visits", format_number(t.visits)));
            }

            eprintln!(
                "  \x1b[1mScanned {} sources in {}ms\x1b[0m",
                source_count, result.scan_duration_ms
            );
            if !parts.is_empty() {
                eprintln!("  {}\n", parts.join(" \u{2022} "));
            }

            // Encode scan data as base64url hash
            let scan_json = serde_json::to_string(&result).unwrap_or_default();
            let hash = URL_SAFE_NO_PAD.encode(&scan_json);
            let url = format!("{}/c/{}", WEBSITE_BASE, hash);

            // URL to stdout (pipeable)
            println!("{}", url);

            eprintln!("  \x1b[90mYour card \u{2192} open the link above\x1b[0m");

            // Auto-open browser
            if !cli.no_open {
                let _ = open_browser(&url);
            }
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

fn open_browser(url: &str) -> Result<(), std::io::Error> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open").arg(url).spawn()?;
    }
    #[cfg(target_os = "linux")]
    {
        std::process::Command::new("xdg-open").arg(url).spawn()?;
    }
    Ok(())
}
