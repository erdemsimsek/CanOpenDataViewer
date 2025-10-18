// connect.rs
use socketcan::{CanSocket, Socket, CanFrame, EmbeddedFrame};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot};
use tokio::task::JoinHandle;
use std::error::Error;
use std::fmt;

use canopen_common::{SdoRequest, SdoResponse, SdoError, SdoWriteRequest,
                     parse_sdo_response, parse_sdo_write_response};

#[derive(Debug)]
pub enum CANopenError {
    SocketError(String),
    #[allow(dead_code)]  // Reserved for future use
    NodeNotConnected(u8),
    RequestFailed(String),
}

impl fmt::Display for CANopenError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SocketError(msg) => write!(f, "Socket error: {}", msg),
            Self::NodeNotConnected(node_id) => write!(f, "Node {} not connected", node_id),
            Self::RequestFailed(msg) => write!(f, "Request failed: {}", msg),
        }
    }
}

impl Error for CANopenError {}

impl From<SdoError> for CANopenError {
    fn from(error: SdoError) -> Self {
        Self::RequestFailed(error.to_string())
    }
}

/// Internal message types for the connection manager
#[derive(Debug)]
enum ConnectionMessage {
    SdoRequest {
        node_id: u8,
        request: SdoRequest,
        response_tx: oneshot::Sender<Result<SdoResponse, SdoError>>,
    },
    SdoWriteRequest {
        node_id: u8,
        request: SdoWriteRequest,
        response_tx: oneshot::Sender<Result<(), SdoError>>,
    },
    AddNode {
        node_id: u8,
        response_tx: oneshot::Sender<Result<(), CANopenError>>,
    },
    #[allow(dead_code)]  // Reserved for future cleanup functionality
    RemoveNode {
        node_id: u8,
        response_tx: oneshot::Sender<Result<(), CANopenError>>,
    },
    SubscribeRawFrames {
        response_tx: oneshot::Sender<mpsc::UnboundedReceiver<CanFrame>>,
    },
}

/// Represents the type of SDO operation
enum SdoOperation {
    Read {
        request: SdoRequest,
        response_tx: oneshot::Sender<Result<SdoResponse, SdoError>>,
    },
    Write {
        request: SdoWriteRequest,
        response_tx: oneshot::Sender<Result<(), SdoError>>,
    },
}

/// Represents a pending SDO request (read or write)
struct PendingSdoRequest {
    operation: SdoOperation,
    timestamp: std::time::Instant,
}

/// Per-node state management
struct NodeState {
    // Queue of pending SDO requests (FIFO)
    pending_requests: std::collections::VecDeque<PendingSdoRequest>,
    // Currently active request (if any)
    active_request: Option<PendingSdoRequest>,
    // Node-specific timeout
    timeout: Duration,
}

impl NodeState {
    fn new(_node_id: u8, timeout: Duration) -> Self {
        Self {
            pending_requests: std::collections::VecDeque::new(),
            active_request: None,
            timeout,
        }
    }

    fn queue_request(&mut self, request: PendingSdoRequest) {
        self.pending_requests.push_back(request);
    }

    fn start_next_request(&mut self) -> Option<&PendingSdoRequest> {
        if self.active_request.is_none() {
            self.active_request = self.pending_requests.pop_front();
        }
        self.active_request.as_ref()
    }

    fn complete_active_request(&mut self) -> Option<PendingSdoRequest> {
        self.active_request.take()
    }

    fn check_timeout(&mut self) -> Option<PendingSdoRequest> {
        if let Some(ref active) = self.active_request {
            if active.timestamp.elapsed() > self.timeout {
                return self.complete_active_request();
            }
        }
        None
    }
}

/// Main CANopen connection handle
pub struct CANopenConnection {
    command_tx: mpsc::UnboundedSender<ConnectionMessage>,
    _background_task: JoinHandle<()>,
}

impl Clone for CANopenConnection {
    fn clone(&self) -> Self {
        Self {
            command_tx: self.command_tx.clone(),
            _background_task: tokio::spawn(async {}), // Create a dummy task for the clone
        }
    }
}

impl CANopenConnection {
    /// Create a new CANopen connection on the specified interface
    pub async fn new(interface: &str, default_timeout: Duration) -> Result<Self, CANopenError> {
        let socket = CanSocket::open(interface)
            .map_err(|e| CANopenError::SocketError(e.to_string()))?;

        // Set non-blocking mode for the socket
        socket.set_nonblocking(true)
            .map_err(|e| CANopenError::SocketError(e.to_string()))?;

        let (command_tx, command_rx) = mpsc::unbounded_channel();

        let background_task = tokio::spawn(connection_manager_task(
            socket,
            command_rx,
            default_timeout,
        ));

        Ok(Self {
            command_tx,
            _background_task: background_task,
        })
    }

    /// Add a node to the connection (enables communication with this node)
    pub async fn add_node(&self, node_id: u8) -> Result<CANopenNodeHandle, CANopenError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.command_tx
            .send(ConnectionMessage::AddNode { node_id, response_tx })
            .map_err(|_| CANopenError::RequestFailed("Connection manager died".to_string()))?;

        response_rx
            .await
            .map_err(|_| CANopenError::RequestFailed("Failed to get response".to_string()))??;

        Ok(CANopenNodeHandle {
            node_id,
            command_tx: self.command_tx.clone(),
        })
    }

    /// Subscribe to raw CAN frames (for TPDO reception)
    pub async fn subscribe_raw_frames(&self) -> Result<mpsc::UnboundedReceiver<CanFrame>, CANopenError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.command_tx
            .send(ConnectionMessage::SubscribeRawFrames { response_tx })
            .map_err(|_| CANopenError::RequestFailed("Connection manager died".to_string()))?;

        response_rx
            .await
            .map_err(|_| CANopenError::RequestFailed("Failed to get response".to_string()))
    }
}

/// TPDO Mapping Entry - defines one object to map into a TPDO
#[derive(Debug, Clone)]
pub struct TpdoMapping {
    pub index: u16,
    pub sub_index: u8,
    pub bit_length: u8,  // 8, 16, 32, etc.
}

/// TPDO Configuration Parameters
#[derive(Debug, Clone)]
pub struct TpdoConfigParams {
    pub tpdo_number: u8,           // 1-4 typically (maps to 0x1800-0x1803 and 0x1A00-0x1A03)
    pub cob_id: u16,               // COB-ID for this TPDO (e.g., 0x180 + node_id for TPDO1)
    pub transmission_type: u8,     // 0xFE = event-driven, 0xFF = device profile specific, 1-240 = sync-based
    pub inhibit_time_100us: u16,   // Minimum time between transmissions (in 100μs units)
    pub event_timer_ms: u16,       // Periodic transmission timer (in ms, 0 = disabled)
    pub mappings: Vec<TpdoMapping>, // Objects to map into this TPDO
}

/// Handle for communicating with a specific CANopen node
#[derive(Clone)]
pub struct CANopenNodeHandle {
    node_id: u8,
    command_tx: mpsc::UnboundedSender<ConnectionMessage>,
}

impl CANopenNodeHandle {
    /// Send an SDO read request to this node
    pub async fn sdo_read(&self, request: SdoRequest) -> Result<SdoResponse, CANopenError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.command_tx
            .send(ConnectionMessage::SdoRequest {
                node_id: self.node_id,
                request,
                response_tx,
            })
            .map_err(|_| CANopenError::RequestFailed("Connection manager died".to_string()))?;

        response_rx
            .await
            .map_err(|_| CANopenError::RequestFailed("Failed to get response".to_string()))?
            .map_err(CANopenError::from)
    }

    /// Send an SDO write request to this node
    pub async fn sdo_write(&self, request: SdoWriteRequest) -> Result<(), CANopenError> {
        let (response_tx, response_rx) = oneshot::channel();

        self.command_tx
            .send(ConnectionMessage::SdoWriteRequest {
                node_id: self.node_id,
                request,
                response_tx,
            })
            .map_err(|_| CANopenError::RequestFailed("Connection manager died".to_string()))?;

        response_rx
            .await
            .map_err(|_| CANopenError::RequestFailed("Failed to get response".to_string()))?
            .map_err(CANopenError::from)
    }

    /// Configure a TPDO on this node via SDO writes
    pub async fn configure_tpdo(&self, config: TpdoConfigParams) -> Result<(), CANopenError> {
        if config.tpdo_number < 1 || config.tpdo_number > 4 {
            return Err(CANopenError::RequestFailed(
                "TPDO number must be 1-4".to_string()
            ));
        }

        if config.mappings.len() > 8 {
            return Err(CANopenError::RequestFailed(
                "Maximum 8 objects can be mapped to a TPDO".to_string()
            ));
        }

        // Calculate object dictionary indices
        let comm_param_index = 0x1800 + (config.tpdo_number - 1) as u16;  // 0x1800-0x1803
        let mapping_param_index = 0x1A00 + (config.tpdo_number - 1) as u16; // 0x1A00-0x1A03

        println!("Configuring TPDO {} on node {}", config.tpdo_number, self.node_id);

        // Step 1: Disable the TPDO (set bit 31 of COB-ID)
        println!("  Step 1: Disabling TPDO...");
        let disabled_cob_id = config.cob_id as u32 | 0x8000_0000;
        self.sdo_write(SdoWriteRequest {
            node_id: self.node_id,
            index: comm_param_index,
            subindex: 1,  // COB-ID subindex
            data: disabled_cob_id.to_le_bytes().to_vec(),
        }).await?;

        // Step 2: Clear the mapping count (set to 0)
        println!("  Step 2: Clearing mapping count...");
        self.sdo_write(SdoWriteRequest {
            node_id: self.node_id,
            index: mapping_param_index,
            subindex: 0,  // Number of mapped objects
            data: vec![0],
        }).await?;

        // Step 3: Write new mappings (subindex 1-8)
        println!("  Step 3: Writing {} mappings...", config.mappings.len());
        for (i, mapping) in config.mappings.iter().enumerate() {
            // Mapping format: Index (16 bits) | Subindex (8 bits) | Bit length (8 bits)
            let mapping_value = ((mapping.index as u32) << 16)
                              | ((mapping.sub_index as u32) << 8)
                              | (mapping.bit_length as u32);

            println!("    Mapping {}: 0x{:04X}:{:02X} ({} bits) = 0x{:08X}",
                     i + 1, mapping.index, mapping.sub_index, mapping.bit_length, mapping_value);

            self.sdo_write(SdoWriteRequest {
                node_id: self.node_id,
                index: mapping_param_index,
                subindex: (i + 1) as u8,  // Mapping subindex 1-8
                data: mapping_value.to_le_bytes().to_vec(),
            }).await?;
        }

        // Step 4: Update the mapping count
        println!("  Step 4: Setting mapping count to {}...", config.mappings.len());
        self.sdo_write(SdoWriteRequest {
            node_id: self.node_id,
            index: mapping_param_index,
            subindex: 0,
            data: vec![config.mappings.len() as u8],
        }).await?;

        // Step 5: Configure TPDO communication parameters
        println!("  Step 5: Setting transmission type to 0x{:02X}...", config.transmission_type);
        self.sdo_write(SdoWriteRequest {
            node_id: self.node_id,
            index: comm_param_index,
            subindex: 2,  // Transmission type
            data: vec![config.transmission_type],
        }).await?;

        // Inhibit time (optional, 0 = no restriction)
        if config.inhibit_time_100us > 0 {
            println!("  Step 6: Setting inhibit time to {} * 100μs...", config.inhibit_time_100us);
            self.sdo_write(SdoWriteRequest {
                node_id: self.node_id,
                index: comm_param_index,
                subindex: 3,  // Inhibit time
                data: config.inhibit_time_100us.to_le_bytes().to_vec(),
            }).await?;
        }

        // Event timer (optional, 0 = disabled)
        if config.event_timer_ms > 0 {
            println!("  Step 7: Setting event timer to {} ms...", config.event_timer_ms);
            self.sdo_write(SdoWriteRequest {
                node_id: self.node_id,
                index: comm_param_index,
                subindex: 5,  // Event timer
                data: config.event_timer_ms.to_le_bytes().to_vec(),
            }).await?;
        }

        // Step 6: Enable the TPDO (clear bit 31 of COB-ID)
        println!("  Final Step: Enabling TPDO with COB-ID 0x{:03X}...", config.cob_id);
        self.sdo_write(SdoWriteRequest {
            node_id: self.node_id,
            index: comm_param_index,
            subindex: 1,  // COB-ID subindex
            data: (config.cob_id as u32).to_le_bytes().to_vec(),
        }).await?;

        println!("✓ TPDO {} configured successfully!", config.tpdo_number);
        Ok(())
    }

    /// Get the node ID for this handle
    pub fn node_id(&self) -> u8 {
        self.node_id
    }

    // Future methods:
    // pub async fn configure_rpdo(&self, config: RpdoConfig) -> Result<(), CANopenError>
    // pub async fn send_nmt_command(&self, command: NmtCommand) -> Result<(), CANopenError>
}

/// Background task that manages all CANopen communication
async fn connection_manager_task(
    socket: CanSocket,
    mut command_rx: mpsc::UnboundedReceiver<ConnectionMessage>,
    default_timeout: Duration,
) {
    let mut nodes: HashMap<u8, NodeState> = HashMap::new();
    let socket = Arc::new(Mutex::new(socket));
    let mut raw_frame_subscribers: Vec<mpsc::UnboundedSender<CanFrame>> = Vec::new();

    // Spawn the CAN frame reader task
    let socket_clone = socket.clone();
    let (frame_tx, mut frame_rx) = mpsc::unbounded_channel::<CanFrame>();

    tokio::spawn(async move {
        loop {
            let frame = {
                let socket = socket_clone.lock().unwrap();
                socket.read_frame()
            };

            match frame {
                Ok(frame) => {
                    if frame_tx.send(frame).is_err() {
                        break; // Channel closed
                    }
                }
                Err(_) => {
                    // No frame available or error, sleep briefly
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            }
        }
    });

    // Main event loop
    loop {
        tokio::select! {
            // Handle commands from the API
            command = command_rx.recv() => {
                match command {
                    Some(ConnectionMessage::AddNode { node_id, response_tx }) => {
                        nodes.insert(node_id, NodeState::new(node_id, default_timeout));
                        let _ = response_tx.send(Ok(()));
                    }

                    Some(ConnectionMessage::RemoveNode { node_id, response_tx }) => {
                        nodes.remove(&node_id);
                        let _ = response_tx.send(Ok(()));
                    }

                    Some(ConnectionMessage::SdoRequest { node_id, request, response_tx }) => {
                        if let Some(node_state) = nodes.get_mut(&node_id) {
                            let pending_request = PendingSdoRequest {
                                operation: SdoOperation::Read { request, response_tx },
                                timestamp: std::time::Instant::now(),
                            };

                            node_state.queue_request(pending_request);

                            // Try to start the request immediately if no active request
                            if let Some(active_request) = node_state.start_next_request() {
                                send_sdo_operation(&socket, &active_request.operation).await;
                            }
                        } else {
                            let _ = response_tx.send(Err(SdoError::InvalidResponse(
                                format!("Node {} not connected", node_id)
                            )));
                        }
                    }

                    Some(ConnectionMessage::SdoWriteRequest { node_id, request, response_tx }) => {
                        if let Some(node_state) = nodes.get_mut(&node_id) {
                            let pending_request = PendingSdoRequest {
                                operation: SdoOperation::Write { request, response_tx },
                                timestamp: std::time::Instant::now(),
                            };

                            node_state.queue_request(pending_request);

                            // Try to start the request immediately if no active request
                            if let Some(active_request) = node_state.start_next_request() {
                                send_sdo_operation(&socket, &active_request.operation).await;
                            }
                        } else {
                            let _ = response_tx.send(Err(SdoError::InvalidResponse(
                                format!("Node {} not connected", node_id)
                            )));
                        }
                    }

                    Some(ConnectionMessage::SubscribeRawFrames { response_tx }) => {
                        let (tx, rx) = mpsc::unbounded_channel();
                        raw_frame_subscribers.push(tx);
                        let _ = response_tx.send(rx);
                    }

                    None => break, // Channel closed
                }
            }

            // Handle incoming CAN frames
            frame = frame_rx.recv() => {
                if let Some(frame) = frame {
                    // Broadcast frame to raw frame subscribers (for TPDO listeners)
                    raw_frame_subscribers.retain(|subscriber| {
                        subscriber.send(frame.clone()).is_ok()
                    });

                    // Handle SDO responses
                    handle_can_frame(&mut nodes, frame).await;
                }
            }

            // Check for timeouts periodically
            _ = tokio::time::sleep(Duration::from_millis(10)) => {
                check_timeouts(&mut nodes).await;
            }
        }

        // Process any pending requests that can be started
        for node_state in nodes.values_mut() {
            if node_state.active_request.is_none() {
                if let Some(active_request) = node_state.start_next_request() {
                    send_sdo_operation(&socket, &active_request.operation).await;
                }
            }
        }
    }
}

async fn send_sdo_operation(socket: &Arc<Mutex<CanSocket>>, operation: &SdoOperation) {
    use canopen_common::{create_sdo_request_frame, create_sdo_write_frame};

    let frame_result = match operation {
        SdoOperation::Read { request, .. } => {
            create_sdo_request_frame(request)
        }
        SdoOperation::Write { request, .. } => {
            create_sdo_write_frame(request)
        }
    };

    if let Ok(frame) = frame_result {
        let socket = socket.lock().unwrap();
        let _ = socket.write_frame(&frame);
    }
}

async fn handle_can_frame(nodes: &mut HashMap<u8, NodeState>, frame: CanFrame) {
    // Check if this is an SDO response (0x580 + node_id)
    let frame_id = match frame.id() {
        socketcan::Id::Standard(std_id) => std_id.as_raw() as u32,
        socketcan::Id::Extended(_) => return, // We don't handle extended IDs for SDO
    };

    if frame_id >= 0x580 && frame_id <= 0x5FF {
        let node_id = (frame_id - 0x580) as u8;

        if let Some(node_state) = nodes.get_mut(&node_id) {
            if let Some(completed_request) = node_state.complete_active_request() {
                // Parse the response based on operation type
                match completed_request.operation {
                    SdoOperation::Read { request, response_tx } => {
                        let response = parse_sdo_response(frame, &request);
                        let _ = response_tx.send(response);
                    }
                    SdoOperation::Write { request, response_tx } => {
                        let response = parse_sdo_write_response(frame, &request);
                        let _ = response_tx.send(response);
                    }
                }
            }
        }
    }

    // Future: Handle PDO frames, NMT frames, etc.
}

async fn check_timeouts(nodes: &mut HashMap<u8, NodeState>) {
    for node_state in nodes.values_mut() {
        if let Some(timed_out_request) = node_state.check_timeout() {
            // Send timeout error based on operation type
            match timed_out_request.operation {
                SdoOperation::Read { response_tx, .. } => {
                    let _ = response_tx.send(Err(SdoError::Timeout));
                }
                SdoOperation::Write { response_tx, .. } => {
                    let _ = response_tx.send(Err(SdoError::Timeout));
                }
            }
        }
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_multiple_concurrent_requests() {
        // Test that multiple concurrent SDO requests to the same node
        // are properly serialized and don't interfere with each other
    }

    #[tokio::test]
    async fn test_different_nodes_concurrent() {
        // Test that requests to different nodes can run concurrently
    }
}