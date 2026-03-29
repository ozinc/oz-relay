// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! Filesystem-backed task manager.
//!
//! State transitions = moving files between directories.
//! Every event is appended to ledger/events.jsonl.
//! Survives restarts. No database. Observable with `ls` and `tail -f`.
//!
//! Directory layout:
//!   tasks/submitted/   — intent received, queued
//!   tasks/working/     — claude is implementing
//!   tasks/completed/   — build succeeded
//!   tasks/failed/      — build failed
//!   tasks/canceled/    — developer canceled
//!   ledger/events.jsonl — append-only audit trail

use std::path::{Path, PathBuf};

use chrono::Utc;
use oz_relay_common::a2a::{Message, Task, TaskState};
use tokio::fs;
use uuid::Uuid;

pub struct TaskManager {
    tasks_dir: PathBuf,
    ledger_path: PathBuf,
}

impl TaskManager {
    /// Create a new filesystem-backed task manager.
    /// Creates all directories if they don't exist.
    pub async fn init(data_dir: &Path) -> Self {
        let tasks_dir = data_dir.join("tasks");
        let ledger_dir = data_dir.join("ledger");

        for subdir in &[
            "submitted",
            "working",
            "completed",
            "failed",
            "canceled",
        ] {
            let dir = tasks_dir.join(subdir);
            fs::create_dir_all(&dir)
                .await
                .unwrap_or_else(|e| panic!("cannot create {}: {}", dir.display(), e));
        }

        fs::create_dir_all(&ledger_dir)
            .await
            .unwrap_or_else(|e| panic!("cannot create {}: {}", ledger_dir.display(), e));

        // Create promotions directories
        let promotions_dir = data_dir.join("promotions");
        for subdir in &["pending", "approved", "merged", "rejected"] {
            let dir = promotions_dir.join(subdir);
            let _ = fs::create_dir_all(&dir).await;
        }

        // Create bugs directories
        let bugs_dir = data_dir.join("bugs");
        for subdir in &["incoming", "triaged", "resolved"] {
            let dir = bugs_dir.join(subdir);
            let _ = fs::create_dir_all(&dir).await;
        }

        let ledger_path = ledger_dir.join("events.jsonl");

        tracing::info!(
            tasks_dir = %tasks_dir.display(),
            ledger = %ledger_path.display(),
            "filesystem task manager initialized"
        );

        Self {
            tasks_dir,
            ledger_path,
        }
    }

    /// For tests — create an in-memory-like manager in a temp directory.
    #[cfg(test)]
    pub fn new() -> Self {
        let dir = std::env::temp_dir().join(format!("oz-relay-test-{}", Uuid::new_v4()));
        std::fs::create_dir_all(dir.join("tasks/submitted")).unwrap();
        std::fs::create_dir_all(dir.join("tasks/working")).unwrap();
        std::fs::create_dir_all(dir.join("tasks/completed")).unwrap();
        std::fs::create_dir_all(dir.join("tasks/failed")).unwrap();
        std::fs::create_dir_all(dir.join("tasks/canceled")).unwrap();
        std::fs::create_dir_all(dir.join("ledger")).unwrap();
        Self {
            tasks_dir: dir.join("tasks"),
            ledger_path: dir.join("ledger/events.jsonl"),
        }
    }

    fn state_dir(&self, state: TaskState) -> PathBuf {
        let name = match state {
            TaskState::Submitted => "submitted",
            TaskState::Working => "working",
            TaskState::Completed => "completed",
            TaskState::Failed => "failed",
            TaskState::Canceled => "canceled",
            TaskState::Rejected => "failed", // treated as failed
            TaskState::InputRequired => "working", // stays in working
        };
        self.tasks_dir.join(name)
    }

    fn task_filename(id: Uuid) -> String {
        format!("{}.json", id)
    }

    /// Append an event to the ledger.
    async fn log_event(&self, event: serde_json::Value) {
        let line = format!("{}\n", event);
        if let Err(e) = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.ledger_path)
            .await
        {
            tracing::error!(error = %e, "failed to open ledger");
            return;
        }
        // Use tokio write
        use tokio::io::AsyncWriteExt;
        match fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.ledger_path)
            .await
        {
            Ok(mut f) => {
                if let Err(e) = f.write_all(line.as_bytes()).await {
                    tracing::error!(error = %e, "failed to write ledger event");
                }
            }
            Err(e) => tracing::error!(error = %e, "failed to open ledger"),
        }
    }

    /// Find which directory a task lives in. Returns the full path.
    async fn find_task_path(&self, id: Uuid) -> Option<PathBuf> {
        let filename = Self::task_filename(id);
        for subdir in &["submitted", "working", "completed", "failed", "canceled"] {
            let path = self.tasks_dir.join(subdir).join(&filename);
            if fs::try_exists(&path).await.unwrap_or(false) {
                return Some(path);
            }
        }
        None
    }

    /// Read a task from disk.
    async fn read_task(&self, path: &Path) -> Option<Task> {
        match fs::read_to_string(path).await {
            Ok(content) => match serde_json::from_str(&content) {
                Ok(task) => Some(task),
                Err(e) => {
                    tracing::error!(path = %path.display(), error = %e, "corrupt task file");
                    None
                }
            },
            Err(_) => None,
        }
    }

    /// Write a task to disk.
    async fn write_task(&self, path: &Path, task: &Task) {
        let json = serde_json::to_string_pretty(task).unwrap();
        if let Err(e) = fs::write(path, json).await {
            tracing::error!(path = %path.display(), error = %e, "failed to write task");
        }
    }

    /// Create a new task. Writes to tasks/submitted/{id}.json.
    pub async fn create_task(&self, owner: &str, message: Message) -> Task {
        let task = Task::new(owner, message);
        let path = self.state_dir(TaskState::Submitted).join(Self::task_filename(task.id));

        self.write_task(&path, &task).await;

        self.log_event(serde_json::json!({
            "ts": Utc::now().to_rfc3339(),
            "event": "task.created",
            "task_id": task.id.to_string(),
            "owner": owner,
            "state": "submitted",
        }))
        .await;

        task
    }

    /// Get a task by ID (internal — no tenant check).
    pub async fn get_task(&self, id: Uuid) -> Option<Task> {
        let path = self.find_task_path(id).await?;
        self.read_task(&path).await
    }

    /// Get a task by ID, only if owned by the given developer.
    pub async fn get_task_for_owner(&self, id: Uuid, owner: &str) -> Option<Task> {
        let task = self.get_task(id).await?;
        if task.owner == owner {
            Some(task)
        } else {
            None
        }
    }

    /// Transition a task to a new state. Moves the file between directories.
    pub async fn transition_task(&self, id: Uuid, new_state: TaskState) -> Result<Task, String> {
        let old_path = self.find_task_path(id).await.ok_or("task not found")?;
        let mut task = self
            .read_task(&old_path)
            .await
            .ok_or("corrupt task file")?;

        task.transition(new_state).map_err(|e| e.to_string())?;

        let new_path = self.state_dir(new_state).join(Self::task_filename(id));

        // Write to new location first, then remove old (crash-safe order)
        self.write_task(&new_path, &task).await;
        let _ = fs::remove_file(&old_path).await;

        self.log_event(serde_json::json!({
            "ts": Utc::now().to_rfc3339(),
            "event": format!("task.{}", state_name(new_state)),
            "task_id": id.to_string(),
        }))
        .await;

        Ok(task)
    }

    /// Transition a task, only if owned by the given developer.
    pub async fn transition_task_for_owner(
        &self,
        id: Uuid,
        owner: &str,
        new_state: TaskState,
    ) -> Result<Task, String> {
        let old_path = self.find_task_path(id).await.ok_or("task not found")?;
        let task = self
            .read_task(&old_path)
            .await
            .ok_or("corrupt task file")?;

        if task.owner != owner {
            return Err("task not found".into());
        }

        // Use the non-owner version now that we've verified ownership
        self.transition_task(id, new_state).await
    }

    /// Add a message to a task (in-place update).
    pub async fn add_message(&self, id: Uuid, message: Message) -> Result<Task, String> {
        let path = self.find_task_path(id).await.ok_or("task not found")?;
        let mut task = self
            .read_task(&path)
            .await
            .ok_or("corrupt task file")?;

        let role = format!("{:?}", message.role);
        task.messages.push(message);
        task.updated_at = Utc::now();

        self.write_task(&path, &task).await;

        self.log_event(serde_json::json!({
            "ts": Utc::now().to_rfc3339(),
            "event": "task.message_added",
            "task_id": id.to_string(),
            "role": role,
            "message_count": task.messages.len(),
        }))
        .await;

        Ok(task)
    }

    /// List all tasks for a given developer (scans all directories).
    pub async fn list_tasks(&self, developer_id: &str) -> Vec<Task> {
        let mut tasks = Vec::new();
        for subdir in &["submitted", "working", "completed", "failed", "canceled"] {
            let dir = self.tasks_dir.join(subdir);
            let mut entries = match fs::read_dir(&dir).await {
                Ok(e) => e,
                Err(_) => continue,
            };
            while let Ok(Some(entry)) = entries.next_entry().await {
                if let Some(task) = self.read_task(&entry.path()).await {
                    if task.owner == developer_id {
                        tasks.push(task);
                    }
                }
            }
        }
        tasks
    }
}

fn state_name(state: TaskState) -> &'static str {
    match state {
        TaskState::Submitted => "submitted",
        TaskState::Working => "working",
        TaskState::Completed => "completed",
        TaskState::Failed => "failed",
        TaskState::Canceled => "canceled",
        TaskState::Rejected => "rejected",
        TaskState::InputRequired => "input_required",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oz_relay_common::a2a::{MessageRole, Part};

    fn test_message() -> Message {
        Message {
            role: MessageRole::User,
            parts: vec![Part::Text {
                text: "test intent".into(),
            }],
        }
    }

    #[tokio::test]
    async fn create_and_get() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message()).await;
        assert_eq!(task.state, TaskState::Submitted);
        assert_eq!(task.owner, "dev_alice");

        let fetched = mgr.get_task_for_owner(task.id, "dev_alice").await.unwrap();
        assert_eq!(fetched.id, task.id);
        assert_eq!(fetched.messages.len(), 1);
    }

    #[tokio::test]
    async fn tenant_isolation() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message()).await;

        assert!(mgr.get_task_for_owner(task.id, "dev_alice").await.is_some());
        assert!(mgr.get_task_for_owner(task.id, "dev_bob").await.is_none());

        assert!(mgr
            .transition_task_for_owner(task.id, "dev_bob", TaskState::Canceled)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn transition_lifecycle() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message()).await;

        let t = mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Working)
            .await
            .unwrap();
        assert_eq!(t.state, TaskState::Working);

        let t = mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Completed)
            .await
            .unwrap();
        assert_eq!(t.state, TaskState::Completed);

        assert!(mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Working)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn task_survives_reread() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message()).await;
        mgr.transition_task(task.id, TaskState::Working).await.unwrap();

        // Read again — should find it in working/
        let fetched = mgr.get_task(task.id).await.unwrap();
        assert_eq!(fetched.state, TaskState::Working);
    }

    #[tokio::test]
    async fn invalid_transition_rejected() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message()).await;
        assert!(mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Completed)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn not_found() {
        let mgr = TaskManager::new();
        assert!(mgr.get_task_for_owner(Uuid::new_v4(), "dev_alice").await.is_none());
        assert!(mgr
            .transition_task_for_owner(Uuid::new_v4(), "dev_alice", TaskState::Working)
            .await
            .is_err());
    }

    #[tokio::test]
    async fn add_message_to_task() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message()).await;
        let agent_msg = Message {
            role: MessageRole::Agent,
            parts: vec![Part::Text {
                text: "implemented the change".into(),
            }],
        };
        let updated = mgr.add_message(task.id, agent_msg).await.unwrap();
        assert_eq!(updated.messages.len(), 2);
    }

    #[tokio::test]
    async fn list_filters_by_owner() {
        let mgr = TaskManager::new();
        mgr.create_task("dev_alice", test_message()).await;
        mgr.create_task("dev_alice", test_message()).await;
        mgr.create_task("dev_bob", test_message()).await;

        assert_eq!(mgr.list_tasks("dev_alice").await.len(), 2);
        assert_eq!(mgr.list_tasks("dev_bob").await.len(), 1);
        assert_eq!(mgr.list_tasks("dev_charlie").await.len(), 0);
    }

    #[tokio::test]
    async fn ledger_records_events() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message()).await;
        mgr.transition_task(task.id, TaskState::Working).await.unwrap();

        let ledger = std::fs::read_to_string(&mgr.ledger_path).unwrap();
        let lines: Vec<&str> = ledger.trim().lines().collect();
        assert!(lines.len() >= 2);
        assert!(lines[0].contains("task.created"));
        assert!(lines[1].contains("task.working"));
    }
}
