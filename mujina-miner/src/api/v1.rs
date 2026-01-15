//! API version 1 endpoints.

use axum::{
    extract::{Json, Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
    Router,
};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, sync::Arc};
use tokio::sync::{Mutex, RwLock};
use tracing::{debug, error, warn};
use utoipa::{OpenApi, ToSchema};

use crate::{
    board::BoardInfo,
    hw_trait::I2c,
    mgmt_protocol::bitaxe_raw::i2c::BitaxeRawI2c,
    peripheral::tps546::Tps546,
};

/// Voltage controller handle for a board.
pub type VoltageControllerHandle = Arc<Mutex<Tps546<BitaxeRawI2c>>>;

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
    /// Whether the board is currently connected
    #[schema(example = true)]
    pub connected: bool,
    /// Whether voltage control is available for this board
    #[schema(example = true)]
    pub voltage_control_available: bool,
    /// Current voltage in volts (if voltage control is available)
    #[schema(example = 1.2)]
    pub current_voltage_v: Option<f32>,
    /// Error message if the board is experiencing issues (e.g., I2C communication failure)
    #[schema(example = "I2C communication timeout")]
    pub error: Option<String>,
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
#[derive(Clone, Default)]
pub struct AppState {
    /// Registry of voltage controllers by board serial number
    pub voltage_controllers: Arc<RwLock<HashMap<String, VoltageControllerHandle>>>,
    /// Registry of board information by serial number
    pub boards: Arc<RwLock<HashMap<String, BoardInfo>>>,
    /// Registry of failed board initialization attempts
    pub failed_boards: Arc<RwLock<Vec<FailedBoardStatus>>>,
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

    /// Get a list of all registered boards with their status.
    pub async fn get_board_list(&self) -> BoardListResponse {
        let boards = self.boards.read().await;
        let controllers = self.voltage_controllers.read().await;
        let failed = self.failed_boards.read().await;

        debug!(
            board_count = boards.len(),
            controller_count = controllers.len(),
            failed_count = failed.len(),
            "Getting board list"
        );

        let mut active_boards = Vec::new();

        for (serial, info) in boards.iter() {
            let has_controller = controllers.contains_key(serial);

            // Read current voltage if controller is available and track any errors
            let mut board_error: Option<String> = None;
            let current_voltage = if has_controller {
                if let Some(controller) = controllers.get(serial) {
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
                            None
                        }
                        Err(_) => {
                            let err_msg = "I2C timeout reading voltage (communication hung)".to_string();
                            warn!(
                                serial = %serial,
                                "Timeout reading voltage for board (I2C may be hung)"
                            );
                            board_error = Some(err_msg);
                            None
                        }
                    }
                } else {
                    None
                }
            } else {
                None
            };

            active_boards.push(BoardStatus {
                model: info.model.clone(),
                firmware_version: info.firmware_version.clone(),
                serial_number: serial.clone(),
                connected: true,
                voltage_control_available: has_controller,
                current_voltage_v: current_voltage,
                error: board_error,
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
/// curl http://localhost:7785/api/v1/boards
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

/// OpenAPI documentation for API v1.
#[derive(OpenApi)]
#[openapi(
    paths(
        echo,
        health,
        list_boards,
        set_board_voltage,
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
        .with_state(state)
}
