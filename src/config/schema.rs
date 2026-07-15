//! Config types + the pure filtering/ordering logic they drive.

use crate::model::Item;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    pub owner: String,
    pub number: u32,
    /// Canonical column order, used to sort the list.
    #[serde(default)]
    pub status_order: Vec<String>,
    /// Statuses hidden by default (unless a preset explicitly includes one).
    #[serde(default)]
    pub exclude_statuses: Vec<String>,
    /// Saved views. The first entry is the default on launch.
    #[serde(default)]
    pub presets: Vec<Preset>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default)]
    pub include: Filter,
}

/// An include filter. Empty fields don't constrain; within a field the match is
/// OR, across fields it's AND (e.g. label Frontend AND status Refine).
#[derive(Debug, Clone, Default, Deserialize)]
pub struct Filter {
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub statuses: Vec<String>,
    #[serde(default)]
    pub assignees: Vec<String>,
}

impl Filter {
    pub fn matches(&self, item: &Item) -> bool {
        let labels_ok = self.labels.is_empty()
            || self
                .labels
                .iter()
                .any(|want| item.labels.iter().any(|have| have.eq_ignore_ascii_case(want)));

        let status_ok = self.statuses.is_empty()
            || item
                .status
                .as_deref()
                .is_some_and(|s| self.statuses.iter().any(|want| want.eq_ignore_ascii_case(s)));

        let assignees_ok = self.assignees.is_empty()
            || self
                .assignees
                .iter()
                .any(|want| item.assignees.iter().any(|have| have.eq_ignore_ascii_case(want)));

        labels_ok && status_ok && assignees_ok
    }
}

impl ProjectConfig {
    /// Sort key for a status: its index in `status_order`, or a large value for
    /// unknown statuses (which sink to the bottom).
    pub fn status_rank(&self, status: Option<&str>) -> usize {
        match status {
            Some(s) => self
                .status_order
                .iter()
                .position(|o| o.eq_ignore_ascii_case(s))
                .unwrap_or(usize::MAX - 1),
            None => usize::MAX,
        }
    }

    /// Whether a preset's include filter should keep this item, honouring
    /// `exclude_statuses` unless the preset explicitly names that status.
    pub fn keeps(&self, preset: &Preset, item: &Item) -> bool {
        if !preset.include.matches(item) {
            return false;
        }
        if let Some(status) = item.status.as_deref() {
            let excluded = self.exclude_statuses.iter().any(|e| e.eq_ignore_ascii_case(status));
            let preset_wants = preset.include.statuses.iter().any(|s| s.eq_ignore_ascii_case(status));
            if excluded && !preset_wants {
                return false;
            }
        }
        true
    }

    /// The inline travel-smart config used until M4 wires up disk loading.
    pub fn travel_smart() -> Self {
        let preset = |name: &str, f: Filter| Preset {
            name: name.to_string(),
            include: f,
        };
        ProjectConfig {
            name: "travel-smart".to_string(),
            owner: "WhiteWolfStudio".to_string(),
            number: 6,
            status_order: [
                "Refine",
                "Create Contract",
                "Ready To Implement",
                "In progress",
                "In review",
                "Backlog",
                "Done",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
            exclude_statuses: vec!["Done".to_string(), "Backlog".to_string()],
            presets: vec![
                preset("all", Filter::default()),
                preset(
                    "mine",
                    Filter {
                        assignees: vec!["YasarMahomedAbbas".to_string()],
                        ..Default::default()
                    },
                ),
                preset(
                    "frontend",
                    Filter {
                        labels: vec!["Frontend".to_string()],
                        ..Default::default()
                    },
                ),
                preset(
                    "refine",
                    Filter {
                        statuses: vec!["Refine".to_string()],
                        ..Default::default()
                    },
                ),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(status: &str, labels: &[&str], assignees: &[&str]) -> Item {
        Item {
            id: "x".into(),
            number: Some(1),
            title: "t".into(),
            repository: Some("o/r".into()),
            status: Some(status.into()),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            assignees: assignees.iter().map(|s| s.to_string()).collect(),
            url: None,
        }
    }

    #[test]
    fn preset_filters_and_excludes() {
        let cfg = ProjectConfig::travel_smart();
        let frontend = &cfg.presets[2];
        assert_eq!(frontend.name, "frontend");

        // Frontend + Refine passes the frontend preset.
        assert!(cfg.keeps(frontend, &item("Refine", &["Frontend"], &[])));
        // Backend label fails it.
        assert!(!cfg.keeps(frontend, &item("Refine", &["Backend"], &[])));
        // Frontend but Done is excluded (preset doesn't name Done).
        assert!(!cfg.keeps(frontend, &item("Done", &["Frontend"], &[])));

        // `mine` matches on assignee.
        let mine = &cfg.presets[1];
        assert!(cfg.keeps(mine, &item("Refine", &[], &["YasarMahomedAbbas"])));
        assert!(!cfg.keeps(mine, &item("Refine", &[], &["someone-else"])));
    }

    #[test]
    fn status_rank_orders_known_before_unknown() {
        let cfg = ProjectConfig::travel_smart();
        assert!(cfg.status_rank(Some("Refine")) < cfg.status_rank(Some("Done")));
        assert!(cfg.status_rank(Some("Done")) < cfg.status_rank(Some("Nonsense")));
        assert!(cfg.status_rank(Some("Nonsense")) < cfg.status_rank(None));
    }
}
