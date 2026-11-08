//! `lazytickets export --json`: the board as JSON for other front-ends (the herdr
//! tickets pane), with every preset already evaluated by the same filter logic the
//! TUI uses. Pure over `(config, items)` so it is unit-tested without the network;
//! `main::run_export` does the I/O.

use crate::app::{children_of, group_by_parent};
use crate::config::schema::ProjectConfig;
use crate::model::{Item, ParentRef};
use serde::Serialize;
use std::collections::HashMap;

/// The area a ticket falls under when it carries none of the project's `areas`.
pub const OTHER_AREA: &str = "Other";

#[derive(Debug, Serialize)]
pub struct Export {
    pub project: String,
    pub repos: Vec<String>,
    pub board: BoardRef,
    /// The start-work skill, e.g. `frontend-pipeline`. `None` when unlinked.
    pub skill: Option<String>,
    pub claude_subdir: Option<String>,
    pub status_order: Vec<String>,
    pub exclude_statuses: Vec<String>,
    /// The configured area labels, in order. Empty → the project isn't split by area.
    pub areas: Vec<String>,
    pub presets: Vec<PresetOut>,
    /// Every numbered card on the board (drafts have no number and are left out),
    /// sorted by status order.
    pub tickets: Vec<Ticket>,
}

#[derive(Debug, Serialize)]
pub struct BoardRef {
    pub owner: String,
    pub number: u32,
}

#[derive(Debug, Serialize)]
pub struct PresetOut {
    pub name: String,
    pub include_parents: bool,
    /// Ticket numbers this preset shows, in status order. For an
    /// `include_parents` preset this includes parents pulled in by a matching
    /// sub-issue, exactly as the TUI lists them.
    pub tickets: Vec<u64>,
}

#[derive(Debug, Serialize)]
pub struct Ticket {
    pub number: u64,
    pub title: String,
    pub status: Option<String>,
    /// The first of the project's `areas` this ticket carries, else `Other`.
    /// `None` when the project has no `areas`.
    pub area: Option<String>,
    pub labels: Vec<String>,
    pub assignees: Vec<String>,
    pub url: Option<String>,
    pub repository: Option<String>,
    /// Whether the status reads as finished (same rule as the TUI's badge).
    pub done: bool,
    /// The parent's number, when the parent is itself a card on this board.
    pub parent: Option<u64>,
    /// Numbers of this card's sub-issues that are on the board, in board order.
    pub sub_issues: Vec<u64>,
    /// How many of `sub_issues` are done.
    pub sub_done: usize,
}

/// The area for an item: the first configured area label it carries.
pub fn area_of(cfg: &ProjectConfig, item: &Item) -> Option<String> {
    if cfg.areas.is_empty() {
        return None;
    }
    let found = cfg
        .areas
        .iter()
        .find(|a| item.labels.iter().any(|l| l.eq_ignore_ascii_case(a)));
    Some(found.cloned().unwrap_or_else(|| OTHER_AREA.to_string()))
}

pub fn build(cfg: &ProjectConfig, items: &[Item]) -> Export {
    let done = |i: usize| crate::ui::list::is_done(items[i].status.as_deref());
    let rank = |i: usize| {
        let status = items[i].status.as_deref();
        (cfg.status_rank(status), status.unwrap_or("").to_ascii_lowercase())
    };
    let children = children_of(items);
    let by_key: HashMap<ParentRef, usize> = items
        .iter()
        .enumerate()
        .filter_map(|(i, it)| Some((it.key()?, i)))
        .collect();

    let presets = cfg
        .presets
        .iter()
        .map(|preset| {
            let mut matched: Vec<usize> = (0..items.len())
                .filter(|&i| cfg.keeps(preset, &items[i]))
                .collect();
            let shown = if preset.include_parents {
                group_by_parent(items, cfg, preset, &matched).0
            } else {
                matched.sort_by_key(|&i| rank(i));
                matched
            };
            let mut shown: Vec<usize> = shown;
            shown.sort_by_key(|&i| rank(i));
            PresetOut {
                name: preset.name.clone(),
                include_parents: preset.include_parents,
                tickets: shown.iter().filter_map(|&i| items[i].number).collect(),
            }
        })
        .collect();

    let mut order: Vec<usize> = (0..items.len()).filter(|&i| items[i].number.is_some()).collect();
    order.sort_by_key(|&i| rank(i));
    let tickets = order
        .into_iter()
        .map(|i| {
            let it = &items[i];
            let kids: Vec<usize> = children
                .get(&i)
                .map(|k| k.iter().copied().filter(|&c| items[c].number.is_some()).collect())
                .unwrap_or_default();
            Ticket {
                number: it.number.expect("filtered to numbered cards"),
                title: it.title.clone(),
                status: it.status.clone(),
                area: area_of(cfg, it),
                labels: it.labels.clone(),
                assignees: it.assignees.clone(),
                url: it.url.clone(),
                repository: it.repository.clone(),
                done: done(i),
                parent: it
                    .parent
                    .as_ref()
                    .and_then(|p| by_key.get(p))
                    .filter(|&&p| p != i)
                    .and_then(|&p| items[p].number),
                sub_done: kids.iter().filter(|&&c| done(c)).count(),
                sub_issues: kids.iter().filter_map(|&c| items[c].number).collect(),
            }
        })
        .collect();

    Export {
        project: cfg.name.clone(),
        repos: cfg.repos.clone(),
        board: BoardRef {
            owner: cfg.board.owner.clone(),
            number: cfg.board.number,
        },
        skill: cfg.skill.start.clone(),
        claude_subdir: cfg.claude_subdir.clone(),
        status_order: cfg.status_order.clone(),
        exclude_statuses: cfg.exclude_statuses.clone(),
        areas: cfg.areas.clone(),
        presets,
        tickets,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{Filter, Preset};

    fn card(n: u64, status: &str, labels: &[&str], parent: Option<u64>) -> Item {
        Item {
            id: format!("PVTI_{n}"),
            number: Some(n),
            title: format!("ticket {n}"),
            repository: Some("o/r".into()),
            status: Some(status.into()),
            labels: labels.iter().map(|s| s.to_string()).collect(),
            assignees: vec![],
            url: Some(format!("https://github.com/o/r/issues/{n}")),
            parent: parent.map(|p| ParentRef {
                repository: "o/r".into(),
                number: p,
            }),
        }
    }

    fn cfg() -> ProjectConfig {
        let mut c = ProjectConfig::travel_smart();
        c.areas = vec!["Frontend".into(), "Backend".into()];
        c.presets.push(Preset {
            name: "frontend+parents".into(),
            include: Filter {
                labels: vec!["Frontend".into()],
                ..Default::default()
            },
            include_parents: true,
        });
        c
    }

    /// Parent #480 (no area label, In progress) with a Frontend child in Ready To
    /// Implement and a Backend child in Create Contract — the split-parent case.
    fn board() -> Vec<Item> {
        vec![
            card(480, "In progress", &["documentation"], None),
            card(481, "Ready To Implement", &["Frontend"], Some(480)),
            card(482, "Create Contract", &["backend"], Some(480)),
            card(490, "Done", &["Frontend"], Some(480)),
            card(500, "Refine", &[], None),
        ]
    }

    fn ticket(e: &Export, n: u64) -> &Ticket {
        e.tickets.iter().find(|t| t.number == n).expect("ticket present")
    }

    #[test]
    fn areas_pick_the_first_configured_label_case_insensitively() {
        let e = build(&cfg(), &board());
        assert_eq!(ticket(&e, 481).area.as_deref(), Some("Frontend"));
        assert_eq!(ticket(&e, 482).area.as_deref(), Some("Backend"));
        assert_eq!(ticket(&e, 480).area.as_deref(), Some(OTHER_AREA));
    }

    #[test]
    fn no_areas_configured_means_no_area() {
        let e = build(&ProjectConfig::travel_smart(), &board());
        assert!(e.tickets.iter().all(|t| t.area.is_none()));
    }

    #[test]
    fn parent_links_and_done_tally() {
        let e = build(&cfg(), &board());
        let p = ticket(&e, 480);
        assert_eq!(p.sub_issues, vec![481, 482, 490]);
        assert_eq!(p.sub_done, 1);
        assert_eq!(ticket(&e, 482).parent, Some(480));
        assert_eq!(ticket(&e, 500).parent, None);
        assert!(ticket(&e, 490).done);
    }

    #[test]
    fn tickets_are_in_status_order() {
        let e = build(&cfg(), &board());
        let nums: Vec<u64> = e.tickets.iter().map(|t| t.number).collect();
        // Refine, Create Contract, Ready To Implement, In progress, …, Done
        assert_eq!(nums, vec![500, 482, 481, 480, 490]);
    }

    #[test]
    fn presets_use_the_tui_filters() {
        let e = build(&cfg(), &board());
        let preset = |name: &str| &e.presets.iter().find(|p| p.name == name).unwrap().tickets;
        // `all` hides Done via exclude_statuses.
        assert_eq!(preset("all"), &vec![500, 482, 481, 480]);
        // A label preset without roll-up shows only Frontend cards (Done hidden).
        assert_eq!(preset("frontend"), &vec![481]);
        // With include_parents, the documentation parent is pulled in by its child.
        assert_eq!(preset("frontend+parents"), &vec![481, 480]);
    }

    #[test]
    fn drafts_are_left_out() {
        let mut items = board();
        let mut draft = card(1, "Refine", &[], None);
        draft.number = None;
        items.push(draft);
        let e = build(&cfg(), &items);
        assert_eq!(e.tickets.len(), 5);
    }

    #[test]
    fn serialises_to_json() {
        let json = serde_json::to_value(build(&cfg(), &board())).unwrap();
        assert_eq!(json["project"], "travel-smart");
        assert_eq!(json["areas"][0], "Frontend");
        assert_eq!(json["tickets"][0]["number"], 500);
    }
}
