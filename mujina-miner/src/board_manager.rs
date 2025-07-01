//! Board lifecycle and hotplug management.
//!
//! This module manages the discovery, creation, and lifecycle of mining
//! boards. It listens for transport events, looks up board types in the
//! registry, and creates appropriate board instances.

use crate::board::Board;
use crate::error::Result;
use std::collections::HashMap;
use tokio::sync::mpsc;

/// USB vendor and product IDs for board identification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct UsbId {
    pub vid: u16,
    pub pid: u16,
}

/// Board type enumeration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoardType {
    BitaxeGamma,
    S19Pro,
    Avalon1366,
    // Add more board types as supported
}

/// Registry of known board types.
pub struct BoardRegistry {
    usb_boards: HashMap<UsbId, BoardType>,
}

impl BoardRegistry {
    /// Create a new board registry with known boards.
    pub fn new() -> Self {
        let mut usb_boards = HashMap::new();
        
        // Register known USB boards
        // TODO: Get actual VID/PID values
        usb_boards.insert(UsbId { vid: 0x0403, pid: 0x6001 }, BoardType::BitaxeGamma);
        
        Self { usb_boards }
    }
    
    /// Find board type by USB VID/PID.
    pub fn find_by_usb(&self, vid: u16, pid: u16) -> Option<BoardType> {
        self.usb_boards.get(&UsbId { vid, pid }).copied()
    }
}

/// Transport discovery event.
#[derive(Debug)]
pub enum TransportEvent {
    UsbConnected {
        vid: u16,
        pid: u16,
        serial_number: Option<String>,
        // Transport handles would go here
    },
    UsbDisconnected {
        serial_number: Option<String>,
    },
    // Future: PCIe, Ethernet, etc.
}

/// Board manager that handles board lifecycle.
pub struct BoardManager {
    registry: BoardRegistry,
    #[expect(dead_code, reason = "Will be used when board creation is implemented")]
    boards: HashMap<String, Box<dyn Board>>,
    event_rx: mpsc::Receiver<TransportEvent>,
}

impl BoardManager {
    /// Create a new board manager.
    pub fn new(event_rx: mpsc::Receiver<TransportEvent>) -> Self {
        Self {
            registry: BoardRegistry::new(),
            boards: HashMap::new(),
            event_rx,
        }
    }
    
    /// Run the board manager event loop.
    pub async fn run(&mut self) -> Result<()> {
        while let Some(event) = self.event_rx.recv().await {
            match event {
                TransportEvent::UsbConnected { vid, pid, serial_number: _, .. } => {
                    tracing::info!("USB device connected: {:04x}:{:04x}", vid, pid);
                    
                    if let Some(board_type) = self.registry.find_by_usb(vid, pid) {
                        // TODO: Create board instance based on type
                        tracing::info!("Identified board type: {:?}", board_type);
                    } else {
                        tracing::warn!("Unknown USB device: {:04x}:{:04x}", vid, pid);
                    }
                }
                TransportEvent::UsbDisconnected { serial_number } => {
                    tracing::info!("USB device disconnected: {:?}", serial_number);
                    // TODO: Remove board from active boards
                }
            }
        }
        
        Ok(())
    }
}