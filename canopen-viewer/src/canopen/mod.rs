// Connection management is still local to the viewer
pub mod connect;

// SDO protocol is now in the common library
// Re-export from canopen-common for backwards compatibility
pub use canopen_common::{
    SdoRequest, SdoDataType
};

pub use connect::{CANopenConnection, CANopenNodeHandle};

