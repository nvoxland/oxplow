//! The GraphQL documents this provider sends, and a Linear issue as a
//! work-item record.

use oxplow_domain::work_items::WorkItemRecord;
use oxplow_provider_protocol::ProtocolError;
use serde_json::{json, Value};

use crate::graphql::Operation;
use crate::states::{canonical_of_type, Team};

/// The fields every issue is read with — what a record is built from.
macro_rules! issue_fields {
    () => {
        "id identifier title description url priority updatedAt trashed \
         state { id name type } parent { identifier } team { key }"
    };
}

pub const TEAM: Operation = Operation {
    name: "Team",
    document: "query Team($key: String!) { teams(filter: { key: { eq: $key } }) { nodes { id key \
               states { nodes { id name type position } } } } }",
};

pub const PROJECT: Operation = Operation {
    name: "Project",
    document:
        "query Project($team: ID!, $name: String!) { projects(filter: { name: { eq: $name }, \
               accessibleTeams: { id: { eq: $team } } }) { nodes { id name } } }",
};

pub const ISSUE: Operation = Operation {
    name: "Issue",
    document: concat!(
        "query Issue($id: String!) { issue(id: $id) { ",
        issue_fields!(),
        " } }"
    ),
};

pub const ISSUE_CREATE: Operation = Operation {
    name: "IssueCreate",
    document: concat!(
        "mutation IssueCreate($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { ",
        issue_fields!(),
        " } } }"
    ),
};

pub const ISSUE_UPDATE: Operation = Operation {
    name: "IssueUpdate",
    document: concat!(
        "mutation IssueUpdate($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success issue { ",
        issue_fields!(),
        " } } }"
    ),
};

pub const RELATION_CREATE: Operation = Operation {
    name: "IssueRelationCreate",
    document: concat!(
        "mutation IssueRelationCreate($input: IssueRelationCreateInput!) { issueRelationCreate(input: $input) { success issueRelation { issue { ",
        issue_fields!(),
        " } } } }"
    ),
};

pub const COMMENT_CREATE: Operation = Operation {
    name: "CommentCreate",
    document: concat!(
        "mutation CommentCreate($input: CommentCreateInput!) { commentCreate(input: $input) { success comment { issue { ",
        issue_fields!(),
        " } } } }"
    ),
};

pub const ISSUE_DELETE: Operation = Operation {
    name: "IssueDelete",
    document: "mutation IssueDelete($id: String!) { issueDelete(id: $id) { success } }",
};

pub const ISSUES: Operation = Operation {
    name: "Issues",
    document: concat!(
        "query Issues($filter: IssueFilter!, $first: Int!, $after: String) { issues(filter: $filter, first: $first, after: $after, includeArchived: true, orderBy: updatedAt) { nodes { ",
        issue_fields!(),
        " } pageInfo { hasNextPage endCursor } } }"
    ),
};

/// The ref prefix of provider `id`'s items: `work_item:<id>:`.
pub fn ref_prefix(id: &str) -> String {
    format!("work_item:{id}:")
}

/// The identifier (`ENG-12`) `item_ref` names under `prefix`; `field` is
/// where it came from, for the error.
pub fn identifier_of<'a>(
    prefix: &str,
    item_ref: &'a str,
    field: &str,
) -> Result<&'a str, ProtocolError> {
    item_ref
        .strip_prefix(prefix)
        .filter(|id| {
            id.split_once('-').is_some_and(|(team, n)| {
                !team.is_empty()
                    && team.chars().all(|c| c.is_ascii_alphanumeric())
                    && !n.is_empty()
                    && n.chars().all(|c| c.is_ascii_digit())
            })
        })
        .ok_or_else(|| ProtocolError::InvalidInput {
            field: field.into(),
            message: format!("`{item_ref}` isn't one of these Linear issues ({prefix}<TEAM>-<n>)"),
        })
}

/// An issue node as a work-item record: the state mapped through `team`,
/// the uuid, url and priority under `native`, a trashed issue deleted.
pub fn record(prefix: &str, team: &Team, node: &Value) -> Result<WorkItemRecord, ProtocolError> {
    let bad = |what: &str| ProtocolError::Internal(format!("Linear sent an issue without {what}"));
    let identifier = node["identifier"]
        .as_str()
        .ok_or_else(|| bad("an identifier"))?;
    let state_name = node["state"]["name"]
        .as_str()
        .ok_or_else(|| bad("a state"))?;
    let state_type = node["state"]["type"].as_str().unwrap_or_default();
    let state = team
        .canonical(state_name, state_type)
        .or_else(|| canonical_of_type(state_type))
        .ok_or_else(|| {
            ProtocolError::Internal(format!(
                "{identifier}'s state `{state_name}` has type `{state_type}`, which maps to no state"
            ))
        })?;
    Ok(WorkItemRecord {
        item_ref: format!("{prefix}{identifier}"),
        title: node["title"].as_str().unwrap_or_default().to_string(),
        body: node["description"].as_str().unwrap_or_default().to_string(),
        state,
        native_state: state_name.to_string(),
        native: json!({ "id": node["id"], "url": node["url"], "priority": node["priority"] }),
        parent_ref: node["parent"]["identifier"]
            .as_str()
            .map(|p| format!("{prefix}{p}")),
        deleted: node["trashed"].as_bool().unwrap_or(false),
    })
}
