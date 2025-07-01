//! Physical transport layer for board connections.
//!
//! This module handles low-level physical connections to mining boards,
//! including USB serial, PCIe, and other future transports. It provides
//! discovery, enumeration, and raw byte stream access without any
//! protocol knowledge.

// TODO: Implement transport traits and USB serial support