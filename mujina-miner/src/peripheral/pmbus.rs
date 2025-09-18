//! PMBus Protocol Support
//!
//! This module provides generic PMBus protocol definitions and utilities
//! that can be used by PMBus-compliant device drivers.
//!
//! PMBus is a variant of SMBus with extensions for power management.
//! Specification: <https://pmbus.org/specification-documents/>

use bitflags::bitflags;
use std::fmt;
use thiserror::Error;

// ============================================================================
// Constants
// ============================================================================

/// Default VOUT_MODE for devices that don't specify
/// Uses exponent -9 (2^-9 ≈ 0.00195V resolution) which provides
/// millivolt-level precision suitable for most power supplies
const DEFAULT_VOUT_MODE: u8 = 0x17; // -9 in 5-bit two's complement

// ============================================================================
// PMBus Commands
// ============================================================================

/// Macro to define PMBus commands with metadata in one place
macro_rules! define_pmbus_commands {
    (
        $(
            $variant:ident = $value:literal,
            $name:literal,
            $desc:literal
        ),* $(,)?
    ) => {
        /// PMBus standard command codes
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
        #[repr(u8)]
        pub enum PmbusCommand {
            $(
                $variant = $value,
            )*
        }

        impl PmbusCommand {
            /// Command metadata: (value, name, description)
            const METADATA: &'static [(u8, &'static str, &'static str)] = &[
                $(
                    ($value, $name, $desc),
                )*
            ];

            /// Get the command name as a string
            pub fn name(&self) -> &'static str {
                let value = self.as_u8();
                Self::METADATA
                    .iter()
                    .find(|(v, _, _)| *v == value)
                    .map(|(_, name, _)| *name)
                    .unwrap_or("UNKNOWN")
            }

            /// Get command description
            pub fn description(&self) -> &'static str {
                let value = self.as_u8();
                Self::METADATA
                    .iter()
                    .find(|(v, _, _)| *v == value)
                    .map(|(_, _, desc)| *desc)
                    .unwrap_or("unknown command")
            }

            /// Convert to u8 command code
            pub fn as_u8(self) -> u8 {
                self as u8
            }
        }

        impl fmt::Display for PmbusCommand {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.name())
            }
        }

        impl TryFrom<u8> for PmbusCommand {
            type Error = PMBusError;

            fn try_from(value: u8) -> Result<Self, Self::Error> {
                match value {
                    $(
                        $value => Ok(Self::$variant),
                    )*
                    _ => Err(PMBusError::CommandNotSupported),
                }
            }
        }

        impl From<PmbusCommand> for u8 {
            fn from(cmd: PmbusCommand) -> Self {
                cmd.as_u8()
            }
        }
    };
}

// Define all PMBus commands in one place
define_pmbus_commands! {
    Page = 0x00, "PAGE", "select page for multi-rail devices",
    Operation = 0x01, "OPERATION", "turn on/off control",
    OnOffConfig = 0x02, "ON_OFF_CONFIG", "on/off configuration",
    ClearFaults = 0x03, "CLEAR_FAULTS", "clears all fault status bits",
    Phase = 0x04, "PHASE", "phase selection",
    Capability = 0x19, "CAPABILITY", "device capability",
    VoutMode = 0x20, "VOUT_MODE", "output voltage data format",
    VoutCommand = 0x21, "VOUT_COMMAND", "commanded output voltage",
    VoutMax = 0x24, "VOUT_MAX", "maximum output voltage",
    VoutMarginHigh = 0x25, "VOUT_MARGIN_HIGH", "margin high voltage",
    VoutMarginLow = 0x26, "VOUT_MARGIN_LOW", "margin low voltage",
    VoutScaleLoop = 0x29, "VOUT_SCALE_LOOP", "scale loop compensation",
    VoutMin = 0x2B, "VOUT_MIN", "minimum output voltage",
    FrequencySwitch = 0x33, "FREQUENCY_SWITCH", "switching frequency",
    VinOn = 0x35, "VIN_ON", "input turn-on voltage",
    VinOff = 0x36, "VIN_OFF", "input turn-off voltage",
    Interleave = 0x37, "INTERLEAVE", "interleave configuration",
    VoutOvFaultLimit = 0x40, "VOUT_OV_FAULT_LIMIT", "output overvoltage fault limit",
    VoutOvWarnLimit = 0x42, "VOUT_OV_WARN_LIMIT", "output overvoltage warning limit",
    VoutUvWarnLimit = 0x43, "VOUT_UV_WARN_LIMIT", "output undervoltage warning limit",
    VoutUvFaultLimit = 0x44, "VOUT_UV_FAULT_LIMIT", "output undervoltage fault limit",
    IoutOcFaultLimit = 0x46, "IOUT_OC_FAULT_LIMIT", "output overcurrent fault limit",
    IoutOcFaultResponse = 0x47, "IOUT_OC_FAULT_RESPONSE", "output overcurrent fault response",
    IoutOcWarnLimit = 0x4A, "IOUT_OC_WARN_LIMIT", "output overcurrent warning limit",
    OtFaultLimit = 0x4F, "OT_FAULT_LIMIT", "overtemperature fault limit",
    OtFaultResponse = 0x50, "OT_FAULT_RESPONSE", "overtemperature fault response",
    OtWarnLimit = 0x51, "OT_WARN_LIMIT", "overtemperature warning limit",
    VinOvFaultLimit = 0x55, "VIN_OV_FAULT_LIMIT", "input overvoltage fault limit",
    VinOvFaultResponse = 0x56, "VIN_OV_FAULT_RESPONSE", "input overvoltage fault response",
    VinUvWarnLimit = 0x58, "VIN_UV_WARN_LIMIT", "input undervoltage warning limit",
    TonDelay = 0x60, "TON_DELAY", "turn-on delay",
    TonRise = 0x61, "TON_RISE", "turn-on rise time",
    TonMaxFaultLimit = 0x62, "TON_MAX_FAULT_LIMIT", "maximum turn-on time limit",
    TonMaxFaultResponse = 0x63, "TON_MAX_FAULT_RESPONSE", "maximum turn-on fault response",
    ToffDelay = 0x64, "TOFF_DELAY", "turn-off delay",
    ToffFall = 0x65, "TOFF_FALL", "turn-off fall time",
    StatusWord = 0x79, "STATUS_WORD", "status summary",
    StatusVout = 0x7A, "STATUS_VOUT", "output voltage status",
    StatusIout = 0x7B, "STATUS_IOUT", "output current status",
    StatusInput = 0x7C, "STATUS_INPUT", "input status",
    StatusTemperature = 0x7D, "STATUS_TEMPERATURE", "temperature status",
    StatusCml = 0x7E, "STATUS_CML", "communication/logic/memory status",
    StatusOther = 0x7F, "STATUS_OTHER", "other status",
    StatusMfrSpecific = 0x80, "STATUS_MFR_SPECIFIC", "manufacturer specific status",
    ReadVin = 0x88, "READ_VIN", "input voltage",
    ReadVout = 0x8B, "READ_VOUT", "output voltage",
    ReadIout = 0x8C, "READ_IOUT", "output current",
    ReadTemperature1 = 0x8D, "READ_TEMPERATURE_1", "temperature 1",
    MfrId = 0x99, "MFR_ID", "manufacturer ID",
    MfrModel = 0x9A, "MFR_MODEL", "manufacturer model",
    MfrRevision = 0x9B, "MFR_REVISION", "manufacturer revision",
    IcDeviceId = 0xAD, "IC_DEVICE_ID", "IC device ID",
    CompensationConfig = 0xB1, "COMPENSATION_CONFIG", "compensation configuration",
    SyncConfig = 0xE4, "SYNC_CONFIG", "synchronization configuration",
    StackConfig = 0xEC, "STACK_CONFIG", "stacking configuration",
    // TPS546-specific commands - TODO: move to device-specific module
    MiscOptions = 0xED, "MISC_OPTIONS", "miscellaneous options",
    PinDetectOverride = 0xEE, "PIN_DETECT_OVERRIDE", "pin detect override",
    SlaveAddress = 0xEF, "SLAVE_ADDRESS", "slave address",
    NvmChecksum = 0xF0, "NVM_CHECKSUM", "NVM checksum",
    SimulateFault = 0xF1, "SIMULATE_FAULT", "simulate fault",
    FusionId0 = 0xFC, "FUSION_ID0", "fusion ID 0",
    FusionId1 = 0xFD, "FUSION_ID1", "fusion ID 1",
}

// [Rest of file remains unchanged from line 324 onward...]

// ============================================================================
// Typed PMBus Values
// ============================================================================

/// Typed PMBus voltage value with units
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct PmbusVoltage(f32);

impl PmbusVoltage {
    pub fn new(value: f32) -> Self {
        Self(value)
    }

    pub fn from_linear11(value: u16) -> Self {
        Self(linear11::to_float_unsigned(value))
    }

    pub fn from_linear16(value: u16, vout_mode: u8) -> Self {
        Self(linear16::to_float(value, vout_mode))
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl From<f32> for PmbusVoltage {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<PmbusVoltage> for f32 {
    fn from(voltage: PmbusVoltage) -> Self {
        voltage.0
    }
}

impl fmt::Display for PmbusVoltage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}V", self.0)
    }
}

/// Typed PMBus current value with units
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct PmbusCurrent(f32);

impl PmbusCurrent {
    pub fn new(value: f32) -> Self {
        Self(value)
    }

    pub fn from_linear11(value: u16) -> Self {
        Self(linear11::to_float(value))
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl From<f32> for PmbusCurrent {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<PmbusCurrent> for f32 {
    fn from(current: PmbusCurrent) -> Self {
        current.0
    }
}

impl fmt::Display for PmbusCurrent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.3}A", self.0)
    }
}

/// Typed PMBus temperature value with units
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct PmbusTemperature(f32);

impl PmbusTemperature {
    pub fn new(value: f32) -> Self {
        Self(value)
    }

    pub fn from_linear11(value: u16) -> Self {
        Self(linear11::to_float(value))
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl From<f32> for PmbusTemperature {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<PmbusTemperature> for f32 {
    fn from(temp: PmbusTemperature) -> Self {
        temp.0
    }
}

impl fmt::Display for PmbusTemperature {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}°C", self.0)
    }
}

/// Typed PMBus frequency value with units
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct PmbusFrequency(f32);

impl PmbusFrequency {
    pub fn new(value: f32) -> Self {
        Self(value)
    }

    pub fn from_linear11(value: u16) -> Self {
        Self(linear11::to_float(value))
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl From<f32> for PmbusFrequency {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<PmbusFrequency> for f32 {
    fn from(freq: PmbusFrequency) -> Self {
        freq.0
    }
}

impl fmt::Display for PmbusFrequency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.0}kHz", self.0)
    }
}

/// Typed PMBus time value with units
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default)]
pub struct PmbusTime(f32);

impl PmbusTime {
    pub fn new(value: f32) -> Self {
        Self(value)
    }

    pub fn from_linear11(value: u16) -> Self {
        Self(linear11::to_float(value))
    }

    pub fn value(&self) -> f32 {
        self.0
    }
}

impl From<f32> for PmbusTime {
    fn from(value: f32) -> Self {
        Self(value)
    }
}

impl From<PmbusTime> for f32 {
    fn from(time: PmbusTime) -> Self {
        time.0
    }
}

impl fmt::Display for PmbusTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.1}ms", self.0)
    }
}

/// PMBus value enumeration for polymorphic value handling
#[derive(Debug, Clone)]
pub enum PmbusValue {
    Voltage(PmbusVoltage),
    Current(PmbusCurrent),
    Temperature(PmbusTemperature),
    Frequency(PmbusFrequency),
    Time(PmbusTime),
    StatusWord(u16, Vec<&'static str>),
    StatusByte(u8, Vec<&'static str>),
    FaultResponse(u8, String),
    String(String),
    Raw(Vec<u8>),
}

impl fmt::Display for PmbusValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Voltage(v) => v.fmt(f),
            Self::Current(c) => c.fmt(f),
            Self::Temperature(t) => t.fmt(f),
            Self::Frequency(freq) => freq.fmt(f),
            Self::Time(t) => t.fmt(f),
            Self::StatusWord(value, flags) => {
                if flags.is_empty() {
                    write!(f, "0x{:04x}", value)
                } else {
                    write!(f, "0x{:04x} (", value)?;
                    for (i, flag) in flags.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", flag)?;
                    }
                    write!(f, ")")
                }
            }
            Self::StatusByte(value, flags) => {
                if flags.is_empty() {
                    write!(f, "0x{:02x}", value)
                } else {
                    write!(f, "0x{:02x} (", value)?;
                    for (i, flag) in flags.iter().enumerate() {
                        if i > 0 {
                            write!(f, ", ")?;
                        }
                        write!(f, "{}", flag)?;
                    }
                    write!(f, ")")
                }
            }
            Self::FaultResponse(value, desc) => write!(f, "0x{:02x} ({})", value, desc),
            Self::String(s) => write!(f, "{}", s),
            Self::Raw(bytes) => write!(f, "{:02x?}", bytes),
        }
    }
}

// ============================================================================
// Helper Functions
// ============================================================================

/// Parse a little-endian u16 from data
fn parse_u16_le(data: &[u8]) -> Option<u16> {
    data.get(..2)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_le_bytes)
}

/// Parse string data from PMBus block read format
fn parse_string_data(data: &[u8]) -> String {
    // PMBus block reads have length byte first
    let text_bytes = if data.len() > 1 && data[0] as usize == data.len() - 1 {
        &data[1..]
    } else {
        data
    };

    match std::str::from_utf8(text_bytes) {
        Ok(s) => s.trim_end_matches('\0').to_string(),
        Err(_) => format!("{:02x?}", text_bytes),
    }
}

// ============================================================================
// Value Parsing
// ============================================================================

/// Parse PMBus value from raw data based on command type
pub fn parse_pmbus_value(cmd: PmbusCommand, data: &[u8], vout_mode: Option<u8>) -> PmbusValue {
    if data.is_empty() {
        return PmbusValue::Raw(vec![]);
    }

    // Try specialized parsers first
    if let Some(value) = parse_voltage_value(cmd, data, vout_mode) {
        return value;
    }
    if let Some(value) = parse_current_value(cmd, data) {
        return value;
    }
    if let Some(value) = parse_temperature_value(cmd, data) {
        return value;
    }
    if let Some(value) = parse_time_value(cmd, data) {
        return value;
    }
    if let Some(value) = parse_status_value(cmd, data) {
        return value;
    }
    if let Some(value) = parse_string_value(cmd, data) {
        return value;
    }

    // Default: raw bytes
    PmbusValue::Raw(data.to_vec())
}

fn parse_voltage_value(
    cmd: PmbusCommand,
    data: &[u8],
    vout_mode: Option<u8>,
) -> Option<PmbusValue> {
    use PmbusCommand::*;

    match cmd {
        ReadVin | VinOn | VinOff | VinOvFaultLimit | VinUvWarnLimit => {
            parse_u16_le(data).map(|v| PmbusValue::Voltage(PmbusVoltage::from_linear11(v)))
        }
        ReadVout | VoutCommand | VoutMax | VoutMarginHigh | VoutMarginLow | VoutScaleLoop
        | VoutMin | VoutOvFaultLimit | VoutOvWarnLimit | VoutUvWarnLimit | VoutUvFaultLimit => {
            parse_u16_le(data).map(|v| {
                let mode = vout_mode.unwrap_or(DEFAULT_VOUT_MODE);
                PmbusValue::Voltage(PmbusVoltage::from_linear16(v, mode))
            })
        }
        _ => None,
    }
}

fn parse_current_value(cmd: PmbusCommand, data: &[u8]) -> Option<PmbusValue> {
    use PmbusCommand::*;

    match cmd {
        ReadIout | IoutOcFaultLimit | IoutOcWarnLimit => {
            parse_u16_le(data).map(|v| PmbusValue::Current(PmbusCurrent::from_linear11(v)))
        }
        _ => None,
    }
}

fn parse_temperature_value(cmd: PmbusCommand, data: &[u8]) -> Option<PmbusValue> {
    use PmbusCommand::*;

    match cmd {
        ReadTemperature1 | OtFaultLimit | OtWarnLimit => {
            parse_u16_le(data).map(|v| PmbusValue::Temperature(PmbusTemperature::from_linear11(v)))
        }
        _ => None,
    }
}

fn parse_time_value(cmd: PmbusCommand, data: &[u8]) -> Option<PmbusValue> {
    use PmbusCommand::*;

    match cmd {
        TonDelay | TonRise | TonMaxFaultLimit | ToffDelay | ToffFall => {
            parse_u16_le(data).map(|v| PmbusValue::Time(PmbusTime::from_linear11(v)))
        }
        _ => None,
    }
}

fn parse_status_value(cmd: PmbusCommand, data: &[u8]) -> Option<PmbusValue> {
    use PmbusCommand::*;

    match cmd {
        StatusWord => parse_u16_le(data).map(|v| {
            let flags = StatusDecoder::decode_status_word(v);
            PmbusValue::StatusWord(v, flags)
        }),
        StatusVout if !data.is_empty() => {
            let flags = StatusDecoder::decode_status_vout(data[0]);
            Some(PmbusValue::StatusByte(data[0], flags))
        }
        StatusIout if !data.is_empty() => {
            let flags = StatusDecoder::decode_status_iout(data[0]);
            Some(PmbusValue::StatusByte(data[0], flags))
        }
        StatusInput if !data.is_empty() => {
            let flags = StatusDecoder::decode_status_input(data[0]);
            Some(PmbusValue::StatusByte(data[0], flags))
        }
        StatusTemperature if !data.is_empty() => {
            let flags = StatusDecoder::decode_status_temp(data[0]);
            Some(PmbusValue::StatusByte(data[0], flags))
        }
        StatusCml if !data.is_empty() => {
            let flags = StatusDecoder::decode_status_cml(data[0]);
            Some(PmbusValue::StatusByte(data[0], flags))
        }
        IoutOcFaultResponse | OtFaultResponse | VinOvFaultResponse | TonMaxFaultResponse
            if !data.is_empty() =>
        {
            let desc = StatusDecoder::decode_fault_response(data[0]);
            Some(PmbusValue::FaultResponse(data[0], desc))
        }
        FrequencySwitch => {
            parse_u16_le(data).map(|v| PmbusValue::Frequency(PmbusFrequency::from_linear11(v)))
        }
        _ => None,
    }
}

fn parse_string_value(cmd: PmbusCommand, data: &[u8]) -> Option<PmbusValue> {
    use PmbusCommand::*;

    match cmd {
        MfrId | MfrModel | MfrRevision => Some(PmbusValue::String(parse_string_data(data))),
        _ => None,
    }
}

// ============================================================================
// Status Register Bits
// ============================================================================

bitflags! {
    /// PMBus STATUS_WORD (0x79) register flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusWord: u16 {
        const VOUT = 0x8000;
        const IOUT = 0x4000;
        const INPUT = 0x2000;
        const MFR = 0x1000;
        const PGOOD = 0x0800;
        const FANS = 0x0400;
        const OTHER = 0x0200;
        const UNKNOWN = 0x0100;
        const BUSY = 0x0080;
        const OFF = 0x0040;
        const VOUT_OV = 0x0020;
        const IOUT_OC = 0x0010;
        const VIN_UV = 0x0008;
        const TEMP = 0x0004;
        const CML = 0x0002;
        const NONE = 0x0001;
    }
}

bitflags! {
    /// PMBus STATUS_VOUT (0x7A) register flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusVout: u8 {
        const VOUT_OV_FAULT = 0x80;
        const VOUT_OV_WARN = 0x40;
        const VOUT_UV_WARN = 0x20;
        const VOUT_UV_FAULT = 0x10;
        const VOUT_MAX = 0x08;
        const TON_MAX_FAULT = 0x02;
        const VOUT_MIN = 0x01;
    }
}

bitflags! {
    /// PMBus STATUS_IOUT (0x7B) register flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusIout: u8 {
        const IOUT_OC_FAULT = 0x80;
        const IOUT_OC_LV_FAULT = 0x40;
        const IOUT_OC_WARN = 0x20;
        const IOUT_UC_FAULT = 0x10;
        const CURR_SHARE_FAULT = 0x08;
        const IN_PWR_LIM = 0x04;
        const POUT_OP_FAULT = 0x02;
        const POUT_OP_WARN = 0x01;
    }
}

bitflags! {
    /// PMBus STATUS_INPUT (0x7C) register flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusInput: u8 {
        const VIN_OV_FAULT = 0x80;
        const VIN_OV_WARN = 0x40;
        const VIN_UV_WARN = 0x20;
        const VIN_UV_FAULT = 0x10;
        const UNIT_OFF_VIN_LOW = 0x08;
        const IIN_OC_FAULT = 0x04;
        const IIN_OC_WARN = 0x02;
        const PIN_OP_WARN = 0x01;
    }
}

bitflags! {
    /// PMBus STATUS_TEMPERATURE (0x7D) register flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusTemperature: u8 {
        const OT_FAULT = 0x80;
        const OT_WARN = 0x40;
        const UT_WARN = 0x20;
        const UT_FAULT = 0x10;
    }
}

bitflags! {
    /// PMBus STATUS_CML (0x7E) register flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct StatusCml: u8 {
        const INVALID_CMD = 0x80;
        const INVALID_DATA = 0x40;
        const PEC_FAULT = 0x20;
        const MEMORY_FAULT = 0x10;
        const PROCESSOR_FAULT = 0x08;
        const OTHER_COMM_FAULT = 0x02;
        const OTHER_MEM_LOGIC = 0x01;
    }
}

/// PMBus OPERATION (0x01) command values
/// Note: These are not bitflags, but discrete command values
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum Operation {
    OffImmediate = 0x00,
    MarginLow = 0x18,
    MarginHigh = 0x28,
    SoftOff = 0x40,
    On = 0x80,
    OnMarginLow = 0x98,
    OnMarginHigh = 0xA8,
}

impl Operation {
    pub fn as_u8(self) -> u8 {
        self as u8
    }
}

impl From<Operation> for u8 {
    fn from(op: Operation) -> Self {
        op as u8
    }
}

impl TryFrom<u8> for Operation {
    type Error = PMBusError;

    fn try_from(value: u8) -> Result<Self, Self::Error> {
        match value {
            0x00 => Ok(Self::OffImmediate),
            0x18 => Ok(Self::MarginLow),
            0x28 => Ok(Self::MarginHigh),
            0x40 => Ok(Self::SoftOff),
            0x80 => Ok(Self::On),
            0x98 => Ok(Self::OnMarginLow),
            0xA8 => Ok(Self::OnMarginHigh),
            _ => Err(PMBusError::InvalidDataFormat),
        }
    }
}

bitflags! {
    /// PMBus ON_OFF_CONFIG (0x02) register flags
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct OnOffConfig: u8 {
        const PU = 0x10;
        const CMD = 0x08;
        const CP = 0x04;
        const POLARITY = 0x02;
        const DELAY = 0x01;
    }
}

// ============================================================================
// Status Decoder
// ============================================================================

/// Macro to generate status decoder methods
macro_rules! decode_status_flags {
    ($flags:expr => {
        $($flag:expr => $desc:literal),* $(,)?
    }) => {{
        let mut desc = Vec::new();
        $(if $flags.contains($flag) { desc.push($desc); })*
        desc
    }};
}

pub struct StatusDecoder;

impl StatusDecoder {
    pub fn decode_status_word(status: u16) -> Vec<&'static str> {
        let flags = StatusWord::from_bits_truncate(status);
        let mut desc = decode_status_flags!(flags => {
            StatusWord::VOUT => "VOUT fault/warning",
            StatusWord::IOUT => "IOUT fault/warning",
            StatusWord::INPUT => "INPUT fault/warning",
            StatusWord::MFR => "MFR specific",
            StatusWord::PGOOD => "PGOOD",
            StatusWord::FANS => "FAN fault/warning",
            StatusWord::OTHER => "OTHER",
            StatusWord::UNKNOWN => "UNKNOWN",
            StatusWord::BUSY => "BUSY",
            StatusWord::OFF => "OFF",
            StatusWord::VOUT_OV => "VOUT_OV fault",
            StatusWord::IOUT_OC => "IOUT_OC fault",
            StatusWord::VIN_UV => "VIN_UV fault",
            StatusWord::TEMP => "TEMP fault/warning",
            StatusWord::CML => "CML fault",
        });

        if flags.contains(StatusWord::NONE) && desc.is_empty() {
            desc.push("NONE_OF_THE_ABOVE");
        }
        desc
    }

    pub fn decode_status_vout(status: u8) -> Vec<&'static str> {
        let flags = StatusVout::from_bits_truncate(status);
        decode_status_flags!(flags => {
            StatusVout::VOUT_OV_FAULT => "OV fault",
            StatusVout::VOUT_OV_WARN => "OV warning",
            StatusVout::VOUT_UV_WARN => "UV warning",
            StatusVout::VOUT_UV_FAULT => "UV fault",
            StatusVout::VOUT_MAX => "at MAX",
            StatusVout::TON_MAX_FAULT => "failed to start",
            StatusVout::VOUT_MIN => "at MIN",
        })
    }

    pub fn decode_status_iout(status: u8) -> Vec<&'static str> {
        let flags = StatusIout::from_bits_truncate(status);
        decode_status_flags!(flags => {
            StatusIout::IOUT_OC_FAULT => "OC fault",
            StatusIout::IOUT_OC_LV_FAULT => "OC+LV fault",
            StatusIout::IOUT_OC_WARN => "OC warning",
            StatusIout::IOUT_UC_FAULT => "UC fault",
            StatusIout::CURR_SHARE_FAULT => "current share fault",
            StatusIout::IN_PWR_LIM => "power limiting",
            StatusIout::POUT_OP_FAULT => "overpower fault",
            StatusIout::POUT_OP_WARN => "overpower warning",
        })
    }

    pub fn decode_status_input(status: u8) -> Vec<&'static str> {
        let flags = StatusInput::from_bits_truncate(status);
        decode_status_flags!(flags => {
            StatusInput::VIN_OV_FAULT => "VIN OV fault",
            StatusInput::VIN_OV_WARN => "VIN OV warning",
            StatusInput::VIN_UV_WARN => "VIN UV warning",
            StatusInput::VIN_UV_FAULT => "VIN UV fault",
            StatusInput::UNIT_OFF_VIN_LOW => "off due to low VIN",
            StatusInput::IIN_OC_FAULT => "IIN OC fault",
            StatusInput::IIN_OC_WARN => "IIN OC warning",
            StatusInput::PIN_OP_WARN => "input overpower warning",
        })
    }

    pub fn decode_status_temp(status: u8) -> Vec<&'static str> {
        let flags = StatusTemperature::from_bits_truncate(status);
        decode_status_flags!(flags => {
            StatusTemperature::OT_FAULT => "overtemp fault",
            StatusTemperature::OT_WARN => "overtemp warning",
            StatusTemperature::UT_WARN => "undertemp warning",
            StatusTemperature::UT_FAULT => "undertemp fault",
        })
    }

    pub fn decode_status_cml(status: u8) -> Vec<&'static str> {
        let flags = StatusCml::from_bits_truncate(status);
        decode_status_flags!(flags => {
            StatusCml::INVALID_CMD => "invalid command",
            StatusCml::INVALID_DATA => "invalid data",
            StatusCml::PEC_FAULT => "PEC error",
            StatusCml::MEMORY_FAULT => "memory fault",
            StatusCml::PROCESSOR_FAULT => "processor fault",
            StatusCml::OTHER_COMM_FAULT => "other comm fault",
            StatusCml::OTHER_MEM_LOGIC => "other mem/logic fault",
        })
    }

    pub fn decode_fault_response(response: u8) -> String {
        let response_type = (response >> 5) & 0x07;
        let retry_count = (response >> 3) & 0x03;
        let delay_time = response & 0x07;

        let response_desc = match response_type {
            0b000 => "ignore fault",
            0b001 => "shutdown, retry indefinitely",
            0b010 => "shutdown, no retry",
            0b011 => "shutdown with retries",
            0b100 => "continue, retry indefinitely",
            0b101 => "continue, no retry",
            0b110 => "continue with retries",
            0b111 => "shutdown with delay and retries",
            _ => "unknown",
        };

        let retries_desc = match retry_count {
            0b00 => "no retries",
            0b01 => "1 retry",
            0b10 => "2 retries",
            0b11 => match response_type {
                0b001 | 0b100 => "infinite retries",
                _ => "3 retries",
            },
            _ => "unknown",
        };

        let delay_desc = match delay_time {
            0b000 => "0ms",
            0b001 => "22.7ms",
            0b010 => "45.4ms",
            0b011 => "91ms",
            0b100 => "182ms",
            0b101 => "364ms",
            0b110 => "728ms",
            0b111 => "1456ms",
            _ => "unknown",
        };

        match response {
            0x00 => "ignore fault".to_string(),
            0xC0 => "shutdown immediately, no retries".to_string(),
            0xFF => "infinite retries, wait for recovery".to_string(),
            _ => {
                if retry_count == 0 || response_type == 0b010 || response_type == 0b101 {
                    response_desc.to_string()
                } else {
                    format!("{}, {}, {} delay", response_desc, retries_desc, delay_desc)
                }
            }
        }
    }
}

// ============================================================================
// Linear Format Conversion Modules
// ============================================================================

/// SLINEAR11 data format conversion
pub mod linear11 {
    const EXPONENT_SHIFT: u8 = 11;
    const MANTISSA_MASK: u16 = 0x07FF;
    const MANTISSA_SIGN_BIT: u16 = 0x0400;
    const EXPONENT_SIGN_BIT: u8 = 0x10;
    const EXPONENT_SIGN_EXTEND: u8 = 0xE0;

    /// Convert SLINEAR11 format to floating point
    pub fn to_float(value: u16) -> f32 {
        let exp_raw = ((value >> EXPONENT_SHIFT) & 0x1F) as i8;
        let exponent = if exp_raw & (EXPONENT_SIGN_BIT as i8) != 0 {
            (exp_raw as u8 | EXPONENT_SIGN_EXTEND) as i8 as i32
        } else {
            exp_raw as i32
        };

        let mant_raw = (value & MANTISSA_MASK) as i16;
        let mantissa = if mant_raw & MANTISSA_SIGN_BIT as i16 != 0 {
            ((mant_raw as u16 | 0xF800) as i16) as i32
        } else {
            mant_raw as i32
        };

        mantissa as f32 * 2.0_f32.powi(exponent)
    }

    /// Convert ULINEAR11 format to floating point (unsigned mantissa)
    pub fn to_float_unsigned(value: u16) -> f32 {
        let exp_raw = ((value >> EXPONENT_SHIFT) & 0x1F) as i8;
        let exponent = if exp_raw & (EXPONENT_SIGN_BIT as i8) != 0 {
            (exp_raw as u8 | EXPONENT_SIGN_EXTEND) as i8 as i32
        } else {
            exp_raw as i32
        };

        let mantissa = (value & MANTISSA_MASK) as u32;
        mantissa as f32 * 2.0_f32.powi(exponent)
    }

    /// Convert floating point to SLINEAR11 format
    pub fn from_float(value: f32) -> u16 {
        if value == 0.0 {
            return 0;
        }

        let mut best_exp = 0i8;
        let mut best_error = f32::MAX;

        for exp in -16i8..=15 {
            let mantissa_f = value / 2.0_f32.powi(exp as i32);

            if (-1024.0..1024.0).contains(&mantissa_f) {
                let mantissa = mantissa_f.round() as i32;
                let reconstructed = mantissa as f32 * 2.0_f32.powi(exp as i32);
                let error = (reconstructed - value).abs();

                if error < best_error {
                    best_error = error;
                    best_exp = exp;
                }
            }
        }

        let mantissa = (value / 2.0_f32.powi(best_exp as i32)).round() as i32;
        let exp_bits = (best_exp as u16) & 0x1F;
        let mant_bits = (mantissa as u16) & MANTISSA_MASK;

        (exp_bits << EXPONENT_SHIFT) | mant_bits
    }
}

/// ULINEAR16 data format conversion
pub mod linear16 {
    use super::PMBusError;

    const EXPONENT_MASK: u8 = 0x1F;
    const EXPONENT_SIGN_BIT: u8 = 0x10;
    const EXPONENT_SIGN_EXTEND: u8 = 0xE0;

    /// Convert ULINEAR16 format to floating point
    pub fn to_float(value: u16, vout_mode: u8) -> f32 {
        let exp_raw = (vout_mode & EXPONENT_MASK) as i8;
        let exponent = if exp_raw & (EXPONENT_SIGN_BIT as i8) != 0 {
            (exp_raw as u8 | EXPONENT_SIGN_EXTEND) as i8 as i32
        } else {
            exp_raw as i32
        };

        value as f32 * 2.0_f32.powi(exponent)
    }

    /// Convert floating point to ULINEAR16 format
    pub fn from_float(value: f32, vout_mode: u8) -> Result<u16, PMBusError> {
        let exp_raw = (vout_mode & EXPONENT_MASK) as i8;
        let exponent = if exp_raw & (EXPONENT_SIGN_BIT as i8) != 0 {
            (exp_raw as u8 | EXPONENT_SIGN_EXTEND) as i8 as i32
        } else {
            exp_raw as i32
        };

        let mantissa = (value / 2.0_f32.powi(exponent)).round() as u32;
        if mantissa > 0xFFFF {
            return Err(PMBusError::ValueOutOfRange);
        }

        Ok(mantissa as u16)
    }
}

// ============================================================================
// Error Types
// ============================================================================

#[derive(Error, Debug)]
pub enum PMBusError {
    #[error("Invalid data format")]
    InvalidDataFormat,
    #[error("Value out of range")]
    ValueOutOfRange,
    #[error("Command not supported")]
    CommandNotSupported,
    #[error("Communication error")]
    CommunicationError,
}
