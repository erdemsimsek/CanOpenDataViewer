// connect.rs
use socketcan::{CanSocket, Socket, CanFrame, StandardId, EmbeddedFrame};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, oneshot, RwLock};
use tokio::task::JoinHandle;
use std::error::Error;
use std::fmt;

use crate::canopen::sdo::{SdoRequest, SdoResponse, SdoError, create_sdo_request_frame, parse_sdo_response};

#[derive(Debug)]
pub enum CANopenError {
    SocketError(String),
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
    AddNode {
        node_id: u8,
        response_tx: oneshot::Sender<Result<(), CANopenError>>,
    },
    RemoveNode {
        node_id: u8,
        response_tx: oneshot::Sender<Result<(), CANopenError>>,
    },
    // Future: PDO configuration, NMT commands, etc.
}

/// Represents a pending SDO request
struct PendingSdoRequest {
    request: SdoRequest,
    response_tx: oneshot::Sender<Result<SdoResponse, SdoError>>,
    timestamp: std::time::Instant,
}

/// Per-node state management
struct NodeState {
    node_id: u8,
    // Queue of pending SDO requests (FIFO)
    pending_requests: std::collections::VecDeque<PendingSdoRequest>,
    // Currently active request (if any)
    active_request: Option<PendingSdoRequest>,
    // Node-specific timeout
    timeout: Duration,
}

impl NodeState {
    fn new(node_id: u8, timeout: Duration) -> Self {
        Self {
            node_id,
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

    /// Get the node ID for this handle
    pub fn node_id(&self) -> u8 {
        self.node_id
    }

    // Future methods:
    // pub async fn configure_tpdo(&self, config: TpdoConfig) -> Result<(), CANopenError>
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
                                request,
                                response_tx,
                                timestamp: std::time::Instant::now(),
                            };

                            node_state.queue_request(pending_request);

                            // Try to start the request immediately if no active request
                            if let Some(active_request) = node_state.start_next_request() {
                                send_sdo_request(&socket, &active_request.request).await;
                            }
                        } else {
                            let _ = response_tx.send(Err(SdoError::InvalidResponse(
                                format!("Node {} not connected", node_id)
                            )));
                        }
                    }

                    None => break, // Channel closed
                }
            }

            // Handle incoming CAN frames
            frame = frame_rx.recv() => {
                if let Some(frame) = frame {
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
                    send_sdo_request(&socket, &active_request.request).await;
                }
            }
        }
    }
}

async fn send_sdo_request(socket: &Arc<Mutex<CanSocket>>, request: &SdoRequest) {
    let request_id = StandardId::new(0x600 + request.node_id as u16);
    if request_id.is_none() {
        return; // Invalid CAN ID
    }
    let request_id = request_id.unwrap();

    let mut data = [0u8; 8];
    data[0] = 0x40; // SDO upload request
    data[1] = (request.index & 0xFF) as u8;
    data[2] = ((request.index >> 8) & 0xFF) as u8;
    data[3] = request.subindex;

    if let Some(frame) = CanFrame::new(request_id, &data) {
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
                // Parse the response and send it back
                let response = parse_sdo_response(frame, &completed_request.request);
                let _ = completed_request.response_tx.send(response);
            }
        }
    }

    // Future: Handle PDO frames, NMT frames, etc.
}

async fn check_timeouts(nodes: &mut HashMap<u8, NodeState>) {
    for node_state in nodes.values_mut() {
        if let Some(timed_out_request) = node_state.check_timeout() {
            let _ = timed_out_request.response_tx.send(Err(SdoError::Timeout));
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