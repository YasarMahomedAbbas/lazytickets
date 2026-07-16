//! Global config: the on-disk `~/.config/lazytickets/config.toml` holding every
//! project, plus path-keyed overrides. `resolver` maps the current repo to one of
//! these projects; `wizard` writes a new one on first run.

pub mod resolver;
pub mod schema;
pub mod wizard;

use anyhow::{Context, Result};
use directories::ProjectDirs;
use schema::ProjectConfig;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

/// The whole config file. Both fields default so a partial/empty file still loads.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub projects: Vec<ProjectConfig>,
    /// Absolute repo-root path → project name. Wins over git-remote resolution.
    #[serde(default)]
    pub overrides: HashMap<String, String>,
}

impl Config {
    /// `~/.config/lazytickets/config.toml` (XDG on Linux).
    pub fn path() -> Result<PathBuf> {
        let dirs = ProjectDirs::from("", "", "lazytickets")
            .context("could not resolve a config directory")?;
        Ok(dirs.config_dir().join("config.toml"))
    }

    /// Load the config, or an empty default if the file doesn't exist yet.
    pub fn load() -> Result<Config> {
        let path = Self::path()?;
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text)
                .with_context(|| format!("parsing config at {}", path.display())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
            Err(e) => Err(e).with_context(|| format!("reading config at {}", path.display())),
        }
    }

    /// Write the config back to disk, creating the directory if needed.
    pub fn save(&self) -> Result<()> {
        let path = Self::path()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating config dir {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self).context("serialising config")?;
        std::fs::write(&path, text).with_context(|| format!("writing config to {}", path.display()))
    }

    pub fn find_project(&self, name: &str) -> Option<&ProjectConfig> {
        self.projects.iter().find(|p| p.name == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use schema::{Board, Filter, Preset, Skill};

    #[test]
    fn config_round_trips_through_toml() {
        let cfg = Config {
            projects: vec![ProjectConfig {
                name: "travel-smart".into(),
                repos: vec!["WhiteWolfStudio/travel-smart".into()],
                board: Board {
                    owner: "WhiteWolfStudio".into(),
                    number: 6,
                },
                target_session: None,
                claude_subdir: None,
                skill: Skill {
                    start: Some("frontend-start-ticket".into()),
                    create: None,
                },
                status_order: vec!["Refine".into(), "Done".into()],
                exclude_statuses: vec!["Done".into()],
                presets: vec![
                    Preset {
                        name: "all".into(),
                        include: Filter::default(),
                    },
                    Preset {
                        name: "mine".into(),
                        include: Filter {
                            assignees: vec!["YasarMahomedAbbas".into()],
                            ..Default::default()
                        },
                    },
                ],
            }],
            overrides: HashMap::from([("/tmp/some-repo".into(), "travel-smart".into())]),
        };

        let text = toml::to_string_pretty(&cfg).unwrap();
        let back: Config = toml::from_str(&text).unwrap();

        let p = &back.projects[0];
        assert_eq!(p.name, "travel-smart");
        assert_eq!(p.board.owner, "WhiteWolfStudio");
        assert_eq!(p.board.number, 6);
        assert_eq!(p.repos, vec!["WhiteWolfStudio/travel-smart".to_string()]);
        assert_eq!(p.skill.start.as_deref(), Some("frontend-start-ticket"));
        assert_eq!(
            p.presets[1].include.assignees,
            vec!["YasarMahomedAbbas".to_string()]
        );
        assert_eq!(
            back.overrides.get("/tmp/some-repo").map(String::as_str),
            Some("travel-smart")
        );
    }
}
