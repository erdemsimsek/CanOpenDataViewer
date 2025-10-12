//! # Mock CANopen Node
//!
//! A simple CANopen node simulator for testing the CANopen Data Viewer application.
//!
//! This mock node:
//! - Listens for SDO upload requests on the CAN bus
//! - Responds with simulated data from a configurable object dictionary
//! - Supports multiple data types (integers, floats, strings)
//! - Can simulate dynamic changing values (like sensor readings)
//!
//! ## Usage
//!
//! ```bash
//! # Start the mock node on vcan0 with node ID 4
//! cargo run -p mock-canopen-node -- --interface vcan0 --node-id 4
//! ```

mod object_dictionary;
mod sdo_server;

use socketcan::{CanSocket, Socket};
use std::time::Duration;
use object_dictionary::ObjectDictionary;
use sdo_server::SdoServer;

fn main() {
    // Parse command line arguments (simplified for now)
    let args: Vec<String> = std::env::args().collect();

    let interface = args.get(1)
        .and_then(|arg| if arg == "--interface" { args.get(2) } else { None })
        .map(|s| s.as_str())
        .unwrap_or("vcan0");

    let node_id = args.get(3)
        .and_then(|arg| if arg == "--node-id" { args.get(4) } else { None })
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(4);

    println!("🤖 Mock CANopen Node Starting...");
    println!("   Interface: {}", interface);
    println!("   Node ID: {}", node_id);
    println!();

    // Open CAN socket
    let socket = match CanSocket::open(interface) {
        Ok(sock) => {
            println!("✓ CAN socket opened successfully");
            sock
        }
        Err(e) => {
            eprintln!("✗ Failed to open CAN socket: {}", e);
            eprintln!("\nTroubleshooting:");
            eprintln!("  1. Create virtual CAN interface:");
            eprintln!("     sudo modprobe vcan");
            eprintln!("     sudo ip link add dev vcan0 type vcan");
            eprintln!("     sudo ip link set up vcan0");
            eprintln!("  2. Check interface exists: ip link show");
            std::process::exit(1);
        }
    };

    // Set read timeout to avoid busy-waiting
    socket.set_read_timeout(Duration::from_millis(10))
        .expect("Failed to set socket timeout");

    // Create object dictionary with test data
    let mut object_dict = ObjectDictionary::new();
    object_dict.add_test_objects();

    println!("✓ Object dictionary loaded with {} objects", object_dict.len());
    println!("\n📋 Available SDO Objects:");
    object_dict.print_summary();
    println!();

    // Create SDO server
    let mut sdo_server = SdoServer::new(node_id, object_dict);

    println!("🚀 Mock node is running!");
    println!("   Waiting for SDO requests on COB-ID 0x{:03X}...", 0x600 + node_id as u16);
    println!("   Press Ctrl+C to stop\n");

    // Main loop: listen for CAN frames and respond to SDO requests
    loop {
        match socket.read_frame() {
            Ok(frame) => {
                // Let the SDO server handle the frame
                if let Some(response_frame) = sdo_server.handle_frame(&frame) {
                    // Send the response
                    if let Err(e) = socket.write_frame(&response_frame) {
                        eprintln!("⚠ Failed to send response: {}", e);
                    }
                }
            }
            Err(err) => {
                // Timeout or no data - this is normal, just continue
                if err.kind() != std::io::ErrorKind::WouldBlock
                   && err.kind() != std::io::ErrorKind::TimedOut {
                    eprintln!("⚠ CAN read error: {}", err);
                }
            }
        }
    }
}
