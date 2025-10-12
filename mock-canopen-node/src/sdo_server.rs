//! SDO Server implementation for responding to SDO upload requests

use socketcan::{CanFrame, StandardId, EmbeddedFrame};
use canopen_common::{SdoDataType, SdoCommand};
use crate::object_dictionary::ObjectDictionary;

pub struct SdoServer {
    node_id: u8,
    object_dict: ObjectDictionary,
    request_cob_id: u16,  // 0x600 + node_id
    response_cob_id: u16, // 0x580 + node_id
}

impl SdoServer {
    pub fn new(node_id: u8, object_dict: ObjectDictionary) -> Self {
        Self {
            node_id,
            object_dict,
            request_cob_id: 0x600 + node_id as u16,
            response_cob_id: 0x580 + node_id as u16,
        }
    }

    /// Handle an incoming CAN frame
    /// Returns Some(response_frame) if this was an SDO request for us
    pub fn handle_frame(&mut self, frame: &CanFrame) -> Option<CanFrame> {
        // Check if this frame is an SDO request for our node
        let frame_id = match frame.id() {
            socketcan::Id::Standard(std_id) => std_id.as_raw(),
            socketcan::Id::Extended(_) => return None, // We don't handle extended IDs
        };

        if frame_id != self.request_cob_id {
            return None; // Not for us
        }

        let data = frame.data();
        if data.len() < 4 {
            return None; // Invalid frame
        }

        // Parse SDO request
        let command = data[0];
        let index = u16::from_le_bytes([data[1], data[2]]);
        let subindex = data[3];

        // Check if this is an SDO upload request (0x40)
        if command == 0x40 {
            println!("📥 SDO Upload Request: Index=0x{:04X}, SubIndex=0x{:02X}", index, subindex);
            return self.create_sdo_response(index, subindex);
        }

        None
    }

    /// Create an SDO response frame
    fn create_sdo_response(&self, index: u16, subindex: u8) -> Option<CanFrame> {
        // Look up the object in the dictionary
        match self.object_dict.get(index, subindex) {
            Some((data, data_type)) => {
                let response_frame = self.create_expedited_response(index, subindex, &data)?;

                // Log the response
                let value_str = format_data(&data, &data_type);
                println!("📤 SDO Response: Value={} (type={:?})", value_str, data_type);

                Some(response_frame)
            }
            None => {
                // Object doesn't exist - send abort
                println!("⚠  Object not found: 0x{:04X}:0x{:02X}", index, subindex);
                self.create_abort_response(index, subindex, 0x06020000) // Object does not exist
            }
        }
    }

    /// Create an expedited SDO upload response (for data ≤ 4 bytes)
    fn create_expedited_response(&self, index: u16, subindex: u8, data: &[u8]) -> Option<CanFrame> {
        if data.len() > 4 {
            // Data too large for expedited transfer
            return self.create_abort_response(index, subindex, 0x05040001); // Command specifier not valid
        }

        let response_id = StandardId::new(self.response_cob_id)?;
        let mut frame_data = [0u8; 8];

        // Calculate command byte:
        // - Bits 7-5: 010 (expedited upload response)
        // - Bit 1: 1 (size is indicated)
        // - Bit 0: 1 (expedited transfer)
        // - Bits 3-2: n (number of bytes that do NOT contain data)
        let n = 4 - data.len();
        let command = 0x43 | ((n as u8) << 2);

        frame_data[0] = command;
        frame_data[1] = (index & 0xFF) as u8;
        frame_data[2] = ((index >> 8) & 0xFF) as u8;
        frame_data[3] = subindex;

        // Copy data (little-endian, pad with zeros)
        for (i, &byte) in data.iter().enumerate() {
            if i < 4 {
                frame_data[4 + i] = byte;
            }
        }

        CanFrame::new(response_id, &frame_data)
    }

    /// Create an SDO abort response
    fn create_abort_response(&self, index: u16, subindex: u8, abort_code: u32) -> Option<CanFrame> {
        let response_id = StandardId::new(self.response_cob_id)?;
        let mut frame_data = [0u8; 8];

        frame_data[0] = 0x80; // Abort transfer
        frame_data[1] = (index & 0xFF) as u8;
        frame_data[2] = ((index >> 8) & 0xFF) as u8;
        frame_data[3] = subindex;

        // Abort code in bytes 4-7 (little-endian)
        let abort_bytes = abort_code.to_le_bytes();
        frame_data[4..8].copy_from_slice(&abort_bytes);

        CanFrame::new(response_id, &frame_data)
    }
}

/// Format data for display based on its type
fn format_data(data: &[u8], data_type: &SdoDataType) -> String {
    match data_type {
        SdoDataType::UInt8 if data.len() >= 1 => {
            format!("{}", data[0])
        }
        SdoDataType::UInt16 if data.len() >= 2 => {
            format!("{}", u16::from_le_bytes([data[0], data[1]]))
        }
        SdoDataType::UInt32 if data.len() >= 4 => {
            format!("{}", u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }
        SdoDataType::Int8 if data.len() >= 1 => {
            format!("{}", data[0] as i8)
        }
        SdoDataType::Int16 if data.len() >= 2 => {
            format!("{}", i16::from_le_bytes([data[0], data[1]]))
        }
        SdoDataType::Int32 if data.len() >= 4 => {
            format!("{}", i32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }
        SdoDataType::Real32 if data.len() >= 4 => {
            format!("{:.2}", f32::from_le_bytes([data[0], data[1], data[2], data[3]]))
        }
        SdoDataType::VisibleString => {
            String::from_utf8_lossy(data).to_string()
        }
        _ => {
            format!("{:02X?}", data)
        }
    }
}
