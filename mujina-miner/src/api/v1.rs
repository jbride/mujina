//! API version 1 endpoints.

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc, time::{Duration, Instant}};
use tokio::sync::{mpsc, Mutex, RwLock};
use tracing::{debug, error, warn};
use utoipa::{OpenApi, ToSchema};

use crate::{
    backplane_cmd::BackplaneCommand,
    board::BoardInfo,
    hw_trait::I2c,
    mgmt_protocol::bitaxe_raw::i2c::BitaxeRawI2c,
    peripheral::{emc2101::Emc2101, tps546::Tps546},
};

/// Voltage controller handle for a board.
pub type VoltageControllerHandle = Arc<Mutex<Tps546<BitaxeRawI2c>>>;

/// Fan controller handle for a board (provides temperature readings).
pub type FanControllerHandle = Arc<Mutex<Emc2101<BitaxeRawI2c>>>;

/// Board health state tracking for auto-recovery.
#[derive(Debug, Clone)]
pub struct BoardHealthState {
    /// Number of consecutive failures
    pub consecutive_failures: u32,
    /// Timestamp of last failure
    pub last_failure_time: Option<Instant>,
    /// Number of automatic retry attempts
    pub retry_count: u32,
    /// Timestamp of last retry attempt
    pub last_retry_time: Option<Instant>,
}

impl Default for BoardHealthState {
    fn default() -> Self {
        Self {
            consecutive_failures: 0,
            last_failure_time: None,
            retry_count: 0,
            last_retry_time: None,
        }
    }
}

/// Board recovery configuration from environment variables.
#[derive(Debug, Clone)]
pub struct BoardRecoveryConfig {
    /// Number of consecutive failures before marking board as needing recovery
    pub failure_threshold: u32,
    /// Maximum number of automatic retry attempts
    pub max_auto_retries: u32,
    /// Duration between automatic retry attempts
    pub retry_interval: Duration,
    /// Whether automatic recovery is enabled
    pub auto_recovery_enabled: bool,
}

impl Default for BoardRecoveryConfig {
    fn default() -> Self {
        Self {
            failure_threshold: std::env::var("MUJINA_BOARD_FAILURE_THRESHOLD")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3),
            max_auto_retries: std::env::var("MUJINA_BOARD_MAX_AUTO_RETRIES")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(3),
            retry_interval: Duration::from_secs(
                std::env::var("MUJINA_BOARD_RETRY_INTERVAL")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(30)
            ),
            auto_recovery_enabled: std::env::var("MUJINA_BOARD_AUTO_RECOVERY")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(false),
        }
    }
}

/// Board status for a board that failed initialization.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FailedBoardStatus {
    /// Board model/type if known
    #[schema(example = "Bitaxe Gamma")]
    pub model: Option<String>,
    /// Serial number if available
    #[schema(example = "ABC12345")]
    pub serial_number: Option<String>,
    /// Error message describing why initialization failed
    #[schema(example = "Failed to initialize I2C communication")]
    pub error: String,
}

/// Board status information for API responses.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BoardStatus {
    /// Board model/type (e.g., "Bitaxe Gamma")
    #[schema(example = "Bitaxe Gamma")]
    pub model: String,
    /// Board firmware version if available
    #[schema(example = "2.1.4")]
    pub firmware_version: Option<String>,
    /// Serial number
    #[schema(example = "ABC12345")]
    pub serial_number: String,
    /// Whether voltage control is available for this board
    #[schema(example = true)]
    pub voltage_control_available: bool,
    /// Current voltage in volts (if voltage control is available)
    #[schema(example = 1.2)]
    pub current_voltage_v: Option<f32>,
    /// Board temperature in degrees Celsius (from external sensor, e.g., EMC2101)
    #[schema(example = 45.5)]
    pub board_temp_c: Option<f32>,
    /// Fan speed in RPM (from fan controller, e.g., EMC2101)
    #[schema(example = 4500)]
    pub fan_speed_rpm: Option<u16>,
    /// Transient I2C communication error (e.g., voltage read timeout)
    #[schema(example = "I2C communication timeout")]
    pub transient_i2c_error: Option<String>,
    /// Whether the board needs reinitialization due to consecutive failures
    #[schema(example = false)]
    pub needs_reinit: bool,
    /// Number of consecutive failures
    #[schema(example = 0)]
    pub consecutive_failures: u32,
    /// Number of automatic retry attempts
    #[schema(example = 0)]
    pub retry_count: u32,
}

/// Complete board list response including both active and failed boards.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct BoardListResponse {
    /// Successfully initialized and active boards
    pub active_boards: Vec<BoardStatus>,
    /// Boards that failed to initialize
    pub failed_boards: Vec<FailedBoardStatus>,
}

/// Shared application state for API endpoints.
#[derive(Clone)]
pub struct AppState {
    /// Registry of voltage controllers by board serial number
    pub voltage_controllers: Arc<RwLock<HashMap<String, VoltageControllerHandle>>>,
    /// Registry of fan controllers by board serial number (for temperature readings)
    pub fan_controllers: Arc<RwLock<HashMap<String, FanControllerHandle>>>,
    /// Registry of board information by serial number
    pub boards: Arc<RwLock<HashMap<String, BoardInfo>>>,
    /// Registry of failed board initialization attempts
    pub failed_boards: Arc<RwLock<Vec<FailedBoardStatus>>>,
    /// Board health state tracking for auto-recovery
    pub board_health: Arc<RwLock<HashMap<String, BoardHealthState>>>,
    /// Recovery configuration
    pub recovery_config: BoardRecoveryConfig,
    /// Command channel to backplane for board operations (optional for testing)
    pub backplane_cmd_tx: Option<mpsc::Sender<BackplaneCommand>>,
    /// Board initialization timeout (read from MUJINA_BOARD_INIT_TIMEOUT_SECS at startup)
    pub board_init_timeout: Duration,
}

/// Default board initialization timeout in seconds.
pub const DEFAULT_BOARD_INIT_TIMEOUT_SECS: u64 = 10;

impl Default for AppState {
    fn default() -> Self {
        // Read timeout from environment or use default
        let board_init_timeout = std::env::var("MUJINA_BOARD_INIT_TIMEOUT_SECS")
            .ok()
            .and_then(|s| s.parse::<u64>().ok())
            .map(Duration::from_secs)
            .unwrap_or(Duration::from_secs(DEFAULT_BOARD_INIT_TIMEOUT_SECS));

        Self {
            voltage_controllers: Arc::new(RwLock::new(HashMap::new())),
            fan_controllers: Arc::new(RwLock::new(HashMap::new())),
            boards: Arc::new(RwLock::new(HashMap::new())),
            failed_boards: Arc::new(RwLock::new(Vec::new())),
            board_health: Arc::new(RwLock::new(HashMap::new())),
            recovery_config: BoardRecoveryConfig::default(),
            backplane_cmd_tx: None,
            board_init_timeout,
        }
    }
}

impl AppState {
    /// Create a new empty application state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a voltage controller for a board.
    pub async fn register_voltage_controller(
        &self,
        serial: String,
        controller: VoltageControllerHandle,
    ) {
        let mut controllers = self.voltage_controllers.write().await;
        controllers.insert(serial, controller);
    }

    /// Unregister a voltage controller for a board.
    pub async fn unregister_voltage_controller(&self, serial: &str) {
        let mut controllers = self.voltage_controllers.write().await;
        controllers.remove(serial);
    }

    /// Register a fan controller for a board (provides temperature readings).
    pub async fn register_fan_controller(
        &self,
        serial: String,
        controller: FanControllerHandle,
    ) {
        let mut controllers = self.fan_controllers.write().await;
        controllers.insert(serial, controller);
    }

    /// Unregister a fan controller for a board.
    pub async fn unregister_fan_controller(&self, serial: &str) {
        let mut controllers = self.fan_controllers.write().await;
        controllers.remove(serial);
    }

    /// Register board information.
    pub async fn register_board(&self, serial: String, info: BoardInfo) {
        debug!(
            serial = %serial,
            model = %info.model,
            "Registering board with API"
        );
        let mut boards = self.boards.write().await;
        boards.insert(serial, info);
    }

    /// Unregister board information.
    pub async fn unregister_board(&self, serial: &str) {
        let mut boards = self.boards.write().await;
        boards.remove(serial);
    }

    /// Register a failed board initialization attempt.
    pub async fn register_failed_board(&self, failed: FailedBoardStatus) {
        debug!(
            model = ?failed.model,
            serial = ?failed.serial_number,
            error = %failed.error,
            "Registering failed board"
        );
        let mut failed_boards = self.failed_boards.write().await;
        failed_boards.push(failed);
    }

    /// Remove a failed board from the failed boards list (e.g., during reinitialization).
    pub async fn remove_failed_board(&self, serial: &str) {
        let mut failed_boards = self.failed_boards.write().await;
        failed_boards.retain(|b| b.serial_number.as_deref() != Some(serial));
        debug!(serial = %serial, "Removed failed board from list");
    }

    /// Get a list of all registered boards with their status.
    pub async fn get_board_list(&self) -> BoardListResponse {
        let boards = self.boards.read().await;
        let voltage_controllers = self.voltage_controllers.read().await;
        let fan_controllers = self.fan_controllers.read().await;
        let failed = self.failed_boards.read().await;
        let mut board_health = self.board_health.write().await;

        debug!(
            board_count = boards.len(),
            voltage_controller_count = voltage_controllers.len(),
            fan_controller_count = fan_controllers.len(),
            failed_count = failed.len(),
            "Getting board list"
        );

        let mut active_boards = Vec::new();

        for (serial, info) in boards.iter() {
            let has_voltage_controller = voltage_controllers.contains_key(serial);

            // Read current voltage if controller is available and track any errors
            let mut board_error: Option<String> = None;
            let current_voltage = if has_voltage_controller {
                if let Some(controller) = voltage_controllers.get(serial) {
                    // Use timeout to prevent blocking on hung I2C operations
                    let voltage_future = async {
                        controller.lock().await.get_vout().await
                    };

                    match tokio::time::timeout(
                        tokio::time::Duration::from_millis(500),
                        voltage_future
                    ).await {
                        Ok(Ok(mv)) => {
                            let volts = mv as f32 / 1000.0;
                            debug!(
                                serial = %serial,
                                voltage = volts,
                                "Read current voltage for board"
                            );

                            // Reset failure counter on success
                            let health = board_health.entry(serial.clone()).or_default();
                            if health.consecutive_failures > 0 {
                                debug!(
                                    serial = %serial,
                                    previous_failures = health.consecutive_failures,
                                    "Board recovered, resetting failure counter"
                                );
                                health.consecutive_failures = 0;
                                health.last_failure_time = None;
                            }

                            Some(volts)
                        }
                        Ok(Err(e)) => {
                            let err_msg = format!("I2C error reading voltage: {}", e);
                            warn!(
                                serial = %serial,
                                error = %e,
                                "Failed to read voltage for board"
                            );
                            board_error = Some(err_msg);

                            // Increment failure counter
                            let health = board_health.entry(serial.clone()).or_default();
                            health.consecutive_failures += 1;
                            health.last_failure_time = Some(Instant::now());

                            if health.consecutive_failures >= self.recovery_config.failure_threshold {
                                warn!(
                                    serial = %serial,
                                    consecutive_failures = health.consecutive_failures,
                                    "Board marked as needing recovery"
                                );
                            }

                            None
                        }
                        Err(_) => {
                            let err_msg = "I2C timeout reading voltage (communication hung)".to_string();
                            warn!(
                                serial = %serial,
                                "Timeout reading voltage for board (I2C may be hung)"
                            );
                            board_error = Some(err_msg);

                            // Increment failure counter
                            let health = board_health.entry(serial.clone()).or_default();
                            health.consecutive_failures += 1;
                            health.last_failure_time = Some(Instant::now());

                            if health.consecutive_failures >= self.recovery_config.failure_threshold {
                                warn!(
                                    serial = %serial,
                                    consecutive_failures = health.consecutive_failures,
                                    "Board marked as needing recovery"
                                );
                            }

                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            // Read board temperature and fan speed if fan controller is available
            let (board_temp, fan_speed_rpm) = if let Some(fan_ctrl) = fan_controllers.get(serial) {
                let temp_future = async {
                    fan_ctrl.lock().await.get_external_temperature().await
                };

                let temp = match tokio::time::timeout(
                    tokio::time::Duration::from_millis(500),
                    temp_future
                ).await {
                    Ok(Ok(temp_c)) => {
                        debug!(
                            serial = %serial,
                            temperature = temp_c,
                            "Read board temperature"
                        );
                        Some(temp_c)
                    }
                    Ok(Err(e)) => {
                        warn!(
                            serial = %serial,
                            error = %e,
                            "Failed to read board temperature"
                        );
                        // Don't set board_error for temperature failures - voltage is more critical
                        None
                    }
                    Err(_) => {
                        warn!(
                            serial = %serial,
                            "Timeout reading board temperature"
                        );
                        None
                    }
                };

                let rpm_future = async {
                    fan_ctrl.lock().await.get_rpm().await
                };

                let rpm = match tokio::time::timeout(
                    tokio::time::Duration::from_millis(500),
                    rpm_future
                ).await {
                    Ok(Ok(rpm)) => {
                        debug!(
                            serial = %serial,
                            fan_rpm = rpm,
                            "Read fan speed"
                        );
                        Some(rpm as u16)
                    }
                    Ok(Err(e)) => {
                        warn!(
                            serial = %serial,
                            error = %e,
                            "Failed to read fan speed"
                        );
                        None
                    }
                    Err(_) => {
                        warn!(
                            serial = %serial,
                            "Timeout reading fan speed"
                        );
                        None
                    }
                };

                (temp, rpm)
            } else {
                (None, None)
            };

            // Get health state for this board
            let health = board_health.entry(serial.clone()).or_default();
            let needs_reinit = health.consecutive_failures >= self.recovery_config.failure_threshold;

            active_boards.push(BoardStatus {
                model: info.model.clone(),
                firmware_version: info.firmware_version.clone(),
                serial_number: serial.clone(),
                voltage_control_available: has_voltage_controller,
                current_voltage_v: current_voltage,
                transient_i2c_error: board_error,
                needs_reinit,
                consecutive_failures: health.consecutive_failures,
                retry_count: health.retry_count,
                board_temp_c: board_temp,
                fan_speed_rpm,
            });
        }

        BoardListResponse {
            active_boards,
            failed_boards: failed.clone(),
        }
    }
}

/// Echo request payload.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct EchoRequest {
    /// The message to echo back.
    #[schema(example = "Hello, Mujina!")]
    pub message: String,
}

/// Echo response payload.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct EchoResponse {
    /// The echoed message.
    #[schema(example = "Hello, Mujina!")]
    pub message: String,
}

/// Set voltage request payload.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SetVoltageRequest {
    /// Target voltage in volts (e.g., 1.2 for 1.2V)
    #[schema(example = 1.2, minimum = 0.5, maximum = 2.0)]
    pub voltage: f32,
}

/// Set voltage response payload.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct SetVoltageResponse {
    /// Whether the operation was successful
    #[schema(example = true)]
    pub success: bool,
    /// The requested voltage in volts
    #[schema(example = 1.2)]
    pub requested_voltage: f32,
    /// The actual voltage readback in volts (if successful)
    #[schema(example = 1.198)]
    pub actual_voltage: Option<f32>,
    /// Error message (if any)
    #[schema(example = "Voltage set to 1.200V (readback: 1.198V)")]
    pub message: Option<String>,
}

/// API error response.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ErrorResponse {
    /// Error message
    #[schema(example = "Board with serial 'XYZ789' not found")]
    pub error: String,
}

/// Reinitialize board response payload.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema)]
pub struct ReinitializeResponse {
    /// Whether the operation was successful
    #[schema(example = true)]
    pub success: bool,
    /// Descriptive message
    #[schema(example = "Board reinitialized successfully")]
    pub message: String,
    /// Previous error message if available
    #[schema(example = "I2C error: WriteRead failed: Response ID mismatch")]
    pub previous_error: Option<String>,
    /// Current voltage after reinitialization if available
    #[schema(example = 1.2)]
    pub current_voltage: Option<f32>,
}

/// Echo endpoint handler.
///
/// Echoes back the provided message. Useful for testing API connectivity.
#[utoipa::path(
    post,
    path = "/api/v1/echo",
    request_body = EchoRequest,
    responses(
        (status = 200, description = "Successfully echoed the message", body = EchoResponse)
    ),
    tag = "Testing"
)]
async fn echo(Json(req): Json<EchoRequest>) -> Json<EchoResponse> {
    Json(EchoResponse {
        message: req.message,
    })
}

/// Health check endpoint handler.
///
/// Returns a simple OK status to verify the API is running.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    responses(
        (status = 200, description = "API is healthy", body = String, example = "OK")
    ),
    tag = "Health"
)]
async fn health() -> &'static str {
    "OK"
}

/// List boards endpoint handler.
///
/// Returns a list of all registered boards with their status information.
/// Includes both successfully initialized boards and boards that failed to initialize.
///
/// # Response Structure
/// - `active_boards`: Successfully initialized boards. May include an `error` field if
///   the board is experiencing runtime issues (e.g., I2C communication failures)
/// - `failed_boards`: Boards that failed initial initialization with error details
///
/// # Example
/// ```bash
/// curl -s http://localhost:7785/api/v1/boards | jq -r '.'
/// ```
#[utoipa::path(
    get,
    path = "/api/v1/boards",
    responses(
        (status = 200, description = "List of all boards", body = BoardListResponse)
    ),
    tag = "Boards"
)]
async fn list_boards(State(state): State<AppState>) -> Json<BoardListResponse> {
    let boards = state.get_board_list().await;
    Json(boards)
}

/*   Set board voltage endpoint handler.

     Sets the core voltage for a specific board identified by its serial number.
     The voltage controller will validate the requested voltage against configured
     safe operating limits before applying it.

    # Example

    export BOARD_SERIAL_ID=123456
    curl -X POST http://localhost:7785/api/v1/board/$BOARD_SERIAL_ID/voltage \
       -H "Content-Type: application/json" \
       -d '{"voltage": 1.2}'
*/
#[utoipa::path(
    post,
    path = "/api/v1/board/{serial}/voltage",
    request_body = SetVoltageRequest,
    params(
        ("serial" = String, Path, description = "Board serial number", example = "ABC12345")
    ),
    responses(
        (status = 200, description = "Voltage successfully set", body = SetVoltageResponse),
        (status = 400, description = "Invalid voltage value", body = ErrorResponse),
        (status = 404, description = "Board not found or voltage control not available", body = ErrorResponse),
        (status = 500, description = "Failed to set voltage", body = SetVoltageResponse)
    ),
    tag = "Boards"
)]
async fn set_board_voltage(
    State(state): State<AppState>,
    Path(serial): Path<String>,
    Json(req): Json<SetVoltageRequest>,
) -> Response {
    debug!(
        serial = %serial,
        voltage = req.voltage,
        "API request to set board voltage"
    );

    // Validate voltage range (basic sanity check)
    if !(0.5..=2.0).contains(&req.voltage) {
        let error = ErrorResponse {
            error: format!(
                "Voltage {} is outside safe range (0.5V - 2.0V)",
                req.voltage
            ),
        };
        return (StatusCode::BAD_REQUEST, Json(error)).into_response();
    }

    // Look up the voltage controller in the registry
    let controllers = state.voltage_controllers.read().await;
    let controller = match controllers.get(&serial) {
        Some(controller) => controller.clone(),
        None => {
            let error = ErrorResponse {
                error: format!("Board with serial '{}' not found or does not support voltage control", serial),
            };
            return (StatusCode::NOT_FOUND, Json(error)).into_response();
        }
    };
    drop(controllers);

    // Acquire lock on the voltage controller
    let mut tps546 = controller.lock().await;

    // Set the voltage
    match tps546.set_vout(req.voltage).await {
        Ok(()) => {
            debug!(
                serial = %serial,
                voltage = req.voltage,
                "Voltage set command successful"
            );

            // Wait for voltage to stabilize
            tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

            // Verify voltage readback
            match tps546.get_vout().await {
                Ok(mv) => {
                    let actual_voltage = mv as f32 / 1000.0;
                    debug!(
                        serial = %serial,
                        requested = req.voltage,
                        actual = actual_voltage,
                        "Core voltage readback"
                    );

                    let response = SetVoltageResponse {
                        success: true,
                        requested_voltage: req.voltage,
                        actual_voltage: Some(actual_voltage),
                        message: Some(format!(
                            "Voltage set to {:.3}V (readback: {:.3}V)",
                            req.voltage, actual_voltage
                        )),
                    };
                    (StatusCode::OK, Json(response)).into_response()
                }
                Err(e) => {
                    warn!(
                        serial = %serial,
                        error = %e,
                        "Failed to read voltage after setting"
                    );

                    let response = SetVoltageResponse {
                        success: true,
                        requested_voltage: req.voltage,
                        actual_voltage: None,
                        message: Some(format!(
                            "Voltage set to {:.3}V but readback failed: {}",
                            req.voltage, e
                        )),
                    };
                    (StatusCode::OK, Json(response)).into_response()
                }
            }
        }
        Err(e) => {
            error!(
                serial = %serial,
                voltage = req.voltage,
                error = %e,
                "Failed to set voltage"
            );

            let response = SetVoltageResponse {
                success: false,
                requested_voltage: req.voltage,
                actual_voltage: None,
                message: Some(format!("Failed to set voltage: {}", e)),
            };
            (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
        }
    }
}

/*   Reinitialize board endpoint handler.

     Manually triggers reinitialization of a board that has experienced persistent failures.
     This endpoint resets the failure counters and attempts to re-probe the board.

    # Example

    export BOARD_SERIAL_ID=ABC12345
    curl -X POST http://localhost:7785/api/v1/board/$BOARD_SERIAL_ID/reinitialize
*/
#[utoipa::path(
    post,
    path = "/api/v1/board/{serial}/reinitialize",
    params(
        ("serial" = String, Path, description = "Board serial number", example = "ABC12345")
    ),
    responses(
        (status = 200, description = "Board reinitialized successfully", body = ReinitializeResponse),
        (status = 404, description = "Board not found", body = ErrorResponse),
        (status = 501, description = "Reinitialization not yet implemented", body = ReinitializeResponse)
    ),
    tag = "Boards"
)]
async fn reinitialize_board(
    State(state): State<AppState>,
    Path(serial): Path<String>,
) -> Response {
    debug!(
        serial = %serial,
        "API request to reinitialize board"
    );

    // Check if board exists (in active boards or failed boards)
    let boards = state.boards.read().await;
    let failed_boards = state.failed_boards.read().await;
    let in_active = boards.contains_key(&serial);
    let in_failed = failed_boards.iter().any(|b| b.serial_number.as_deref() == Some(&serial));
    drop(boards);
    drop(failed_boards);

    if !in_active && !in_failed {
        let error = ErrorResponse {
            error: format!("Board with serial '{}' not found", serial),
        };
        return (StatusCode::NOT_FOUND, Json(error)).into_response();
    }

    // Get current health state to capture previous error
    let mut board_health = state.board_health.write().await;
    let health = board_health.entry(serial.clone()).or_default();
    let previous_failures = health.consecutive_failures;

    // Reset health state immediately
    health.consecutive_failures = 0;
    health.last_failure_time = None;
    health.retry_count = 0;
    health.last_retry_time = None;
    drop(board_health);

    warn!(
        serial = %serial,
        previous_failures = previous_failures,
        "Manual board reinitialization requested"
    );

    // If backplane command channel is available, use it for full reinitialization
    if let Some(cmd_tx) = &state.backplane_cmd_tx {
        use crate::backplane_cmd::{BackplaneCommand, ReinitializeResult};
        use tokio::sync::oneshot;

        let (response_tx, response_rx) = oneshot::channel();

        let cmd = BackplaneCommand::ReinitializeBoard {
            serial: serial.clone(),
            response_tx,
        };

        // Send command to backplane
        if let Err(e) = cmd_tx.send(cmd).await {
            error!(
                serial = %serial,
                error = %e,
                "Failed to send reinitialize command to backplane"
            );

            let response = ReinitializeResponse {
                success: false,
                message: "Failed to communicate with backplane".to_string(),
                previous_error: if previous_failures > 0 {
                    Some(format!("{} consecutive failures before reset", previous_failures))
                } else {
                    None
                },
                current_voltage: None,
            };
            return (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response();
        }

        // Wait for response from backplane (with timeout)
        // Timeout must be longer than board init timeout to allow init to complete
        let api_timeout = state.board_init_timeout + Duration::from_secs(5);
        match tokio::time::timeout(api_timeout, response_rx).await {
            Ok(Ok(result)) => {
                warn!(
                    serial = %serial,
                    success = result.success,
                    message = %result.message,
                    "Board reinitialization completed"
                );

                let response = ReinitializeResponse {
                    success: result.success,
                    message: result.message,
                    previous_error: if previous_failures > 0 {
                        Some(format!("{} consecutive failures before reset", previous_failures))
                    } else {
                        result.error
                    },
                    current_voltage: result.current_voltage,
                };

                let status = if result.success {
                    StatusCode::OK
                } else {
                    StatusCode::INTERNAL_SERVER_ERROR
                };

                (status, Json(response)).into_response()
            }
            Ok(Err(_)) => {
                error!(serial = %serial, "Backplane response channel closed");
                let response = ReinitializeResponse {
                    success: false,
                    message: "Backplane did not respond".to_string(),
                    previous_error: None,
                    current_voltage: None,
                };
                (StatusCode::INTERNAL_SERVER_ERROR, Json(response)).into_response()
            }
            Err(_) => {
                error!(serial = %serial, "Timeout waiting for backplane response");
                let response = ReinitializeResponse {
                    success: false,
                    message: "Timeout waiting for backplane to reinitialize board".to_string(),
                    previous_error: None,
                    current_voltage: None,
                };
                (StatusCode::GATEWAY_TIMEOUT, Json(response)).into_response()
            }
        }
    } else {
        // Fallback: backplane command channel not available
        warn!(
            serial = %serial,
            "Backplane command channel not available, only resetting failure counters"
        );

        let response = ReinitializeResponse {
            success: true,
            message: "Failure counters reset (backplane command channel not configured)".to_string(),
            previous_error: if previous_failures > 0 {
                Some(format!("{} consecutive failures before reset", previous_failures))
            } else {
                None
            },
            current_voltage: None,
        };

        (StatusCode::OK, Json(response)).into_response()
    }
}

/// OpenAPI documentation for API v1.
#[derive(OpenApi)]
#[openapi(
    paths(
        echo,
        health,
        list_boards,
        set_board_voltage,
        reinitialize_board,
    ),
    components(
        schemas(
            EchoRequest,
            EchoResponse,
            BoardStatus,
            FailedBoardStatus,
            BoardListResponse,
            SetVoltageRequest,
            SetVoltageResponse,
            ErrorResponse,
            ReinitializeResponse,
        )
    ),
    tags(
        (name = "Health", description = "Health check endpoints"),
        (name = "Testing", description = "Testing and debugging endpoints"),
        (name = "Boards", description = "Board management and control endpoints")
    ),
    servers(
        (url = "/", description = "Current server")
    ),
    info(
        title = "Mujina Miner API",
        version = "1.0.0",
        description = "REST API for controlling and monitoring Mujina Bitcoin mining hardware",
        license(
            name = "GPL-3.0-or-later",
            url = "https://www.gnu.org/licenses/gpl-3.0.html"
        )
    )
)]
pub struct ApiDoc;

/// Build the v1 API routes.
pub fn routes(state: AppState) -> Router {
    Router::new()
        .route("/echo", post(echo))
        .route("/health", get(health))
        .route("/boards", get(list_boards))
        .route("/board/:serial/voltage", post(set_board_voltage))
        .route("/board/:serial/reinitialize", post(reinitialize_board))
        .with_state(state)
}

#[cfg(test)]
mod tests {
    use super::*;

    // ============================================
    // BoardHealthState tests
    // ============================================

    #[test]
    fn test_board_health_state_default() {
        let state = BoardHealthState::default();

        assert_eq!(state.consecutive_failures, 0);
        assert!(state.last_failure_time.is_none());
        assert_eq!(state.retry_count, 0);
        assert!(state.last_retry_time.is_none());
    }

    #[test]
    fn test_board_health_state_clone() {
        let mut state = BoardHealthState::default();
        state.consecutive_failures = 5;
        state.retry_count = 2;
        state.last_failure_time = Some(Instant::now());

        let cloned = state.clone();

        assert_eq!(cloned.consecutive_failures, 5);
        assert_eq!(cloned.retry_count, 2);
        assert!(cloned.last_failure_time.is_some());
    }

    #[test]
    fn test_board_health_state_failure_tracking() {
        let mut state = BoardHealthState::default();

        // Simulate consecutive failures
        for i in 1..=5 {
            state.consecutive_failures = i;
            state.last_failure_time = Some(Instant::now());
        }

        assert_eq!(state.consecutive_failures, 5);
        assert!(state.last_failure_time.is_some());
    }

    // ============================================
    // BoardRecoveryConfig tests
    // ============================================

    #[test]
    fn test_board_recovery_config_default_values() {
        // Clear any environment variables that might affect the test
        std::env::remove_var("MUJINA_BOARD_FAILURE_THRESHOLD");
        std::env::remove_var("MUJINA_BOARD_MAX_AUTO_RETRIES");
        std::env::remove_var("MUJINA_BOARD_RETRY_INTERVAL");
        std::env::remove_var("MUJINA_BOARD_AUTO_RECOVERY");

        let config = BoardRecoveryConfig::default();

        assert_eq!(config.failure_threshold, 3);
        assert_eq!(config.max_auto_retries, 3);
        assert_eq!(config.retry_interval, Duration::from_secs(30));
        assert!(!config.auto_recovery_enabled);
    }

    #[test]
    fn test_board_recovery_config_clone() {
        let config = BoardRecoveryConfig {
            failure_threshold: 5,
            max_auto_retries: 10,
            retry_interval: Duration::from_secs(60),
            auto_recovery_enabled: true,
        };

        let cloned = config.clone();

        assert_eq!(cloned.failure_threshold, 5);
        assert_eq!(cloned.max_auto_retries, 10);
        assert_eq!(cloned.retry_interval, Duration::from_secs(60));
        assert!(cloned.auto_recovery_enabled);
    }

    // ============================================
    // FailedBoardStatus tests
    // ============================================

    #[test]
    fn test_failed_board_status_serialization() {
        let status = FailedBoardStatus {
            model: Some("Bitaxe Gamma".to_string()),
            serial_number: Some("ABC12345".to_string()),
            error: "I2C communication timeout".to_string(),
        };

        let json = serde_json::to_string(&status).expect("serialization should succeed");

        assert!(json.contains("Bitaxe Gamma"));
        assert!(json.contains("ABC12345"));
        assert!(json.contains("I2C communication timeout"));
    }

    #[test]
    fn test_failed_board_status_with_none_fields() {
        let status = FailedBoardStatus {
            model: None,
            serial_number: None,
            error: "Unknown error".to_string(),
        };

        let json = serde_json::to_string(&status).expect("serialization should succeed");

        assert!(json.contains("null") || json.contains("\"model\":null"));
        assert!(json.contains("Unknown error"));
    }

    // ============================================
    // BoardStatus tests
    // ============================================

    #[test]
    fn test_board_status_serialization() {
        let status = BoardStatus {
            model: "Bitaxe Gamma".to_string(),
            firmware_version: Some("2.1.4".to_string()),
            serial_number: "ABC12345".to_string(),
            voltage_control_available: true,
            current_voltage_v: Some(1.15),
            board_temp_c: Some(45.5),
            fan_speed_rpm: Some(4500),
            transient_i2c_error: None,
            needs_reinit: false,
            consecutive_failures: 0,
            retry_count: 0,
        };

        let json = serde_json::to_string(&status).expect("serialization should succeed");

        assert!(json.contains("Bitaxe Gamma"));
        assert!(json.contains("1.15"));
        assert!(json.contains("45.5"));
        assert!(json.contains("4500"));
    }

    #[test]
    fn test_board_status_with_error() {
        let status = BoardStatus {
            model: "Bitaxe Gamma".to_string(),
            firmware_version: None,
            serial_number: "ABC12345".to_string(),
            voltage_control_available: true,
            current_voltage_v: None,
            board_temp_c: None,
            fan_speed_rpm: None,
            transient_i2c_error: Some("I2C timeout".to_string()),
            needs_reinit: true,
            consecutive_failures: 5,
            retry_count: 2,
        };

        let json = serde_json::to_string(&status).expect("serialization should succeed");

        assert!(json.contains("I2C timeout"));
        assert!(json.contains("\"needs_reinit\":true"));
        assert!(json.contains("\"consecutive_failures\":5"));
    }

    // ============================================
    // BoardListResponse tests
    // ============================================

    #[test]
    fn test_board_list_response_serialization() {
        let response = BoardListResponse {
            active_boards: vec![BoardStatus {
                model: "Bitaxe Gamma".to_string(),
                firmware_version: Some("bitaxe-raw".to_string()),
                serial_number: "SERIAL001".to_string(),
                voltage_control_available: true,
                current_voltage_v: Some(1.2),
                board_temp_c: Some(50.0),
                fan_speed_rpm: Some(5000),
                transient_i2c_error: None,
                needs_reinit: false,
                consecutive_failures: 0,
                retry_count: 0,
            }],
            failed_boards: vec![FailedBoardStatus {
                model: Some("Bitaxe Gamma".to_string()),
                serial_number: Some("SERIAL002".to_string()),
                error: "Init failed".to_string(),
            }],
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");

        assert!(json.contains("active_boards"));
        assert!(json.contains("failed_boards"));
        assert!(json.contains("SERIAL001"));
        assert!(json.contains("SERIAL002"));
    }

    #[test]
    fn test_board_list_response_empty() {
        let response = BoardListResponse {
            active_boards: vec![],
            failed_boards: vec![],
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");

        assert!(json.contains("\"active_boards\":[]"));
        assert!(json.contains("\"failed_boards\":[]"));
    }

    // ============================================
    // AppState tests
    // ============================================

    #[test]
    fn test_app_state_default() {
        // Clear environment variable to ensure default
        std::env::remove_var("MUJINA_BOARD_INIT_TIMEOUT_SECS");

        let state = AppState::default();

        assert!(state.backplane_cmd_tx.is_none());
        assert_eq!(
            state.board_init_timeout,
            Duration::from_secs(DEFAULT_BOARD_INIT_TIMEOUT_SECS)
        );
    }

    #[test]
    fn test_app_state_new() {
        let state = AppState::new();

        assert!(state.backplane_cmd_tx.is_none());
    }

    // ============================================
    // API response type tests
    // ============================================

    #[test]
    fn test_reinitialize_response_serialization() {
        let response = ReinitializeResponse {
            success: true,
            message: "Board reinitialized successfully".to_string(),
            previous_error: Some("Previous I2C error".to_string()),
            current_voltage: Some(1.15),
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");

        assert!(json.contains("\"success\":true"));
        assert!(json.contains("Board reinitialized successfully"));
        assert!(json.contains("Previous I2C error"));
        assert!(json.contains("1.15"));
    }

    #[test]
    fn test_set_voltage_response_serialization() {
        let response = SetVoltageResponse {
            success: true,
            requested_voltage: 1.2,
            actual_voltage: Some(1.198),
            message: Some("Voltage set successfully".to_string()),
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");

        assert!(json.contains("\"success\":true"));
        assert!(json.contains("1.2"));
        assert!(json.contains("1.198"));
    }

    #[test]
    fn test_error_response_serialization() {
        let response = ErrorResponse {
            error: "Board not found".to_string(),
        };

        let json = serde_json::to_string(&response).expect("serialization should succeed");

        assert!(json.contains("Board not found"));
    }
}
