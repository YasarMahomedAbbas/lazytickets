//! Per-project configuration: filters, presets, ordering.
//!
//! The schema mirrors `~/vault-automation/configs/*.json` (include/exclude/
//! status_order) extended with named presets. Loading from disk and git-remote
//! resolution arrive in M4; for now `ProjectConfig::travel_smart()` supplies an
//! inline config.

pub mod schema;
