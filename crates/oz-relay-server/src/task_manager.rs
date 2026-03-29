// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! In-memory task store with A2A lifecycle management.
//!
//! Security fix #5: Tasks are owned by the developer who created them.
//! All access methods enforce tenant isolation via the `owner` field.

use std::collections::HashMap;
use std::sync::Mutex;

use oz_relay_common::a2a::{Message, Task, TaskState};
use uuid::Uuid;

pub struct TaskManager {
    tasks: Mutex<HashMap<Uuid, Task>>,
}

impl TaskManager {
    pub fn new() -> Self {
        Self {
            tasks: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new task from a user message (intent), owned by the given developer.
    pub fn create_task(&self, owner: &str, message: Message) -> Task {
        let task = Task::new(owner, message);
        let id = task.id;
        self.tasks.lock().unwrap().insert(id, task.clone());
        task
    }

    /// Get a task by ID, but only if owned by the given developer.
    pub fn get_task_for_owner(&self, id: Uuid, owner: &str) -> Option<Task> {
        self.tasks
            .lock()
            .unwrap()
            .get(&id)
            .filter(|t| t.owner == owner)
            .cloned()
    }

    /// Get a task by ID (internal use only — no tenant check).
    pub fn get_task(&self, id: Uuid) -> Option<Task> {
        self.tasks.lock().unwrap().get(&id).cloned()
    }

    /// Transition a task to a new state, only if owned by the given developer.
    pub fn transition_task_for_owner(
        &self,
        id: Uuid,
        owner: &str,
        state: TaskState,
    ) -> Result<Task, String> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks.get_mut(&id).ok_or("task not found")?;
        if task.owner != owner {
            return Err("task not found".into());
        }
        task.transition(state).map_err(|e| e.to_string())?;
        Ok(task.clone())
    }

    /// Transition a task (internal use — no tenant check).
    pub fn transition_task(&self, id: Uuid, state: TaskState) -> Result<Task, String> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks.get_mut(&id).ok_or("task not found")?;
        task.transition(state).map_err(|e| e.to_string())?;
        Ok(task.clone())
    }

    /// Add an agent message to a task.
    pub fn add_message(&self, id: Uuid, message: Message) -> Result<Task, String> {
        let mut tasks = self.tasks.lock().unwrap();
        let task = tasks.get_mut(&id).ok_or("task not found")?;
        task.messages.push(message);
        task.updated_at = chrono::Utc::now();
        Ok(task.clone())
    }

    /// List all tasks for a given developer.
    pub fn list_tasks(&self, developer_id: &str) -> Vec<Task> {
        self.tasks
            .lock()
            .unwrap()
            .values()
            .filter(|t| t.owner == developer_id)
            .cloned()
            .collect()
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

    #[test]
    fn create_and_get() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message());
        assert_eq!(task.state, TaskState::Submitted);
        assert_eq!(task.owner, "dev_alice");

        let fetched = mgr.get_task_for_owner(task.id, "dev_alice").unwrap();
        assert_eq!(fetched.id, task.id);
        assert_eq!(fetched.messages.len(), 1);
    }

    #[test]
    fn tenant_isolation() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message());

        // Owner can see it
        assert!(mgr.get_task_for_owner(task.id, "dev_alice").is_some());

        // Other developer cannot
        assert!(mgr.get_task_for_owner(task.id, "dev_bob").is_none());

        // Other developer cannot cancel it
        assert!(mgr
            .transition_task_for_owner(task.id, "dev_bob", TaskState::Canceled)
            .is_err());
    }

    #[test]
    fn transition_lifecycle() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message());

        let t = mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Working)
            .unwrap();
        assert_eq!(t.state, TaskState::Working);

        let t = mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Completed)
            .unwrap();
        assert_eq!(t.state, TaskState::Completed);

        assert!(mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Working)
            .is_err());
    }

    #[test]
    fn invalid_transition_rejected() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message());
        assert!(mgr
            .transition_task_for_owner(task.id, "dev_alice", TaskState::Completed)
            .is_err());
    }

    #[test]
    fn not_found() {
        let mgr = TaskManager::new();
        assert!(mgr.get_task_for_owner(Uuid::new_v4(), "dev_alice").is_none());
        assert!(mgr
            .transition_task_for_owner(Uuid::new_v4(), "dev_alice", TaskState::Working)
            .is_err());
    }

    #[test]
    fn add_message_to_task() {
        let mgr = TaskManager::new();
        let task = mgr.create_task("dev_alice", test_message());
        let agent_msg = Message {
            role: MessageRole::Agent,
            parts: vec![Part::Text {
                text: "implemented the change".into(),
            }],
        };
        let updated = mgr.add_message(task.id, agent_msg).unwrap();
        assert_eq!(updated.messages.len(), 2);
    }

    #[test]
    fn list_filters_by_owner() {
        let mgr = TaskManager::new();
        mgr.create_task("dev_alice", test_message());
        mgr.create_task("dev_alice", test_message());
        mgr.create_task("dev_bob", test_message());

        assert_eq!(mgr.list_tasks("dev_alice").len(), 2);
        assert_eq!(mgr.list_tasks("dev_bob").len(), 1);
        assert_eq!(mgr.list_tasks("dev_charlie").len(), 0);
    }
}
