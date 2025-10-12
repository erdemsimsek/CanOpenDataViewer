// Connection management is still local to the viewer
pub mod connect;

// SDO protocol is now in the common library
// Re-export from canopen-common for backwards compatibility
pub use canopen_common::{
    SdoRequest, SdoResponse, SdoResponseData, SdoDataType, SdoError,
    create_sdo_request_frame, parse_sdo_response, parse_payload,
    get_abort_code_description, SdoCommand
};

pub use connect::{CANopenConnection, CANopenNodeHandle, CANopenError};

