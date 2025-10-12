// sdo.rs - Updated for the new connection architecture
use socketcan::{CanFrame, StandardId};
use socketcan::EmbeddedFrame as Frame;
use std::error::Error;
use std::fmt;

/// SDO Command Specifiers
#[derive(Debug, Clone, Copy)]
#[repr(u8)]
pub enum SdoCommand {
    /// Initiate domain upload (read)
    InitiateUploadRequest = 0x40,
    /// Expedited upload response (1-4 bytes)
    ExpeditedUploadResponse = 0x43,
    /// Segmented upload response
    SegmentedUploadResponse = 0x41,
    /// Upload segment request
    UploadSegmentRequest = 0x60,
    /// Upload segment response
    UploadSegmentResponse = 0x00,
    /// Abort transfer
    AbortTransfer = 0x80,
}

impl SdoCommand {
    pub(crate) fn is_expedited_response(value: u8) -> bool {
        (value & 0xE0) == 0x40 && (value & 0x02) != 0
    }
}

/// SDO Data Types
#[derive(Debug, Clone)]
pub enum SdoDataType {
    UInt8,
    UInt16,
    UInt32,
    Int8,
    Int16,
    Int32,
    Real32,
    VisibleString,
    OctetString,
}

impl SdoDataType {
    pub fn from_eds_type(eds_type: &str) -> Option<Self> {
        match eds_type {
            "0x0001" | "1" => Some(Self::UInt8),
            "0x0002" | "2" => Some(Self::Int8),
            "0x0003" | "3" => Some(Self::UInt16),
            "0x0004" | "4" => Some(Self::Int16),
            "0x0005" | "5" => Some(Self::UInt32),
            "0x0006" | "6" => Some(Self::Int32),
            "0x0008" | "8" => Some(Self::Real32),
            "0x0009" | "9" => Some(Self::VisibleString),
            "0x000A" | "10" => Some(Self::OctetString),
            _ => None,
        }
    }
}

/// SDO Request structure
#[derive(Debug, Clone)]
pub struct SdoRequest {
    pub node_id: u8,
    pub index: u16,
    pub subindex: u8,
    pub expected_type: SdoDataType,
}

/// SDO Response data
#[derive(Debug, Clone)]
pub enum SdoResponseData {
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Real32(f32),
    String(String),
    Bytes(Vec<u8>),
    Error { code: u32, info: String },
}

impl fmt::Display for SdoResponseData {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UInt8(v) => write!(f, "{}", v),
            Self::UInt16(v) => write!(f, "{}", v),
            Self::UInt32(v) => write!(f, "{}", v),
            Self::Int8(v) => write!(f, "{}", v),
            Self::Int16(v) => write!(f, "{}", v),
            Self::Int32(v) => write!(f, "{}", v),
            Self::Real32(v) => write!(f, "{}", v),
            Self::String(v) => write!(f, "{}", v),
            Self::Bytes(v) => write!(f, "{:02X?}", v),
            Self::Error { code, info } => write!(f, "Error 0x{:08X}: {}", code, info),
        }
    }
}

/// SDO Response structure
#[derive(Debug, Clone)]
pub struct SdoResponse {
    pub node_id: u8,
    pub index: u16,
    pub subindex: u8,
    pub data: SdoResponseData,
    pub raw_data: Vec<u8>,
}

/// Custom error type for SDO operations
#[derive(Debug)]
pub enum SdoError {
    SocketError(String),
    Timeout,
    InvalidResponse(String),
    AbortTransfer { code: u32, info: String },
    ParseError(String),
}

impl fmt::Display for SdoError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SocketError(msg) => write!(f, "Socket error: {}", msg),
            Self::Timeout => write!(f, "SDO request timeout"),
            Self::InvalidResponse(msg) => write!(f, "Invalid response: {}", msg),
            Self::AbortTransfer { code, info } => write!(f, "SDO abort 0x{:08X}: {}", code, info),
            Self::ParseError(msg) => write!(f, "Parse error: {}", msg),
        }
    }
}

impl Error for SdoError {}

/// Create an SDO request CAN frame
pub fn create_sdo_request_frame(request: &SdoRequest) -> Result<CanFrame, SdoError> {
    let request_id = StandardId::new(0x600 + request.node_id as u16)
        .ok_or_else(|| SdoError::InvalidResponse("Invalid CAN ID".to_string()))?;

    let mut data = [0u8; 8];

    // SDO command specifier (initiate upload request)
    data[0] = SdoCommand::InitiateUploadRequest as u8;

    // Index in little endian
    data[1] = (request.index & 0xFF) as u8;
    data[2] = ((request.index >> 8) & 0xFF) as u8;

    // Subindex
    data[3] = request.subindex;

    // Remaining bytes are zero (padding)
    data[4..8].fill(0);

    CanFrame::new(request_id, &data)
        .ok_or_else(|| SdoError::InvalidResponse("Failed to create CAN frame".to_string()))
}

/// Parse SDO response frame
pub fn parse_sdo_response(frame: CanFrame, request: &SdoRequest) -> Result<SdoResponse, SdoError> {
    let data = frame.data();
    if data.len() < 4 {
        return Err(SdoError::InvalidResponse("Frame too short".to_string()));
    }

    let command = data[0];
    let index = u16::from_le_bytes([data[1], data[2]]);
    let subindex = data[3];

    // Verify this response matches our request
    if index != request.index || subindex != request.subindex {
        return Err(SdoError::InvalidResponse(format!(
            "Response mismatch: expected index=0x{:04X}, subindex={}, got index=0x{:04X}, subindex={}",
            request.index, request.subindex, index, subindex
        )));
    }

    // Check for abort transfer
    if command == SdoCommand::AbortTransfer as u8 {
        let abort_code = if data.len() >= 8 {
            u32::from_le_bytes([data[4], data[5], data[6], data[7]])
        } else {
            0
        };

        let error_info = get_abort_code_description(abort_code);
        return Err(SdoError::AbortTransfer {
            code: abort_code,
            info: error_info,
        });
    }

    // Parse expedited response (most common case)
    if SdoCommand::is_expedited_response(command) {
        let n = (command & 0x0C) >> 2; // Number of bytes that do NOT contain data
        let data_size = 4 - n as usize;  // Actual data size

        let payload = &data[4..4 + data_size];
        let response_data = parse_payload(payload, &request.expected_type)?;

        return Ok(SdoResponse {
            node_id: request.node_id,
            index: request.index,
            subindex: request.subindex,
            data: response_data,
            raw_data: data.to_vec(),
        });
    }

    // Handle segmented transfer (for larger data)
    Err(SdoError::InvalidResponse(format!(
        "Segmented SDO transfer not implemented yet (command=0x{:02X})", command
    )))
}

/// Parse payload data based on expected type
pub fn parse_payload(payload: &[u8], data_type: &SdoDataType) -> Result<SdoResponseData, SdoError> {
    match data_type {
        SdoDataType::UInt8 => {
            if payload.len() >= 1 {
                Ok(SdoResponseData::UInt8(payload[0]))
            } else {
                Err(SdoError::ParseError("Insufficient data for UInt8".to_string()))
            }
        }
        SdoDataType::UInt16 => {
            if payload.len() >= 2 {
                let value = u16::from_le_bytes([payload[0], payload[1]]);
                Ok(SdoResponseData::UInt16(value))
            } else {
                Err(SdoError::ParseError("Insufficient data for UInt16".to_string()))
            }
        }
        SdoDataType::UInt32 => {
            if payload.len() >= 4 {
                let value = u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(SdoResponseData::UInt32(value))
            } else {
                Err(SdoError::ParseError("Insufficient data for UInt32".to_string()))
            }
        }
        SdoDataType::Int8 => {
            if payload.len() >= 1 {
                Ok(SdoResponseData::Int8(payload[0] as i8))
            } else {
                Err(SdoError::ParseError("Insufficient data for Int8".to_string()))
            }
        }
        SdoDataType::Int16 => {
            if payload.len() >= 2 {
                let value = i16::from_le_bytes([payload[0], payload[1]]);
                Ok(SdoResponseData::Int16(value))
            } else {
                Err(SdoError::ParseError("Insufficient data for Int16".to_string()))
            }
        }
        SdoDataType::Int32 => {
            if payload.len() >= 4 {
                let value = i32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(SdoResponseData::Int32(value))
            } else {
                Err(SdoError::ParseError("Insufficient data for Int32".to_string()))
            }
        }
        SdoDataType::Real32 => {
            if payload.len() >= 4 {
                let value = f32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]);
                Ok(SdoResponseData::Real32(value))
            } else {
                Err(SdoError::ParseError("Insufficient data for Real32".to_string()))
            }
        }
        SdoDataType::VisibleString => {
            let string = String::from_utf8_lossy(payload).trim_end_matches('\0').to_string();
            Ok(SdoResponseData::String(string))
        }
        SdoDataType::OctetString => {
            Ok(SdoResponseData::Bytes(payload.to_vec()))
        }
    }
}

/// Get human-readable description of SDO abort codes
pub fn get_abort_code_description(code: u32) -> String {
    match code {
        0x05030000 => "Toggle bit not alternated".to_string(),
        0x05040000 => "SDO protocol timed out".to_string(),
        0x05040001 => "Client/server command specifier not valid or unknown".to_string(),
        0x05040005 => "Out of memory".to_string(),
        0x06010000 => "Unsupported access to an object".to_string(),
        0x06010001 => "Attempt to read a write only object".to_string(),
        0x06010002 => "Attempt to write a read only object".to_string(),
        0x06020000 => "Object does not exist in the object dictionary".to_string(),
        0x06040041 => "Object cannot be mapped to the PDO".to_string(),
        0x06040042 => "The number and length of the objects to be mapped would exceed PDO length".to_string(),
        0x06040043 => "General parameter incompatibility reason".to_string(),
        0x06040047 => "General internal incompatibility in the device".to_string(),
        0x06060000 => "Access failed due to a hardware error".to_string(),
        0x06070010 => "Data type does not match, length of service parameter does not match".to_string(),
        0x06070012 => "Data type does not match, length of service parameter too high".to_string(),
        0x06070013 => "Data type does not match, length of service parameter too low".to_string(),
        0x06090011 => "Sub-index does not exist".to_string(),
        0x06090030 => "Value range of parameter exceeded (only for write access)".to_string(),
        0x06090031 => "Value of parameter written too high".to_string(),
        0x06090032 => "Value of parameter written too low".to_string(),
        0x06090036 => "Maximum value is less than minimum value".to_string(),
        0x08000000 => "General error".to_string(),
        0x08000020 => "Data cannot be transferred or stored to the application".to_string(),
        0x08000021 => "Data cannot be transferred or stored to the application because of local control".to_string(),
        0x08000022 => "Data cannot be transferred or stored to the application because of the present device state".to_string(),
        _ => format!("Unknown abort code: 0x{:08X}", code),
    }
}
