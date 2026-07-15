//! Projects v2 status writes. Resolving the ids (project/field/options) needs
//! only `read:project`; the actual move (`gh project item-edit`) needs the
//! `project` scope (see `gh::scope`).

use anyhow::{Context, Result};
use serde::Deserialize;

/// A board's Status field: the ids `item-edit` needs plus option name↔id.
/// Fetched once per board and cached in `App`.
#[derive(Debug, Clone)]
pub struct StatusField {
    pub project_id: String,
    pub field_id: String,
    pub options: Vec<StatusOption>,
}

#[derive(Debug, Clone)]
pub struct StatusOption {
    pub id: String,
    pub name: String,
}

impl StatusField {
    /// The option id for a status name, case-insensitive.
    pub fn option_id(&self, name: &str) -> Option<&str> {
        self.options
            .iter()
            .find(|o| o.name.eq_ignore_ascii_case(name))
            .map(|o| o.id.as_str())
    }

    /// Option names in board order (drives the mover list).
    pub fn names(&self) -> Vec<String> {
        self.options.iter().map(|o| o.name.clone()).collect()
    }
}

#[derive(Deserialize)]
struct RawProjectView {
    id: String,
}

#[derive(Deserialize)]
struct RawFieldList {
    fields: Vec<RawField>,
}

#[derive(Deserialize)]
struct RawField {
    #[serde(default)]
    name: String,
    #[serde(default)]
    id: String,
    #[serde(default)]
    options: Vec<RawOption>,
}

#[derive(Deserialize)]
struct RawOption {
    id: String,
    name: String,
}

/// Fetch the board's project id + Status field id + options (read scope only).
pub async fn status_field(owner: &str, number: u32) -> Result<StatusField> {
    let num = number.to_string();

    let view = super::run(&["project", "view", &num, "--owner", owner, "--format", "json"]).await?;
    let project: RawProjectView = serde_json::from_slice(&view).context("parsing gh project view JSON")?;

    let fl = super::run(&["project", "field-list", &num, "--owner", owner, "--format", "json"]).await?;
    let raw: RawFieldList = serde_json::from_slice(&fl).context("parsing gh project field-list JSON")?;

    let status = raw
        .fields
        .into_iter()
        .find(|f| f.name.eq_ignore_ascii_case("Status"))
        .context("board has no single-select `Status` field")?;

    Ok(StatusField {
        project_id: project.id,
        field_id: status.id,
        options: status
            .options
            .into_iter()
            .map(|o| StatusOption { id: o.id, name: o.name })
            .collect(),
    })
}

/// Move item `item_id`'s Status to `option_id`. Needs the `project` scope; the
/// error from `gh` names the missing scope if it's absent.
pub async fn set_status(field: &StatusField, item_id: &str, option_id: &str) -> Result<()> {
    super::run(&[
        "project",
        "item-edit",
        "--project-id",
        &field.project_id,
        "--field-id",
        &field.field_id,
        "--id",
        item_id,
        "--single-select-option-id",
        option_id,
    ])
    .await?;
    Ok(())
}
