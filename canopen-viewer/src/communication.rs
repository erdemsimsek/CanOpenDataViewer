use std::sync::mpsc::{Receiver, Sender};
use std::path::PathBuf;
use configparser::ini::Ini;
use std::collections::{BTreeMap, HashMap};
use tokio::task::JoinHandle;
use std::time::Duration;
use chrono::{DateTime, Local};
use socketcan::EmbeddedFrame;
use crate::canopen::{
    CANopenConnection, CANopenNodeHandle,
    SdoRequest, SdoDataType
};


#[derive(Debug, Clone)]
pub struct SdoSubObject {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone)]
pub struct SdoObject {
    pub name: String,
    pub sub_objects: BTreeMap<u8, SdoSubObject>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct SdoAddress {
    pub index: u16,
    pub sub_index: u8,
}

/// Represents a single object mapped into a TPDO
#[derive(Debug, Clone)]
pub struct TpdoMappedObject {
    pub index: u16,
    pub sub_index: u8,
    pub bit_length: u8,
    pub data_type: SdoDataType,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct TpdoConfig {
    pub tpdo_number: u8,
    pub cob_id: u16,
    pub mapped_objects: Vec<TpdoMappedObject>,
}

/// Parsed TPDO data received from CAN bus
#[derive(Debug, Clone)]
pub struct TpdoData {
    pub tpdo_number: u8,
    pub timestamp: DateTime<Local>,
    pub values: Vec<(String, String)>, // (object_name, parsed_value)
}

#[derive(Debug)]
pub enum Command {
    Connect,
    FetchSdos,
    Subscribe {
        address: SdoAddress,
        interval_ms: u64,
        data_type: SdoDataType,
    },
    Unsubscribe(SdoAddress),
    DiscoverTpdos,
    StartTpdoListener(TpdoConfig),
    StopTpdoListener(u8),
}

#[derive(Debug)]
pub enum Update {
    ConnectionStatus(bool),
    ConnectionFailed(String),
    SdoList(BTreeMap<u16, SdoObject>),
    SdoData {
        address: SdoAddress,
        value: String,
    },
    SdoReadError {
        address: SdoAddress,
        error: String,
    },
    TpdoData(TpdoData),
    TpdosDiscovered(Vec<TpdoConfig>),
}

async fn sdo_polling_task(
    address: SdoAddress,
    interval_ms: u64,
    update_tx: Sender<Update>,
    node_handle: CANopenNodeHandle,
    data_type: SdoDataType,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));

    loop {
        interval.tick().await;

        let request = SdoRequest{
            node_id: node_handle.node_id(),
            index: address.index,
            subindex: address.sub_index,
            expected_type: data_type.clone(),
        };

        match node_handle.sdo_read(request).await {
            Ok(sdo_response) => {
                let value_string = sdo_response.data.to_string();
                let _ = update_tx.send(Update::SdoData {
                    address: address.clone(),
                    value: value_string,
                });
            },
            Err(err) => {
                let _ = update_tx.send(Update::SdoReadError {
                    address: address.clone(),
                    error: err.to_string(),
                });
            }
        };
    }
}

/// Health check task that periodically reads Device Type (0x1000:00) to verify node is alive
async fn health_check_task(
    update_tx: Sender<Update>,
    node_handle: CANopenNodeHandle,
) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let mut consecutive_failures = 0;
    const MAX_FAILURES: u32 = 2; // Mark disconnected after 2 consecutive failures

    loop {
        interval.tick().await;

        // Read mandatory Device Type object (0x1000:00)
        let request = SdoRequest {
            node_id: node_handle.node_id(),
            index: 0x1000,
            subindex: 0x00,
            expected_type: SdoDataType::UInt32,
        };

        match node_handle.sdo_read(request).await {
            Ok(_) => {
                consecutive_failures = 0;
                let _ = update_tx.send(Update::ConnectionStatus(true));
            },
            Err(err) => {
                consecutive_failures += 1;
                if consecutive_failures >= MAX_FAILURES {
                    println!("Health check failed: {}", err);
                    let _ = update_tx.send(Update::ConnectionStatus(false));
                    let _ = update_tx.send(Update::ConnectionFailed(
                        format!("Node not responding: {}", err)
                    ));
                }
            }
        }
    }
}

/// Parse a TPDO CAN frame according to the mapping configuration
fn parse_tpdo_frame(data: &[u8], config: &TpdoConfig) -> Vec<(String, String)> {
    let mut results = Vec::new();
    let mut bit_offset = 0usize;

    for obj in &config.mapped_objects {
        let value_str = extract_value_from_bytes(data, bit_offset, obj.bit_length, &obj.data_type);
        results.push((obj.name.clone(), value_str));
        bit_offset += obj.bit_length as usize;
    }

    results
}

/// TPDO listener task that receives raw CAN frames and parses them
async fn tpdo_listener_task(
    config: TpdoConfig,
    mut can_frame_rx: tokio::sync::mpsc::UnboundedReceiver<socketcan::CanFrame>,
    update_tx: Sender<Update>,
) {
    println!("TPDO listener started for TPDO {} on COB-ID {:#X}", config.tpdo_number, config.cob_id);

    while let Some(frame) = can_frame_rx.recv().await {
        // Check if this frame matches our TPDO COB-ID
        let frame_id = match frame.id() {
            socketcan::Id::Standard(std_id) => std_id.as_raw(),
            socketcan::Id::Extended(_) => continue, // Skip extended IDs
        };

        if frame_id == config.cob_id {
            let values = parse_tpdo_frame(frame.data(), &config);

            let tpdo_data = TpdoData {
                tpdo_number: config.tpdo_number,
                timestamp: Local::now(),
                values,
            };

            let _ = update_tx.send(Update::TpdoData(tpdo_data));
        }
    }

    println!("TPDO listener stopped for TPDO {}", config.tpdo_number);
}

fn merge_tpdo_configs(
    device_configs: Vec<TpdoConfig>,
    eds_configs: Vec<TpdoConfig>,
    object_dictionary: &BTreeMap<u16, SdoObject>,
) -> Vec<TpdoConfig> {
    let mut merged = Vec::new();

    for mut device_tpdo in device_configs {
        let eds_tpdo = eds_configs.iter()
            .find(|eds| eds.tpdo_number == device_tpdo.tpdo_number);

        for mapped_obj in &mut device_tpdo.mapped_objects {
            if let Some(eds) = eds_tpdo {
                if let Some(eds_obj) = eds.mapped_objects.iter()
                    .find(|obj| obj.index == mapped_obj.index && obj.sub_index == mapped_obj.sub_index)
                {
                    mapped_obj.name = eds_obj.name.clone();
                    mapped_obj.data_type = eds_obj.data_type.clone();
                    continue;
                }
            }

            if let Some(obj) = object_dictionary.get(&mapped_obj.index) {
                if let Some(sub_obj) = obj.sub_objects.get(&mapped_obj.sub_index) {
                    mapped_obj.name = sub_obj.name.clone();
                    if let Some(dt) = SdoDataType::from_eds_type(&sub_obj.data_type) {
                        mapped_obj.data_type = dt;
                    }
                }
            }
        }

        println!("Merged TPDO {} (from device, enriched with EDS)", device_tpdo.tpdo_number);
        merged.push(device_tpdo);
    }

    for eds_tpdo in eds_configs {
        if !merged.iter().any(|m| m.tpdo_number == eds_tpdo.tpdo_number) {
            println!("Adding TPDO {} from EDS (not found on device)", eds_tpdo.tpdo_number);
            merged.push(eds_tpdo);
        }
    }

    merged
}

fn parse_tpdos_from_eds(eds_file: &PathBuf, object_dictionary: &BTreeMap<u16, SdoObject>) -> Vec<TpdoConfig> {
    let mut tpdo_configs = Vec::new();
    let mut eds_parser = Ini::new();

    if eds_parser.load(eds_file).is_err() {
        println!("Failed to load EDS file for TPDO parsing");
        return tpdo_configs;
    }

    for tpdo_num in 1..=4u8 {
        let comm_param_index = 0x1800 + (tpdo_num - 1) as u16;
        let mapping_param_index = 0x1A00 + (tpdo_num - 1) as u16;

        let comm_section = format!("{:04X}sub1", comm_param_index);
        let cob_id = match eds_parser.get(&comm_section, "DefaultValue") {
            Some(value_str) => {
                // Parse hex value (format: "0x184" or "$NODEID+0x180")
                // For "$NODEID+0x180", we ignore $NODEID (treat as 0) and parse the hex part
                let to_parse = if value_str.contains("$NODEID+") {
                    value_str.split("+").nth(1).unwrap_or("0")
                } else if value_str.contains("+") {
                    // Handle "NODEID+0x180" without $ (some EDS formats)
                    value_str.split("+").nth(1).unwrap_or("0")
                } else {
                    value_str.as_str()
                };

                if let Ok(val) = if to_parse.starts_with("0x") || to_parse.starts_with("0X") {
                    u32::from_str_radix(&to_parse[2..], 16)
                } else {
                    to_parse.parse::<u32>()
                } {
                    // Check valid bit (bit 31)
                    if val & 0x80000000 != 0 {
                        println!("EDS: TPDO {} is disabled (invalid bit set)", tpdo_num);
                        continue;
                    }
                    (val & 0x7FF) as u16
                } else {
                    println!("EDS: Failed to parse COB-ID '{}' for TPDO {}", to_parse, tpdo_num);
                    continue;
                }
            }
            None => {
                println!("EDS: No COB-ID found for TPDO {}", tpdo_num);
                continue;
            }
        };

        let mapping_section_0 = format!("{:04X}sub0", mapping_param_index);
        let num_mapped = match eds_parser.get(&mapping_section_0, "DefaultValue") {
            Some(value_str) => value_str.parse::<u8>().unwrap_or(0),
            None => {
                println!("EDS: No mapping count found for TPDO {}", tpdo_num);
                continue;
            }
        };

        if num_mapped == 0 {
            println!("EDS: TPDO {} has no mapped objects", tpdo_num);
            continue;
        }

        let mut mapped_objects = Vec::new();
        for sub in 1..=num_mapped {
            let mapping_section = format!("{:04X}sub{}", mapping_param_index, sub);
            let mapping_value = match eds_parser.get(&mapping_section, "DefaultValue") {
                Some(value_str) => {
                    // Parse hex value
                    if let Ok(val) = if value_str.starts_with("0x") {
                        u32::from_str_radix(&value_str[2..], 16)
                    } else {
                        value_str.parse::<u32>()
                    } {
                        val
                    } else {
                        println!("EDS: Failed to parse mapping value for TPDO {} sub {}", tpdo_num, sub);
                        continue;
                    }
                }
                None => {
                    println!("EDS: No mapping found for TPDO {} sub {}", tpdo_num, sub);
                    continue;
                }
            };

            let obj_index = ((mapping_value >> 16) & 0xFFFF) as u16;
            let obj_subindex = ((mapping_value >> 8) & 0xFF) as u8;
            let bit_length = (mapping_value & 0xFF) as u8;

            let (name, data_type) = if let Some(obj) = object_dictionary.get(&obj_index) {
                if let Some(sub_obj) = obj.sub_objects.get(&obj_subindex) {
                    let dt = SdoDataType::from_eds_type(&sub_obj.data_type).unwrap_or_else(|| {
                        match bit_length {
                            8 => SdoDataType::UInt8,
                            16 => SdoDataType::UInt16,
                            32 => SdoDataType::UInt32,
                            _ => SdoDataType::UInt32,
                        }
                    });
                    (sub_obj.name.clone(), dt)
                } else {
                    (format!("0x{:04X}:{:02X}", obj_index, obj_subindex),
                     match bit_length {
                        8 => SdoDataType::UInt8,
                        16 => SdoDataType::UInt16,
                        _ => SdoDataType::UInt32,
                    })
                }
            } else {
                (format!("0x{:04X}:{:02X}", obj_index, obj_subindex),
                 match bit_length {
                    8 => SdoDataType::UInt8,
                    16 => SdoDataType::UInt16,
                    _ => SdoDataType::UInt32,
                })
            };

            mapped_objects.push(TpdoMappedObject {
                index: obj_index,
                sub_index: obj_subindex,
                bit_length,
                data_type,
                name,
            });
        }

        if !mapped_objects.is_empty() {
            println!("EDS: Found TPDO {} with COB-ID 0x{:03X} and {} mapped objects",
                     tpdo_num, cob_id, mapped_objects.len());

            tpdo_configs.push(TpdoConfig {
                tpdo_number: tpdo_num,
                cob_id,
                mapped_objects,
            });
        }
    }

    tpdo_configs
}

/// Discover TPDO configurations from the device via SDO reads
async fn discover_tpdos_from_device(node_handle: &CANopenNodeHandle) -> Vec<TpdoConfig> {
    let mut tpdo_configs = Vec::new();

    // Try to read TPDO 1-4 (standard CANopen supports 4 TPDOs)
    for tpdo_num in 1..=4u8 {
        let comm_param_index = 0x1800 + (tpdo_num - 1) as u16;
        let mapping_param_index = 0x1A00 + (tpdo_num - 1) as u16;

        // Read COB-ID from communication parameters (subindex 1)
        let cob_id_request = SdoRequest {
            node_id: node_handle.node_id(),
            index: comm_param_index,
            subindex: 1,
            expected_type: SdoDataType::UInt32,
        };

        let cob_id = match node_handle.sdo_read(cob_id_request).await {
            Ok(response) => {
                if let canopen_common::SdoResponseData::UInt32(value) = response.data {
                    // Bit 31 = valid bit (0 = valid, 1 = invalid)
                    if value & 0x80000000 != 0 {
                        println!("TPDO {} is disabled (invalid bit set)", tpdo_num);
                        continue; // TPDO is disabled
                    }
                    (value & 0x7FF) as u16 // Extract 11-bit COB-ID
                } else {
                    println!("TPDO {} COB-ID has unexpected type", tpdo_num);
                    continue;
                }
            }
            Err(err) => {
                println!("Failed to read TPDO {} COB-ID: {}", tpdo_num, err);
                continue;
            }
        };

        // Read number of mapped objects (subindex 0)
        let num_mapped_request = SdoRequest {
            node_id: node_handle.node_id(),
            index: mapping_param_index,
            subindex: 0,
            expected_type: SdoDataType::UInt8,
        };

        let num_mapped = match node_handle.sdo_read(num_mapped_request).await {
            Ok(response) => {
                if let canopen_common::SdoResponseData::UInt8(count) = response.data {
                    count
                } else {
                    println!("TPDO {} mapping count has unexpected type", tpdo_num);
                    continue;
                }
            }
            Err(err) => {
                println!("Failed to read TPDO {} mapping count: {}", tpdo_num, err);
                continue;
            }
        };

        if num_mapped == 0 {
            println!("TPDO {} has no mapped objects", tpdo_num);
            continue;
        }

        // Read each mapped object
        let mut mapped_objects = Vec::new();
        for sub in 1..=num_mapped {
            let mapping_request = SdoRequest {
                node_id: node_handle.node_id(),
                index: mapping_param_index,
                subindex: sub,
                expected_type: SdoDataType::UInt32,
            };

            let mapping_value = match node_handle.sdo_read(mapping_request).await {
                Ok(response) => {
                    if let canopen_common::SdoResponseData::UInt32(value) = response.data {
                        value
                    } else {
                        println!("TPDO {} mapping {} has unexpected type", tpdo_num, sub);
                        continue;
                    }
                }
                Err(err) => {
                    println!("Failed to read TPDO {} mapping {}: {}", tpdo_num, sub, err);
                    continue;
                }
            };

            // Parse mapping value: bits 31-16 = index, bits 15-8 = subindex, bits 7-0 = bit length
            let obj_index = ((mapping_value >> 16) & 0xFFFF) as u16;
            let obj_subindex = ((mapping_value >> 8) & 0xFF) as u8;
            let bit_length = (mapping_value & 0xFF) as u8;

            // For now, use a generic name - this will be enriched from EDS later
            let name = format!("0x{:04X}:{:02X}", obj_index, obj_subindex);

            // Infer data type from bit length (will be refined with EDS data)
            let data_type = match bit_length {
                8 => SdoDataType::UInt8,
                16 => SdoDataType::UInt16,
                32 => SdoDataType::UInt32,
                _ => {
                    println!("TPDO {} mapping {} has unsupported bit length: {}", tpdo_num, sub, bit_length);
                    continue;
                }
            };

            mapped_objects.push(TpdoMappedObject {
                index: obj_index,
                sub_index: obj_subindex,
                bit_length,
                data_type,
                name,
            });
        }

        if !mapped_objects.is_empty() {
            println!("Discovered TPDO {} with COB-ID 0x{:03X} and {} mapped objects",
                     tpdo_num, cob_id, mapped_objects.len());

            tpdo_configs.push(TpdoConfig {
                tpdo_number: tpdo_num,
                cob_id,
                mapped_objects,
            });
        }
    }

    tpdo_configs
}

/// Extract a value from a byte array at a specific bit offset
fn extract_value_from_bytes(data: &[u8], bit_offset: usize, bit_length: u8, data_type: &SdoDataType) -> String {
    let byte_offset = bit_offset / 8;

    // For Phase 1, we'll assume byte-aligned data (most common case)
    // Full bit-level extraction can be added later if needed
    match (bit_length, data_type) {
        (8, SdoDataType::UInt8) => {
            if byte_offset < data.len() {
                data[byte_offset].to_string()
            } else {
                "N/A".to_string()
            }
        },
        (8, SdoDataType::Int8) => {
            if byte_offset < data.len() {
                (data[byte_offset] as i8).to_string()
            } else {
                "N/A".to_string()
            }
        },
        (16, SdoDataType::UInt16) => {
            if byte_offset + 1 < data.len() {
                let value = u16::from_le_bytes([data[byte_offset], data[byte_offset + 1]]);
                value.to_string()
            } else {
                "N/A".to_string()
            }
        },
        (16, SdoDataType::Int16) => {
            if byte_offset + 1 < data.len() {
                let value = i16::from_le_bytes([data[byte_offset], data[byte_offset + 1]]);
                value.to_string()
            } else {
                "N/A".to_string()
            }
        },
        (32, SdoDataType::UInt32) => {
            if byte_offset + 3 < data.len() {
                let value = u32::from_le_bytes([
                    data[byte_offset],
                    data[byte_offset + 1],
                    data[byte_offset + 2],
                    data[byte_offset + 3],
                ]);
                value.to_string()
            } else {
                "N/A".to_string()
            }
        },
        (32, SdoDataType::Int32) => {
            if byte_offset + 3 < data.len() {
                let value = i32::from_le_bytes([
                    data[byte_offset],
                    data[byte_offset + 1],
                    data[byte_offset + 2],
                    data[byte_offset + 3],
                ]);
                value.to_string()
            } else {
                "N/A".to_string()
            }
        },
        (32, SdoDataType::Real32) => {
            if byte_offset + 3 < data.len() {
                let value = f32::from_le_bytes([
                    data[byte_offset],
                    data[byte_offset + 1],
                    data[byte_offset + 2],
                    data[byte_offset + 3],
                ]);
                format!("{:.2}", value)
            } else {
                "N/A".to_string()
            }
        },
        _ => {
            format!("Unsupported: {} bits, {:?}", bit_length, data_type)
        }
    }
}

pub fn communication_thread_main(
    command_rx: Receiver<Command>,
    update_tx: Sender<Update>,
    can_interface: String,
    node_id: u8,
    eds_file: Option<PathBuf>,
) {
    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut subscription_handles: HashMap<SdoAddress, JoinHandle<()>> = HashMap::new();
    let mut tpdo_handles: HashMap<u8, JoinHandle<()>> = HashMap::new();
    let mut _health_check_handle: Option<JoinHandle<()>> = None;
    let mut connection_handle: Option<CANopenConnection> = None;
    let mut node_handle: Option<CANopenNodeHandle> = None;
    let mut object_dictionary: BTreeMap<u16, SdoObject> = BTreeMap::new();


    for command in command_rx {
        match command {
            Command::Connect => {
                match rt.block_on(async {
                    let conn = CANopenConnection::new(&can_interface, Duration::from_millis(1000)).await?;
                    let handle = conn.add_node(node_id).await?;
                    Ok::<(CANopenConnection, CANopenNodeHandle), Box<dyn std::error::Error>>((conn, handle))
                }){
                    Ok((conn, handle)) => {
                        connection_handle = Some(conn);
                        node_handle = Some(handle.clone());

                        let update_tx_clone = update_tx.clone();
                        let health_handle = rt.spawn(health_check_task(update_tx_clone, handle));
                        _health_check_handle = Some(health_handle);

                        println!("Connection established, health check started");
                    },
                    Err(err) => {
                        let _ = update_tx.send(Update::ConnectionFailed(err.to_string()));
                    }
                };
            },
            Command::FetchSdos => {
                if let Some(path) = eds_file.as_ref() {
                    match search_for_readable_sdo(path.clone()) {
                        Ok(objects) => {
                            object_dictionary = objects.clone();
                            let _ = update_tx.send(Update::SdoList(objects));
                        },
                        Err(_) => {
                            let _ = update_tx.send(Update::SdoList(BTreeMap::new()));
                        }
                    }
                } else {
                    let _ = update_tx.send(Update::SdoList(BTreeMap::new()));
                }
            },
            Command::DiscoverTpdos => {
                println!("Starting TPDO discovery...");

                let device_tpdos = if let Some(ref handle) = node_handle {
                    rt.block_on(discover_tpdos_from_device(handle))
                } else {
                    println!("Cannot discover TPDOs from device: not connected");
                    Vec::new()
                };

                let eds_tpdos = if let Some(ref path) = eds_file {
                    parse_tpdos_from_eds(path, &object_dictionary)
                } else {
                    println!("No EDS file available for TPDO parsing");
                    Vec::new()
                };

                let merged_tpdos = merge_tpdo_configs(device_tpdos, eds_tpdos, &object_dictionary);

                println!("TPDO discovery complete - found {} TPDOs", merged_tpdos.len());
                let _ = update_tx.send(Update::TpdosDiscovered(merged_tpdos));
            },
            Command::Subscribe { address, interval_ms, data_type } => {
                if let Some(ref handle) = node_handle {
                    println!("Subscribing to address {:?} with interval {} ms", &address, interval_ms);

                    let update_tx_clone = update_tx.clone();
                    let handle_clone = handle.clone();

                    let subscription_handle = rt.spawn(sdo_polling_task(
                        address.clone(),
                        interval_ms,
                        update_tx_clone,
                        handle_clone,
                        data_type,
                    ));

                    subscription_handles.insert(address, subscription_handle);
                } else {
                    let _ = update_tx.send(Update::ConnectionFailed(
                        "Not connected to CANopen network".to_string()
                    ));
                }
            },
            Command::Unsubscribe(address) => {
                println!("Unsubscribing from address {:?}", &address);
                if let Some(subscription_handle) = subscription_handles.remove(&address) {
                    subscription_handle.abort();
                }
            },
            Command::StartTpdoListener(config) => {
                if let Some(ref conn) = connection_handle {
                    let tpdo_num = config.tpdo_number;
                    println!("Starting TPDO listener for TPDO {} on COB-ID {:#X}", tpdo_num, config.cob_id);

                    match rt.block_on(conn.subscribe_raw_frames()) {
                        Ok(frame_rx) => {
                            let update_tx_clone = update_tx.clone();
                            let tpdo_handle = rt.spawn(tpdo_listener_task(config, frame_rx, update_tx_clone));
                            tpdo_handles.insert(tpdo_num, tpdo_handle);
                        }
                        Err(err) => {
                            let _ = update_tx.send(Update::ConnectionFailed(
                                format!("Failed to subscribe to CAN frames: {}", err)
                            ));
                        }
                    }
                } else {
                    let _ = update_tx.send(Update::ConnectionFailed(
                        "Not connected to CANopen network".to_string()
                    ));
                }
            },
            Command::StopTpdoListener(tpdo_num) => {
                println!("Stopping TPDO listener for TPDO {}", tpdo_num);
                if let Some(handle) = tpdo_handles.remove(&tpdo_num) {
                    handle.abort();
                }
            },
        }
    }
}

pub fn search_for_readable_sdo(eds_file: PathBuf) -> Result<BTreeMap<u16, SdoObject>, String> {
    let mut eds_parser = Ini::new();
    if let Ok(eds_sections) = eds_parser.load(eds_file) {
        let mut objects: BTreeMap<u16, SdoObject> = BTreeMap::new();

        for (section, properties) in &eds_sections {
            if section.contains("sub") {
                if let (Some(index_str), Some(sub_index_str)) =
                    (section.split("sub").next(), section.split("sub").nth(1))
                {
                    if let (Ok(index), Ok(sub_index)) =
                        (u16::from_str_radix(index_str, 16), sub_index_str.parse::<u8>())
                    {
                        if let Some(Some(access)) = properties.get("accesstype") {
                            if access == "ro" || access == "rw" {
                                let sub_name = properties.get("parametername")
                                    .and_then(|opt| opt.as_ref())
                                    .map(|s| s.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let data_type = properties.get("datatype")
                                    .and_then(|opt| opt.as_ref())
                                    .map(|s| s.as_str())
                                    .unwrap_or("")
                                    .to_string();

                                let parent_object = objects.entry(index).or_insert_with(|| {
                                    let object_name = eds_sections.get(index_str)
                                        .and_then(|props| props.get("parametername"))
                                        .and_then(|opt| opt.as_ref())
                                        .map(|s| s.as_str())
                                        .unwrap_or("Unnamed Object")
                                        .to_string();
                                    SdoObject {
                                        name: object_name,
                                        sub_objects: BTreeMap::new(),
                                    }
                                });

                                let sub_object = SdoSubObject { name: sub_name, data_type };
                                parent_object.sub_objects.insert(sub_index, sub_object);
                            }
                        }
                    }
                }
            }
        }
        return Ok(objects);
    }
    Err("Failed to parse EDS file".to_string())
}