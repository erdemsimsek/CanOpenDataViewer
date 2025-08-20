pub mod sdo;
pub mod connect;

// Re-export the main types for easy access
pub use sdo::{
    SdoRequest, SdoResponse, SdoResponseData, SdoDataType, SdoError,
    create_sdo_request_frame, parse_sdo_response, parse_payload, get_abort_code_description
};

pub use connect::{CANopenConnection, CANopenNodeHandle, CANopenError};

