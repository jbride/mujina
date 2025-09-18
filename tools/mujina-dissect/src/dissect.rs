//! Protocol dissection engine.
//!
//! TODO: Build comprehensive unit tests based on known serial captures
//! - Use captured frames from ~/mujina/captures/bitaxe-gamma-logic/esp-miner-boot.csv
//! - Test CRC validation for both work frames (CRC16) and response frames (CRC5)
//! - Test frame parsing for JobFull work frames and register responses
//! - Add regression tests to prevent future parsing failures

use crate::i2c::I2cOperation;
use crate::serial::{Direction, SerialFrame};
use colored::Colorize;
use mujina_miner::asic::bm13xx::crc::crc5_is_valid;
use mujina_miner::asic::bm13xx::protocol::Command;
use mujina_miner::peripheral::{emc2101, pmbus};
use std::collections::HashMap;
use std::fmt;

/// Dissected frame with decoded content
#[derive(Debug)]
pub struct DissectedFrame {
    pub timestamp: f64,
    pub direction: Direction,
    pub raw_data: Vec<u8>,
    pub content: FrameContent,
    pub crc_status: CrcStatus,
}

/// Decoded frame content
#[derive(Debug)]
pub enum FrameContent {
    Command(Command),
    Unknown(String),
    Invalid(String),
}

/// CRC validation status
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrcStatus {
    Valid,
    Invalid,
    NotChecked,
}

impl fmt::Display for CrcStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CrcStatus::Valid => write!(f, "{}", "CRC OK".green()),
            CrcStatus::Invalid => write!(f, "{}", "CRC FAIL".red()),
            CrcStatus::NotChecked => write!(f, ""),
        }
    }
}

/// Dissect a serial frame
pub fn dissect_serial_frame(frame: &SerialFrame) -> DissectedFrame {
    let (content, crc_status) = match frame.direction {
        Direction::HostToChip => dissect_command(&frame.data),
        Direction::ChipToHost => dissect_response(&frame.data),
    };

    DissectedFrame {
        timestamp: frame.start_time,
        direction: frame.direction,
        raw_data: frame.data.clone(),
        content,
        crc_status,
    }
}

/// Dissect a command frame using main library types
fn dissect_command(data: &[u8]) -> (FrameContent, CrcStatus) {
    match Command::try_parse_frame(data) {
        Ok((command, crc_valid)) => (
            FrameContent::Command(command),
            if crc_valid {
                CrcStatus::Valid
            } else {
                CrcStatus::Invalid
            },
        ),
        Err(e) => (
            FrameContent::Invalid(format!("Parse error: {}", e)),
            CrcStatus::NotChecked,
        ),
    }
}

/// Dissect a response frame (simplified for now)
fn dissect_response(data: &[u8]) -> (FrameContent, CrcStatus) {
    if data.len() < 3 {
        return (
            FrameContent::Invalid(format!("Response too short: {} bytes", data.len())),
            CrcStatus::NotChecked,
        );
    }

    // Check preamble
    if data[0] != 0xAA || data[1] != 0x55 {
        return (
            FrameContent::Invalid("Invalid response preamble".to_string()),
            CrcStatus::NotChecked,
        );
    }

    // For now, use a simple response representation until we need full parsing
    // Response frames use CRC5 (confirmed by test case in crc.rs)
    let payload = &data[2..];
    let crc_valid = crc5_is_valid(payload);
    let crc_status = if crc_valid {
        CrcStatus::Valid
    } else {
        CrcStatus::Invalid
    };

    // Simple response parsing - can be enhanced later
    let content = if data.len() >= 9 {
        FrameContent::Unknown(format!(
            "Response(chip={:02x}{:02x}, len={})",
            data[2],
            data[3],
            data.len()
        ))
    } else {
        FrameContent::Unknown(format!("Response(len={})", data.len()))
    };

    (content, crc_status)
}

/// Dissected I2C operation
#[derive(Debug)]
pub struct DissectedI2c {
    pub timestamp: f64,
    pub address: u8,
    pub device: I2cDevice,
    pub operation: String,
    pub raw_data: Vec<u8>,
    pub was_naked: bool,
}

/// I2C device contexts for state tracking
#[derive(Debug, Default)]
pub struct I2cContexts {
    /// VOUT_MODE cache for each TPS546 device address
    pub tps546_vout_modes: HashMap<u8, u8>,
}

/// Known I2C devices
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum I2cDevice {
    Emc2101,
    Tps546,
    Unknown,
}

/// Dissect an I2C operation with context tracking
pub fn dissect_i2c_operation_with_context(
    op: &I2cOperation,
    contexts: &mut I2cContexts,
) -> DissectedI2c {
    let device = match op.address {
        0x4C => I2cDevice::Emc2101,
        0x24 => I2cDevice::Tps546,
        _ => I2cDevice::Unknown,
    };

    let operation = if let Some(reg) = op.register {
        let is_read = op.read_data.is_some();

        // Get data directly - PMBus parser already separated command from data
        let data = if is_read {
            op.read_data.as_ref().map(|v| v.as_slice())
        } else {
            op.write_data.as_ref().map(|v| v.as_slice())
        };

        match device {
            I2cDevice::Emc2101 => {
                // For now, keep using EMC2101 formatting until we refactor it too
                emc2101::protocol::format_transaction(reg, data, is_read)
            }
            I2cDevice::Tps546 => {
                // Update VOUT_MODE cache if this is a VOUT_MODE operation
                if reg == pmbus::PmbusCommand::VoutMode.as_u8() {
                    if let Some(data) = data {
                        if data.len() >= 1 && !is_read {
                            contexts.tps546_vout_modes.insert(op.address, data[0]);
                        } else if data.len() >= 1 && is_read {
                            contexts.tps546_vout_modes.insert(op.address, data[0]);
                        }
                    }
                }

                // Format using PMBus value parser
                if let Ok(pmbus_cmd) = pmbus::PmbusCommand::try_from(reg) {
                    let direction = if is_read { "⟶" } else { "⟵" };
                    let op_type = if is_read { "READ" } else { "WRITE" };

                    if let Some(data) = data {
                        let vout_mode = contexts.tps546_vout_modes.get(&op.address).copied();
                        let value = pmbus::parse_pmbus_value(pmbus_cmd, data, vout_mode);
                        format!("{} {} {}={}", direction, op_type, pmbus_cmd, value)
                    } else {
                        // Data-less command
                        if pmbus_cmd == pmbus::PmbusCommand::ClearFaults {
                            format!(
                                "{} {} {} ({})",
                                direction,
                                op_type,
                                pmbus_cmd,
                                pmbus_cmd.description()
                            )
                        } else {
                            format!("{} {} {} (register select)", direction, op_type, pmbus_cmd)
                        }
                    }
                } else {
                    // Unknown command
                    if let Some(data) = data {
                        let direction = if is_read { "⟶" } else { "⟵" };
                        let op_type = if is_read { "READ" } else { "WRITE" };
                        format!("{} {} CMD[0x{:02x}]={:02x?}", direction, op_type, reg, data)
                    } else {
                        format!("⟵ WRITE CMD[0x{:02x}] (unknown command)", reg)
                    }
                }
            }
            I2cDevice::Unknown => {
                if let Some(data) = &op.read_data {
                    format!("⟶ READ [0x{:02x}]={:02x?}", reg, data)
                } else if let Some(data) = &op.write_data {
                    format!("⟵ WRITE [0x{:02x}]={:02x?}", reg, data)
                } else {
                    // Command-only write (no data after register/command byte)
                    format!("⟵ WRITE [0x{:02x}]", reg)
                }
            }
        }
    } else {
        // No register specified, but we can still describe the operation
        if let Some(data) = &op.read_data {
            format!("⟶ READ {:02x?} (no register)", data)
        } else if let Some(data) = &op.write_data {
            format!("⟵ WRITE {:02x?} (no register)", data)
        } else {
            format!("I2C op @ 0x{:02x}", op.address)
        }
    };

    let raw_data = op
        .write_data
        .as_ref()
        .or(op.read_data.as_ref())
        .cloned()
        .unwrap_or_default();

    DissectedI2c {
        timestamp: op.start_time,
        address: op.address,
        device,
        operation,
        raw_data,
        was_naked: op.was_naked,
    }
}
