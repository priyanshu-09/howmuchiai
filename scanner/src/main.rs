use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use clap::Parser;
use flate2::write::GzEncoder;
use flate2::Compression;
use howmuchiai::types::ScanResult;
use std::io::Write;

const WEBSITE_BASE: &str = "https://howmuchiai.xyz";

/// Encode a ScanResult into a share-URL-safe hash:
///
/// 1. Serialize JSON
/// 2. gzip (best compression — runs once per scan, 50ms is fine)
/// 3. base64url, no padding
///
/// On a full scan the raw JSON reaches ~85 KB, which is over Cloudflare
/// Pages' `_redirects` rewrite buffer and triggers error 1036 in URL-length-
/// strict browsers (Dia, Safari). Gzip brings it under ~20 KB.
fn encode_share_hash(scan: &ScanResult) -> Result<String, std::io::Error> {
    let scan_json = serde_json::to_vec(scan)?;
    let mut encoder = GzEncoder::new(Vec::new(), Compression::best());
    encoder.write_all(&scan_json)?;
    let gz = encoder.finish()?;
    Ok(URL_SAFE_NO_PAD.encode(gz))
}

/// Count providers whose result metadata is `{ "skipped": "tcc-denied" }`.
/// macOS Full Disk Access (TCC) gates `~/Library/Safari/*` behind a Finder grant
/// the CLI cannot self-issue, so a denied provider returns Ok(empty) with
/// this sentinel rather than aborting the whole scan.
fn count_tcc_denied(result: &ScanResult) -> usize {
    result
        .sources
        .values()
        .filter(|p| {
            p.metadata
                .as_ref()
                .and_then(|m| m.get("skipped"))
                .and_then(|v| v.as_str())
                == Some("tcc-denied")
        })
        .count()
}

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

            // Encode scan data as gzip + base64url hash. Always gzip — no
            // sentinel prefix, no fallback. Web decoder always gunzips.
            let hash = encode_share_hash(&result).unwrap_or_default();
            let url = format!("{}/c/{}", WEBSITE_BASE, hash);

            // Output policy:
            //   - Interactive tty: just tell the user we're opening their card.
            //     The full URL is visually jarring and the browser open handles
            //     the common case. Use `howmuchiai --no-open` or pipe to see it.
            //   - Piped / redirected stdout: emit URL so `howmuchiai | pbcopy`
            //     and similar scripting flows still work.
            //   - `--no-open`: print URL (caller explicitly opted out of the
            //     browser hand-off, they need the URL).
            let stdout_is_tty = std::io::IsTerminal::is_terminal(&std::io::stdout());
            if !stdout_is_tty || cli.no_open {
                println!("{}", url);
            }

            if stdout_is_tty {
                eprintln!(
                    "  \x1b[1m\u{2713}\x1b[0m Your card is ready \u{2192} \x1b[90mopening in your browser\u{2026}\x1b[0m"
                );
                eprintln!("  \x1b[90m(pass --no-open to print the URL instead)\x1b[0m");
            }

            // Auto-open browser
            if !cli.no_open {
                let _ = open_browser(&url);
            }

            // TCC (Full Disk Access) hint — fires when ≥1 provider returned
            // skipped=tcc-denied (e.g. Safari on a terminal without FDA).
            let skipped = count_tcc_denied(&result);
            if skipped > 0 {
                eprintln!();
                eprintln!(
                    "\u{26a0}  {} provider(s) skipped due to macOS Full Disk Access.",
                    skipped
                );
                eprintln!(
                    "    Grant access: System Settings \u{2192} Privacy & Security \u{2192} Full Disk Access"
                );
                eprintln!(
                    "    \u{2192} add your terminal app (Terminal / iTerm / Ghostty / etc.)."
                );
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
