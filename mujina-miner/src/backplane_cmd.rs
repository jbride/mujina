//! Backplane command queue for external control interfaces.
//!
//! This module provides a command-based interface for external systems (REST API,
//! MQTT, CLI, etc.) to interact with the backplane without tight coupling.

use tokio::sync::oneshot;

/// Commands that can be sent to the backplane for execution.
#[derive(Debug)]
pub enum BackplaneCommand {
    /// Request to reinitialize a specific board by serial number.
    ReinitializeBoard {
        /// Serial number of the board to reinitialize
        serial: String,
        /// Response channel to send the result back
        response_tx: oneshot::Sender<ReinitializeResult>,
    },
}

/// Result of a board reinitialization attempt.
#[derive(Debug, Clone)]
pub struct ReinitializeResult {
    /// Whether the reinitialization was successful
    pub success: bool,
    /// Descriptive message about the outcome
    pub message: String,
    /// Error details if the operation failed
    pub error: Option<String>,
    /// Current voltage after reinitialization if available
    pub current_voltage: Option<f32>,
}

impl ReinitializeResult {
    /// Create a success result.
    pub fn success(message: String, current_voltage: Option<f32>) -> Self {
        Self {
            success: true,
            message,
            error: None,
            current_voltage,
        }
    }

    /// Create a failure result.
    pub fn failure(message: String, error: String) -> Self {
        Self {
            success: false,
            message,
            error: Some(error),
            current_voltage: None,
        }
    }
}
