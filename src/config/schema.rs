//! Config types + the pure filtering/ordering logic they drive.

use crate::model::Item;
use serde::{Deserialize, Serialize};

/// A single project entry in the global config. Modeled on PLAN.md §4.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectConfig {
    pub name: String,
    /// `owner/name` repos that resolve to this project (git-remote match).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub repos: Vec<String>,
    /// The GitHub Projects v2 board this project reads.
    pub board: Board,
    /// tmux session for start-work (M5). Optional until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_session: Option<String>,
    /// Subdir Claude boots inside for worktree-start (`t`), e.g. `Frontend` when
    /// that folder has its own CLAUDE.md. Unset → the worktree root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude_subdir: Option<String>,
    /// Skill names invoked by start-work / create (M5).
    #[serde(default, skip_serializing_if = "Skill::is_empty")]
    pub skill: Skill,
    /// Files + setup commands that make a fresh worktree runnable (worktree-start).
    #[serde(default, skip_serializing_if = "Worktree::is_empty")]
    pub worktree: Worktree,
    /// Canonical column order, used to sort the list.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub status_order: Vec<String>,
    /// Statuses hidden by default (unless a preset explicitly includes one).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude_statuses: Vec<String>,
    /// Saved views. The first entry is the default on launch.
    #[serde(default)]
    pub presets: Vec<Preset>,
}

/// A GitHub Projects v2 board coordinate.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Board {
    pub owner: String,
    pub number: u32,
}

/// Skill names linked to a project (M5 start-work). Both optional; falls back to
/// a global `~/.claude/skills/` skill when unset.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Skill {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub create: Option<String>,
}

impl Skill {
    fn is_empty(&self) -> bool {
        self.start.is_none() && self.create.is_none()
    }
}

/// Post-create bootstrap for worktree-start (`t`). A fresh worktree only has
/// *tracked* files, so gitignored env files are missing and dependencies aren't
/// installed — these make it actually runnable.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Worktree {
    /// Paths (relative to the repo root) copied from the main checkout into the
    /// new worktree at the same relative path — for gitignored files like `.env`
    /// that a fresh checkout lacks. A missing source is skipped, not an error.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub copy: Vec<String>,
    /// Shell commands run in the new session (in the launch dir) before Claude
    /// starts, e.g. `npm install`. Joined with `&&`; Claude launches regardless of
    /// their exit status, so its scrollback shows any failure.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub setup: Vec<String>,
}

impl Worktree {
    fn is_empty(&self) -> bool {
        self.copy.is_empty() && self.setup.is_empty()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preset {
    pub name: String,
    #[serde(default, skip_serializing_if = "Filter::is_empty")]
    pub include: Filter,
}

/// An include filter. Empty fields don't constrain; within a field the match is
/// OR, across fields it's AND (e.g. label Frontend AND status Refine).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Filter {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub labels: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub statuses: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub assignees: Vec<String>,
}

impl Filter {
    fn is_empty(&self) -> bool {
        self.labels.is_empty() && self.statuses.is_empty() && self.assignees.is_empty()
    }

    pub fn matches(&self, item: &Item) -> bool {
        let labels_ok = self.labels.is_empty()
            || self.labels.iter().any(|want| {
                item.labels
                    .iter()
                    .any(|have| have.eq_ignore_ascii_case(want))
            });

        let status_ok = self.statuses.is_empty()
            || item.status.as_deref().is_some_and(|s| {
                self.statuses
                    .iter()
                    .any(|want| want.eq_ignore_ascii_case(s))
            });

        let assignees_ok = self.assignees.is_empty()
            || self.assignees.iter().any(|want| {
                item.assignees
                    .iter()
                    .any(|have| have.eq_ignore_ascii_case(want))
            });

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
            let excluded = self
                .exclude_statuses
                .iter()
                .any(|e| e.eq_ignore_ascii_case(status));
            let preset_wants = preset
                .include
                .statuses
                .iter()
                .any(|s| s.eq_ignore_ascii_case(status));
            if excluded && !preset_wants {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
impl ProjectConfig {
    /// A travel-smart fixture used by the filtering/ordering tests. (Real configs
    /// come from disk / the wizard since M4.)
    pub fn travel_smart() -> Self {
        let preset = |name: &str, f: Filter| Preset {
            name: name.to_string(),
            include: f,
        };
        ProjectConfig {
            name: "travel-smart".to_string(),
            repos: vec!["WhiteWolfStudio/travel-smart".to_string()],
            board: Board {
                owner: "WhiteWolfStudio".to_string(),
                number: 6,
            },
            target_session: None,
            claude_subdir: None,
            skill: Skill::default(),
            worktree: Worktree::default(),
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

        // A preset that names Done reveals it despite the exclude list.
        let done_preset = Preset {
            name: "done".into(),
            include: Filter {
                statuses: vec!["Done".into()],
                ..Default::default()
            },
        };
        assert!(cfg.keeps(&done_preset, &item("Done", &["Frontend"], &[])));

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
