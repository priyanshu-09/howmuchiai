# Contributing to howmuchiai

Thanks for your interest in contributing! Here's how to get started.

## Development setup

```bash
# Clone the repo
git clone https://github.com/priyanshu-09/howmuchiai.git
cd howmuchiai

# Build
cargo build

# Run
cargo run

# Run with JSON output
cargo run -- --format json
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
4. Register in `scanner/src/providers/mod.rs`:
   - Add `pub mod your_provider;`
   - Add `Box::new(your_provider::YourProvider)` to `all_providers()`
5. Test on your machine

### Provider rules

- **Never panic.** Use `Result` and `?` everywhere. Return `ScanError::NotFound` if data doesn't exist.
- **Never network.** Providers read local files only. No HTTP, no APIs, no DNS.
- **Never read secrets.** Don't touch `.env`, API keys, auth tokens, or cookies.
- **Never store URLs.** Use SQL `CASE` expressions to convert URLs to display names server-side.
- **Never store commands.** Shell history matching returns only tool names, never command text.
- **Skip gracefully.** If a data source doesn't exist, return an error — don't crash.
- **Use SafeSqlite.** Always copy locked SQLite DBs to temp before reading.

## Pull requests

- One PR per feature/fix
- Include what you tested and on which platform (macOS/Linux)
- Keep PRs focused — don't bundle unrelated changes

## Reporting security issues

See [SECURITY.md](SECURITY.md).
