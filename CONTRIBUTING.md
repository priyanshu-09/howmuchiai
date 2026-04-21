# Contributing to howmuchiai

Thanks for your interest! This repo is the open-source scanner for [howmuchiai.xyz](https://howmuchiai.xyz). It scans local machines for AI tool usage — no network calls, no business logic.

## Development setup

```bash
git clone https://github.com/priyanshu-09/howmuchiai.git
cd howmuchiai
cargo build
cargo run -- --no-open
```

## Adding a new provider

Each AI tool scanner is an independent provider module. To add one:

1. Create `scanner/src/providers/your_provider.rs`
2. Implement the `Provider` trait:

```rust
pub struct YourProvider;

impl Provider for YourProvider {
    fn name(&self) -> &'static str { "your_provider" }
    fn display_name(&self) -> &'static str { "Your Tool Name" }
    fn is_available(&self) -> bool { /* check if data source exists */ }
    fn scan(&self) -> Result<ProviderResult, ScanError> { /* scan logic */ }
}
```

3. Add platform paths to `scanner/src/platform.rs`
4. Register in `scanner/src/providers/mod.rs`
5. Test on your machine

### Provider rules

- **Never panic.** Use `Result` and `?` everywhere.
- **Never network.** Providers read local files only.
- **Never read secrets.** Don't touch `.env`, API keys, auth tokens, or cookies.
- **Never store URLs.** Use SQL `CASE` expressions to convert URLs to display names.
- **Never store commands.** Shell history matching returns only tool names.
- **Use SafeSqlite.** Always copy locked SQLite DBs to temp before reading.
- **No business logic.** The scanner outputs raw data. The website handles tiers, cards, and presentation.

## Reporting security issues

See [SECURITY.md](SECURITY.md).
