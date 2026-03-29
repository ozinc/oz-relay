// Copyright (c) 2026 OZ Global Inc.
// Licensed under the Apache License, Version 2.0.

//! OIP (Open Intent Protocol) types — A2A-compatible schema for agent-mediated
//! contribution to closed-source software.
//!
//! Built on Google's Agent2Agent (A2A) protocol primitives:
//! - AgentCard: capability advertisement
//! - Task: lifecycle-managed unit of work
//! - Message: communication turn (user intent or agent response)
//! - Artifact: output binary with signed manifest

pub mod a2a;
pub mod intent;
pub mod report;
pub mod validation;

pub use a2a::*;
pub use intent::*;
