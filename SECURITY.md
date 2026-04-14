# Security Policy

## Reporting a vulnerability

If you find a security vulnerability in howmuchiai, please report it responsibly.

**Do not open a public GitHub issue for security vulnerabilities.**

Instead, email: priyanshushukla9801@gmail.com with:
- Description of the vulnerability
- Steps to reproduce
- Potential impact
- Suggested fix (if any)

We will respond within 48 hours and work with you on a fix before public disclosure.

## Security design

howmuchiai is designed to handle sensitive local data safely:

- **Zero network calls** — no HTTP dependencies in `Cargo.toml`, verified by security audit
- **Read-only** — never writes to user directories, only to OS temp for SQLite copies
- **No URL storage** — browser history URLs are mapped to display names inside SQLite, never loaded into Rust memory
- **No command storage** — shell history matching returns only tool names and counts
- **No secret access** — never reads `.env`, API keys, cookies, or credentials
- **Temp file isolation** — SQLite copies use `0o600` permissions, auto-cleaned on drop
- **No user input in SQL** — all query parameters are compile-time constants
