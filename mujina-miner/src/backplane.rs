//! Backplane for board communication and lifecycle management.
//!
//! The Backplane acts as the communication substrate between mining boards and
//! the scheduler. Like a hardware backplane, it provides connection points for
//! boards to plug into, routes events between components, and manages board
//! lifecycle (hotplug, emergency shutdown, etc.).

use crate::{
    api::{AppState, FailedBoardStatus},
    asic::hash_thread::HashThread,
    backplane_cmd::{BackplaneCommand, ReinitializeResult},
    board::{Board, BoardDescriptor, VirtualBoardRegistry},
    error::Result,
    tracing::prelude::*,
    transport::{
        cpu::TransportEvent as CpuTransportEvent, usb::TransportEvent as UsbTransportEvent,
        TransportEvent, UsbDeviceInfo,
    },
};
use std::{collections::HashMap, time::Duration};
use tokio::sync::mpsc;

/// Get board initialization timeout from environment or use default.
fn get_board_init_timeout() -> Duration {
    std::env::var("MUJINA_BOARD_INIT_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(Duration::from_secs)
        .unwrap_or(Duration::from_secs(30))
}

/// Board registry that uses inventory to find registered boards.
pub struct BoardRegistry;

impl BoardRegistry {
    /// Find the best matching board descriptor for this USB device.
    ///
    /// Uses pattern matching with specificity scoring to select the most
    /// appropriate board handler. When multiple patterns match, the one
    /// with the highest specificity score wins.
    ///
    /// Returns None if no registered boards match the device.
    pub fn find_descriptor(&self, device: &UsbDeviceInfo) -> Option<&'static BoardDescriptor> {
        inventory::iter::<BoardDescriptor>()
            .filter(|desc| desc.pattern.matches(device))
            .max_by_key(|desc| desc.pattern.specificity())
    }
}

/// Backplane that connects boards to the scheduler.
///
/// Acts as the communication substrate between mining boards and the work
/// scheduler. Boards plug into the backplane, which routes their events and
/// manages their lifecycle.
pub struct Backplane {
    registry: BoardRegistry,
    virtual_registry: VirtualBoardRegistry,
    /// Active boards managed by the backplane
    boards: HashMap<String, Box<dyn Board + Send>>,
    /// Device info for each board (for reinitialization)
    board_devices: HashMap<String, UsbDeviceInfo>,
    event_rx: mpsc::Receiver<TransportEvent>,
    /// Command channel for external control (API, MQTT, etc.)
    cmd_rx: mpsc::Receiver<BackplaneCommand>,
    /// Channel to send hash threads to the scheduler
    scheduler_tx: mpsc::Sender<Box<dyn HashThread>>,
    /// Shared API state for registering board controllers
    api_state: AppState,
}

impl Backplane {
    /// Create a new backplane.
    pub fn new(
        event_rx: mpsc::Receiver<TransportEvent>,
        cmd_rx: mpsc::Receiver<BackplaneCommand>,
        scheduler_tx: mpsc::Sender<Box<dyn HashThread>>,
        api_state: AppState,
    ) -> Self {
        Self {
            registry: BoardRegistry,
            virtual_registry: VirtualBoardRegistry,
            boards: HashMap::new(),
            board_devices: HashMap::new(),
            event_rx,
            cmd_rx,
            scheduler_tx,
            api_state,
        }
    }

    /// Run the backplane event loop.
    pub async fn run(&mut self) -> Result<()> {
        loop {
            tokio::select! {
                Some(event) = self.event_rx.recv() => {
                    match event {
                        TransportEvent::Usb(usb_event) => {
                            self.handle_usb_event(usb_event).await?;
                        }
                        TransportEvent::Cpu(cpu_event) => {
                            self.handle_cpu_event(cpu_event).await?;
                        }
                    }
                }
                Some(cmd) = self.cmd_rx.recv() => {
                    self.handle_command(cmd).await;
                }
                else => break,
            }
        }

        Ok(())
    }

    /// Shutdown all boards managed by this backplane.
    pub async fn shutdown_all_boards(&mut self) {
        let board_ids: Vec<String> = self.boards.keys().cloned().collect();

        for board_id in board_ids {
            if let Some(mut board) = self.boards.remove(&board_id) {
                let model = board.board_info().model;
                debug!(board = %model, serial = %board_id, "Shutting down board");

                match board.shutdown().await {
                    Ok(()) => {
                        debug!(board = %model, serial = %board_id, "Board shutdown complete");
                    }
                    Err(e) => {
                        error!(
                            board = %model,
                            serial = %board_id,
                            error = %e,
                            "Failed to shutdown board"
                        );
                    }
                }
            }
        }
    }

    /// Handle commands from external interfaces (API, MQTT, etc.).
    async fn handle_command(&mut self, cmd: BackplaneCommand) {
        match cmd {
            BackplaneCommand::ReinitializeBoard { serial, response_tx } => {
                let result = self.reinitialize_board(&serial).await;
                // Send response back (ignore if receiver dropped)
                let _ = response_tx.send(result);
            }
        }
    }

    /// Reinitialize a specific board by serial number.
    async fn reinitialize_board(&mut self, serial: &str) -> ReinitializeResult {
        // Check if board exists
        if !self.boards.contains_key(serial) {
            return ReinitializeResult::failure(
                "Board not found".to_string(),
                format!("No board with serial '{}' is currently active", serial),
            );
        }

        // Get the device info before removing the board
        let device_info = match self.board_devices.get(serial) {
            Some(info) => info.clone(),
            None => {
                return ReinitializeResult::failure(
                    "Device info not found".to_string(),
                    format!("No device info stored for board '{}'", serial),
                );
            }
        };

        // Remove the board
        if let Some(mut board) = self.boards.remove(serial) {
            let board_info = board.board_info();
            let model = board_info.model.clone();

            info!(
                serial = %serial,
                model = %model,
                "Beginning board reinitialization"
            );

            // Shutdown the existing board
            match board.shutdown().await {
                Ok(()) => {
                    info!(serial = %serial, model = %model, "Board shutdown complete");
                }
                Err(e) => {
                    warn!(
                        serial = %serial,
                        model = %model,
                        error = %e,
                        "Error during board shutdown (continuing with reinitialization)"
                    );
                }
            }

            // Unregister from API
            self.api_state.unregister_voltage_controller(serial).await;
            self.api_state.unregister_fan_controller(serial).await;
            self.api_state.unregister_board(serial).await;

            // Remove from device tracking
            self.board_devices.remove(serial);

            // Drop the board to release serial ports before reprobing
            drop(board);

            info!(
                serial = %serial,
                model = %model,
                "Board shutdown complete, reprobing device"
            );

            // Re-probe the device to reinitialize it
            match self.handle_usb_event(UsbTransportEvent::UsbDeviceConnected(device_info)).await {
                Ok(()) => {
                    info!(
                        serial = %serial,
                        model = %model,
                        "Board successfully reinitialized"
                    );

                    // Try to read the new voltage
                    let new_voltage = if let Some(controller) = self.api_state.voltage_controllers.read().await.get(serial) {
                        match tokio::time::timeout(
                            Duration::from_millis(500),
                            async { controller.lock().await.get_vout().await }
                        ).await {
                            Ok(Ok(mv)) => Some(mv as f32 / 1000.0),
                            _ => None,
                        }
                    } else {
                        None
                    };

                    ReinitializeResult::success(
                        format!("Board '{}' ({}) successfully reinitialized", serial, model),
                        new_voltage,
                    )
                }
                Err(e) => {
                    warn!(
                        serial = %serial,
                        model = %model,
                        error = %e,
                        "Failed to reprobe device after shutdown"
                    );

                    ReinitializeResult::failure(
                        format!("Board shutdown succeeded but reprobe failed"),
                        format!("Reprobe error: {}", e),
                    )
                }
            }
        } else {
            ReinitializeResult::failure(
                "Failed to remove board".to_string(),
                format!("Board '{}' disappeared during reinitialization", serial),
            )
        }
    }

    /// Handle USB transport events.
    async fn handle_usb_event(&mut self, event: UsbTransportEvent) -> Result<()> {
        match event {
            UsbTransportEvent::UsbDeviceConnected(device_info) => {
                // Check if this device matches any registered board pattern
                let Some(descriptor) = self.registry.find_descriptor(&device_info) else {
                    // No match - this is expected for most USB devices
                    return Ok(());
                };

                // Pattern matched - log the match
                info!(
                    board = descriptor.name,
                    vid = %format!("{:04x}", device_info.vid),
                    pid = %format!("{:04x}", device_info.pid),
                    manufacturer = ?device_info.manufacturer,
                    product = ?device_info.product,
                    serial = ?device_info.serial_number,
                    "Hash board connected via USB."
                );

                // Capture info needed for error reporting and reinitialization
                let board_name = descriptor.name;
                let serial_for_error = device_info.serial_number.clone();
                let device_info_clone = device_info.clone(); // Save for reinitialization

                // Create the board using the descriptor's factory function with timeout
                let timeout = get_board_init_timeout();
                debug!(
                    board = board_name,
                    timeout_secs = timeout.as_secs(),
                    "Starting board initialization with timeout"
                );

                // Spawn the initialization task so we can truly abandon it on timeout
                let create_task = tokio::spawn((descriptor.create_fn)(device_info));

                let mut board = match tokio::time::timeout(timeout, create_task).await {
                    Ok(Ok(Ok(board))) => board,
                    Ok(Ok(Err(e))) => {
                        error!(
                            board = board_name,
                            error = %e,
                            "Failed to create board"
                        );

                        // Register the failed board
                        self.api_state
                            .register_failed_board(FailedBoardStatus {
                                model: Some(board_name.to_string()),
                                serial_number: serial_for_error.clone(),
                                error: format!("Failed to create board: {}", e),
                            })
                            .await;

                        return Ok(());
                    }
                    Ok(Err(join_error)) => {
                        error!(
                            board = board_name,
                            error = %join_error,
                            "Board initialization task panicked"
                        );

                        // Register the failed board
                        self.api_state
                            .register_failed_board(FailedBoardStatus {
                                model: Some(board_name.to_string()),
                                serial_number: serial_for_error.clone(),
                                error: format!("Board initialization task panicked: {}", join_error),
                            })
                            .await;

                        return Ok(());
                    }
                    Err(_) => {
                        error!(
                            board = board_name,
                            timeout_secs = timeout.as_secs(),
                            "Board initialization timed out (task abandoned)"
                        );

                        // Register the failed board due to timeout
                        self.api_state
                            .register_failed_board(FailedBoardStatus {
                                model: Some(board_name.to_string()),
                                serial_number: serial_for_error,
                                error: format!("Board initialization timed out after {} seconds", timeout.as_secs()),
                            })
                            .await;

                        return Ok(());
                    }
                };

                let board_info = board.board_info();
                let board_id = board_info
                    .serial_number
                    .clone()
                    .unwrap_or_else(|| "unknown".to_string());

                debug!(
                    model = %board_info.model,
                    serial = %board_id,
                    "Board created successfully, registering with API"
                );

                // Register board information with API
                self.api_state
                    .register_board(board_id.clone(), board_info.clone())
                    .await;

                // Register voltage controller with API if board supports it
                // This must be done before create_hash_threads() which may consume resources
                if let Some(bitaxe_board) = board.as_any().downcast_ref::<crate::board::bitaxe::BitaxeBoard>() {
                    if let Some(regulator) = bitaxe_board.get_voltage_regulator() {
                        debug!(
                            board = %board_info.model,
                            serial = %board_id,
                            "Registering voltage controller with API"
                        );
                        self.api_state
                            .register_voltage_controller(board_id.clone(), regulator)
                            .await;
                    }

                    // Register fan controller with API for temperature readings
                    if let Some(fan_ctrl) = bitaxe_board.get_fan_controller() {
                        debug!(
                            board = %board_info.model,
                            serial = %board_id,
                            "Registering fan controller with API"
                        );
                        self.api_state
                            .register_fan_controller(board_id.clone(), fan_ctrl)
                            .await;
                    }
                }

                // Create hash threads from the board
                match board.create_hash_threads().await {
                    Ok(threads) => {
                        // Store board for lifecycle management
                        self.boards.insert(board_id.clone(), board);
                        // Store device info for reinitialization
                        self.board_devices.insert(board_id.clone(), device_info_clone);

                        // Send threads to scheduler individually
                        for thread in threads {
                            if let Err(e) = self.scheduler_tx.send(thread).await {
                                tracing::error!(
                                    board = %board_info.model,
                                    error = %e,
                                    "Failed to send thread to scheduler"
                                );
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        tracing::error!(
                            board = %board_info.model,
                            serial = %board_id,
                            error = %e,
                            "Hash board failed to start."
                        );

                        // Register the failed board and unregister the partially initialized one
                        self.api_state.unregister_board(&board_id).await;
                        self.api_state
                            .register_failed_board(FailedBoardStatus {
                                model: Some(board_info.model.clone()),
                                serial_number: Some(board_id.clone()),
                                error: format!("Failed to create hash threads: {}", e),
                            })
                            .await;
                    }
                }
            }
            UsbTransportEvent::UsbDeviceDisconnected { device_path: _ } => {
                // Find and shutdown the board
                // Note: Current design uses serial number as key, but we get device_path
                // in disconnect event. For single-board setups this works fine.
                // TODO: Maintain device_path -> board_id mapping for multi-board support
                let board_ids: Vec<String> = self.boards.keys().cloned().collect();
                for board_id in board_ids {
                    if let Some(mut board) = self.boards.remove(&board_id) {
                        let model = board.board_info().model;
                        debug!(board = %model, serial = %board_id, "Shutting down board");

                        match board.shutdown().await {
                            Ok(()) => {
                                info!(
                                    board = %model,
                                    serial = %board_id,
                                    "Board disconnected"
                                );
                            }
                            Err(e) => {
                                tracing::error!(
                                    board = %model,
                                    serial = %board_id,
                                    error = %e,
                                    "Failed to shutdown board"
                                );
                            }
                        }

                        // Unregister voltage controller, fan controller, and board info from API
                        self.api_state.unregister_voltage_controller(&board_id).await;
                        self.api_state.unregister_fan_controller(&board_id).await;
                        self.api_state.unregister_board(&board_id).await;

                        // Don't re-insert - board is removed
                        break; // For now, assume one board per device
                    }
                }
            }
        }

        Ok(())
    }

    /// Handle CPU miner transport events.
    async fn handle_cpu_event(&mut self, event: CpuTransportEvent) -> Result<()> {
        match event {
            CpuTransportEvent::CpuDeviceConnected(device_info) => {
                // Find the virtual board descriptor for cpu_miner
                let Some(descriptor) = self.virtual_registry.find("cpu_miner") else {
                    error!("No virtual board descriptor found for cpu_miner");
                    return Ok(());
                };

                info!(
                    board = descriptor.name,
                    threads = device_info.thread_count,
                    duty = device_info.duty_percent,
                    "CPU miner board connected."
                );

                // Create the board using the descriptor's factory function
                let mut board = match (descriptor.create_fn)().await {
                    Ok(board) => board,
                    Err(e) => {
                        error!(
                            board = descriptor.name,
                            error = %e,
                            "Failed to create CPU miner board"
                        );

                        // Register the failed board
                        self.api_state
                            .register_failed_board(FailedBoardStatus {
                                model: Some(descriptor.name.to_string()),
                                serial_number: Some(device_info.device_id.clone()),
                                error: format!("Failed to create CPU miner: {}", e),
                            })
                            .await;

                        return Ok(());
                    }
                };

                let board_info = board.board_info();
                let board_id = device_info.device_id.clone();

                // Register board information with API
                self.api_state
                    .register_board(board_id.clone(), board_info.clone())
                    .await;

                // Create hash threads from the board
                match board.create_hash_threads().await {
                    Ok(threads) => {
                        let thread_count = threads.len();

                        // Store board for lifecycle management
                        self.boards.insert(board_id.clone(), board);

                        // Send threads to scheduler individually
                        for thread in threads {
                            if let Err(e) = self.scheduler_tx.send(thread).await {
                                tracing::error!(
                                    board = %board_info.model,
                                    error = %e,
                                    "Failed to send thread to scheduler"
                                );
                                break;
                            }
                        }

                        info!(
                            board = %board_info.model,
                            threads = thread_count,
                            "CPU miner started."
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            board = %board_info.model,
                            error = %e,
                            "CPU miner failed to start."
                        );

                        // Register the failed board and unregister the partially initialized one
                        self.api_state.unregister_board(&board_id).await;
                        self.api_state
                            .register_failed_board(FailedBoardStatus {
                                model: Some(board_info.model.clone()),
                                serial_number: Some(board_id.clone()),
                                error: format!("Failed to create hash threads: {}", e),
                            })
                            .await;
                    }
                }
            }
            CpuTransportEvent::CpuDeviceDisconnected { device_id } => {
                if let Some(mut board) = self.boards.remove(&device_id) {
                    let model = board.board_info().model;
                    debug!(board = %model, id = %device_id, "Shutting down CPU miner");

                    match board.shutdown().await {
                        Ok(()) => {
                            info!(board = %model, id = %device_id, "CPU miner disconnected");
                        }
                        Err(e) => {
                            tracing::error!(
                                board = %model,
                                id = %device_id,
                                error = %e,
                                "Failed to shutdown CPU miner"
                            );
                        }
                    }

                    // Unregister board info from API
                    self.api_state.unregister_board(&device_id).await;
                }
            }
        }

        Ok(())
    }
}
