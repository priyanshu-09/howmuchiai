# Changelog

All notable changes to this project are documented here.

## [Unreleased]
### Added
- `ScanResult.schema_version` (u32, defaults to 2) so the web app can detect schema epochs.
- `ProviderResult.daily_buckets` (optional map of YYYY-MM-DD → hours/tokens/sessions/invocations) for real streak + heatmap widgets. Implemented for shell_history and browser providers; other providers may opt in later.
- Windows release binary (x86_64-pc-windows-msvc).
