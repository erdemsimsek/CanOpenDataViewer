//! # CANopen Common Library
//!
//! Shared CANopen protocol implementation used by both the viewer application
//! and the mock CANopen node for testing.
//!
//! This library provides:
//! - SDO (Service Data Object) protocol encoding/decoding
//! - Common data types and error handling
//! - Frame parsing utilities

pub mod sdo;

// Re-export commonly used types for convenience
pub use sdo::{
    SdoRequest, SdoResponse, SdoResponseData, SdoDataType, SdoError,
    create_sdo_request_frame, parse_sdo_response, parse_payload,
    get_abort_code_description, SdoCommand
};
