use std::path::Path;

use super::{format_priority, format_status};
use crate::db::Database;

/// Workspace details for display, with its project name and on-disk state.
struct WorkspaceInfo {
    id: i64,
    name: String,
    path: String,
    project_id: i64,
    project_name: String,
    missing: bool,
}

fn workspace_info(
    db: &Database,
    workspace_id: Option<i64>,
) -> Result<Option<WorkspaceInfo>, String> {
    let Some(id) = workspace_id else {
        return Ok(None);
    };
    let Some(w) = db.get_workspace(id)? else {
        return Ok(None);
    };
    let project_name = db
        .get_project(w.project_id)?
        .map(|p| p.name)
        .unwrap_or_default();
    let missing = !Path::new(&w.path).exists();
    Ok(Some(WorkspaceInfo {
        id: w.id,
        name: w.name,
        path: w.path,
        project_id: w.project_id,
        project_name,
        missing,
    }))
}

fn workspace_json(db: &Database, workspace_id: Option<i64>) -> Result<serde_json::Value, String> {
    Ok(match workspace_info(db, workspace_id)? {
        Some(w) => serde_json::json!({
            "id": w.id,
            "name": w.name,
            "path": w.path,
            "project_id": w.project_id,
            "project_name": w.project_name,
            "missing": w.missing,
        }),
        None => serde_json::Value::Null,
    })
}

/// Show a task with blockers, dependents, subtasks, comments and its workspace.
pub fn run(db_path: &Path, id: &str, json: bool) -> Result<(), String> {
    let db = Database::open(db_path)?;
    let task = db
        .get_task(id)?
        .ok_or_else(|| format!("task not found: {id}"))?;

    if json {
        let mut value = serde_json::to_value(&task).map_err(|e| format!("json error: {e}"))?;
        // Add comments, blockers, children, and dependents to JSON output
        let comments = db.get_comments(id)?;
        let blocker_deps = db.get_blockers(id)?;
        let blocker_tasks: Vec<_> = blocker_deps
            .iter()
            .filter_map(|d| db.get_task(&d.parent_id).ok().flatten())
            .collect();
        let children = db.get_children(id)?;
        let dependents = db.get_dependents(id)?;
        let workspace = workspace_json(&db, task.workspace_id)?;
        // Additive: ids of user comments still awaiting an agent reply.
        let pending_ids: Vec<i64> = db
            .task_pending_user_comments(id)?
            .iter()
            .map(|c| c.id)
            .collect();
        if let Some(obj) = value.as_object_mut() {
            obj.insert(
                "pending_user_comments".to_string(),
                serde_json::to_value(&pending_ids).unwrap_or_default(),
            );
            obj.insert("workspace".to_string(), workspace);
            obj.insert(
                "comments".to_string(),
                serde_json::to_value(&comments).unwrap_or_default(),
            );
            obj.insert(
                "blockers".to_string(),
                serde_json::to_value(&blocker_tasks).unwrap_or_default(),
            );
            obj.insert(
                "children".to_string(),
                serde_json::to_value(&children).unwrap_or_default(),
            );
            obj.insert(
                "dependents".to_string(),
                serde_json::to_value(&dependents).unwrap_or_default(),
            );
        }
        let j = serde_json::to_string_pretty(&value).map_err(|e| format!("json error: {e}"))?;
        println!("{j}");
        return Ok(());
    }

    // Human-readable output
    println!("ID:          {}", task.id);
    println!("Title:       {}", task.title);
    println!("Status:      {}", format_status(&task.status));
    if let Some(ref reason) = task.close_reason {
        println!("Reason:      {reason}");
    }
    println!("Priority:    {}", format_priority(task.priority));
    if let Some(ref desc) = task.description {
        println!("Description: {desc}");
    }
    if let Some(ref notes) = task.notes {
        println!("Notes:       {notes}");
    }
    if let Some(ref assignee) = task.assignee {
        println!("Assignee:    {assignee}");
    }
    if let Some(ref parent) = task.parent_id {
        println!("Parent:      {parent}");
    }
    if !task.tags.is_empty() {
        println!("Tags:        {}", task.tags.join(", "));
    }
    match workspace_info(&db, task.workspace_id)? {
        Some(w) => println!(
            "Workspace:   {} ({}) project: {}{}",
            w.name,
            w.path,
            w.project_name,
            if w.missing { " (missing)" } else { "" }
        ),
        None => println!("Workspace:   none"),
    }
    println!("Created:     {}", task.created_at.format("%Y-%m-%d %H:%M"));
    println!("Updated:     {}", task.updated_at.format("%Y-%m-%d %H:%M"));

    // Show blockers
    let blockers = db.get_blockers(id)?;
    if !blockers.is_empty() {
        println!("\nBlockers:");
        for dep in &blockers {
            if let Some(blocker_task) = db.get_task(&dep.parent_id)? {
                println!(
                    "  - {} [{}] {}",
                    dep.parent_id,
                    format_status(&blocker_task.status),
                    blocker_task.title
                );
            }
        }
    }

    // Show dependents
    let dependents = db.get_dependents(id)?;
    if !dependents.is_empty() {
        println!("\nDependents:");
        for dep in &dependents {
            println!(
                "  - {} [{}] {}",
                dep.id,
                format_status(&dep.status),
                dep.title
            );
        }
    }

    // Show children
    let children = db.get_children(id)?;
    if !children.is_empty() {
        println!("\nSubtasks:");
        for child in &children {
            println!(
                "  - {} [{}] {} {}",
                child.id,
                format_status(&child.status),
                format_priority(child.priority),
                child.title
            );
        }
    }

    // Show comments
    let comments = db.get_comments(id)?;
    if !comments.is_empty() {
        let pending: Vec<i64> = db
            .task_pending_user_comments(id)?
            .iter()
            .map(|c| c.id)
            .collect();
        println!("\nComments:");
        for c in &comments {
            let author = match c.author.as_deref() {
                Some(a) => format!(" [{a}]"),
                None => String::new(),
            };
            let flag = if pending.contains(&c.id) {
                "  ⚠ awaiting reply"
            } else {
                ""
            };
            println!(
                "  [{}]{} {}{}",
                c.created_at.format("%Y-%m-%d %H:%M"),
                author,
                c.body,
                flag
            );
        }
    }

    Ok(())
}
