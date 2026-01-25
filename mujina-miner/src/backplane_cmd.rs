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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reinitialize_result_success_with_voltage() {
        let result = ReinitializeResult::success(
            "Board reinitialized".to_string(),
            Some(1.15),
        );

        assert!(result.success);
        assert_eq!(result.message, "Board reinitialized");
        assert!(result.error.is_none());
        assert_eq!(result.current_voltage, Some(1.15));
    }

    #[test]
    fn test_reinitialize_result_success_without_voltage() {
        let result = ReinitializeResult::success(
            "Board reinitialized".to_string(),
            None,
        );

        assert!(result.success);
        assert_eq!(result.message, "Board reinitialized");
        assert!(result.error.is_none());
        assert!(result.current_voltage.is_none());
    }

    #[test]
    fn test_reinitialize_result_failure() {
        let result = ReinitializeResult::failure(
            "Board not found".to_string(),
            "No board with serial 'ABC123' exists".to_string(),
        );

        assert!(!result.success);
        assert_eq!(result.message, "Board not found");
        assert_eq!(
            result.error,
            Some("No board with serial 'ABC123' exists".to_string())
        );
        assert!(result.current_voltage.is_none());
    }

    #[test]
    fn test_reinitialize_result_clone() {
        let original = ReinitializeResult::success(
            "Test message".to_string(),
            Some(1.2),
        );
        let cloned = original.clone();

        assert_eq!(original.success, cloned.success);
        assert_eq!(original.message, cloned.message);
        assert_eq!(original.error, cloned.error);
        assert_eq!(original.current_voltage, cloned.current_voltage);
    }

    #[test]
    fn test_reinitialize_result_debug() {
        let result = ReinitializeResult::failure(
            "Error".to_string(),
            "Details".to_string(),
        );

        // Verify Debug trait is implemented and produces output
        let debug_output = format!("{:?}", result);
        assert!(debug_output.contains("ReinitializeResult"));
        assert!(debug_output.contains("success: false"));
    }
}
