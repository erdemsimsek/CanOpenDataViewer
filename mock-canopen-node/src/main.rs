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

use socketcan::{CanSocket, Socket, CanFrame, StandardId, EmbeddedFrame};
use std::time::{Duration, Instant};
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
    object_dict.add_test_objects_for_node(node_id);

    println!("✓ Object dictionary loaded with {} objects", object_dict.len());
    println!("\n📋 Available SDO Objects:");
    object_dict.print_summary();
    println!();

    // Create SDO server
    let mut sdo_server = SdoServer::new(node_id, object_dict);

    println!("🚀 Mock node is running!");
    println!("   Waiting for SDO requests on COB-ID 0x{:03X}...", 0x600 + node_id as u16);
    println!("   Broadcasting TPDO1 on COB-ID 0x{:03X} every 100ms", 0x180 + node_id as u16);
    println!("   TPDO1 contains: CabinTemperature (0x2000:01), OutsideTemperature (0x2000:02)");
    println!("   Press Ctrl+C to stop\n");

    // TPDO broadcasting state
    let mut last_tpdo_time = Instant::now();
    let tpdo_interval = Duration::from_millis(100);

    // Main loop: listen for CAN frames and respond to SDO requests
    loop {
        // Handle incoming SDO requests
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

        // Broadcast TPDO periodically
        if last_tpdo_time.elapsed() >= tpdo_interval {
            // Read current values from Object Dictionary
            // TPDO1 mapping: 0x2000:01 (CabinTemperature, Real32), 0x2000:02 (OutsideTemperature, Real32)
            let cabin_temp = sdo_server.object_dict().get(0x2000, 0x01);
            let outside_temp = sdo_server.object_dict().get(0x2000, 0x02);

            if let (Some((cabin_data, _)), Some((outside_data, _))) = (cabin_temp, outside_temp) {
                // Create TPDO frame
                // TPDO1 COB-ID = 0x180 + node_id
                let tpdo_cob_id = 0x180 + node_id as u16;

                if let Some(std_id) = StandardId::new(tpdo_cob_id) {
                    let mut data = [0u8; 8];

                    // Pack data according to TPDO mapping
                    // Bytes 0-3: CabinTemperature (Real32, little-endian)
                    data[0..4].copy_from_slice(&cabin_data[..4]);
                    // Bytes 4-7: OutsideTemperature (Real32, little-endian)
                    data[4..8].copy_from_slice(&outside_data[..4]);

                    if let Some(frame) = CanFrame::new(std_id, &data) {
                        if let Err(e) = socket.write_frame(&frame) {
                            eprintln!("⚠ Failed to send TPDO: {}", e);
                        } else {
                            // Decode for display
                            let cabin_f32 = f32::from_le_bytes([cabin_data[0], cabin_data[1], cabin_data[2], cabin_data[3]]);
                            let outside_f32 = f32::from_le_bytes([outside_data[0], outside_data[1], outside_data[2], outside_data[3]]);
                            print!("📤 TPDO1: CabinTemp={:.2}°C, OutsideTemp={:.2}°C\r", cabin_f32, outside_f32);
                            use std::io::Write;
                            std::io::stdout().flush().ok();
                        }
                    }
                }
            }

            last_tpdo_time = Instant::now();
        }
    }
}
