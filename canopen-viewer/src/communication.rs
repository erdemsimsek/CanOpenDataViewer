use std::sync::mpsc::{Receiver, Sender};
use std::path::PathBuf;
use configparser::ini::Ini;
use std::collections::{BTreeMap, HashMap};
use tokio::task::JoinHandle;
use std::time::Duration;
use crate::canopen::{
    CANopenConnection, CANopenNodeHandle,
    SdoRequest, SdoDataType
};


#[derive(Debug, Clone)]
pub struct SdoSubObject {
    #[allow(dead_code)]  // Stored from EDS for reference
    pub sub_index: u8,
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone)]
pub struct SdoObject {
    #[allow(dead_code)]  // Used internally by BTreeMap, needed for EDS parsing
    pub index: u16,
    pub name: String,
    /// We use a BTreeMap to keep sub-objects automatically sorted by their sub_index (the u8 key).
    pub sub_objects: BTreeMap<u8, SdoSubObject>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct SdoAddress {
    pub index: u16,
    #[allow(dead_code)]  // Used in HashMap key, accessed via pattern matching
    pub sub_index: u8,
}

#[derive(Debug)]
pub enum Command {
    FetchSdos,
    Connect,
    Subscribe {
        address: SdoAddress,
        interval_ms: u64,
        data_type: SdoDataType,
    },
    Unsubscribe(SdoAddress),
}

#[derive(Debug)]
pub enum Update {
    SdoList(BTreeMap<u16, SdoObject>),
    #[allow(dead_code)]  // TODO: Will be used for successful connection notifications
    ConnectionSuccess(BTreeMap<u16, SdoObject>),
    ConnectionFailed(String),
    ConnectionStatus(bool),  // true = node alive, false = node not responding
    SdoData {
        address: SdoAddress,
        value: String,
    },
    SdoReadError {
        address: SdoAddress,
        error: String,
    },
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
                // Node is alive
                consecutive_failures = 0;
                let _ = update_tx.send(Update::ConnectionStatus(true));
            },
            Err(err) => {
                // Node not responding
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

// Make the main function for the thread public as well.
pub fn communication_thread_main(
    command_rx: Receiver<Command>,
    update_tx: Sender<Update>,
    can_interface: String,
    node_id: u8,
    eds_file: Option<PathBuf>,
) {

    let rt = tokio::runtime::Runtime::new().unwrap();
    let mut subscription_handles: HashMap<SdoAddress, JoinHandle<()>> = HashMap::new();
    let mut _health_check_handle: Option<JoinHandle<()>> = None;
    // Keep connection alive - it owns the background CAN reader task
    let mut _connection_handle: Option<CANopenConnection> = None;
    let mut node_handle: Option<CANopenNodeHandle> = None;


    for command in command_rx {
        match command {
            Command::Connect => {
                match rt.block_on(async {
                    let conn = CANopenConnection::new(&can_interface, Duration::from_millis(1000)).await?;
                    let handle = conn.add_node(node_id).await?;
                    Ok::<(CANopenConnection, CANopenNodeHandle), Box<dyn std::error::Error>>((conn, handle))
                }){
                    Ok((conn, handle)) => {
                        _connection_handle = Some(conn);
                        node_handle = Some(handle.clone());

                        // Spawn health check task to monitor node
                        let update_tx_clone = update_tx.clone();
                        let health_handle = rt.spawn(health_check_task(
                            update_tx_clone,
                            handle,
                        ));
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
                        Ok(sdo_groups) => {
                            let _ = update_tx.send(Update::SdoList(sdo_groups));
                        },
                        Err(_err) => {
                            let _ = update_tx.send(Update::SdoList(BTreeMap::new()));
                        }
                    }
                }
                else {
                    let _ = update_tx.send(Update::SdoList(BTreeMap::new()));
                }
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
            }
        }
    }
}

/// Parses an EDS file by first finding all supported objects.
pub fn search_for_readable_sdo(eds_file: PathBuf) -> Result<BTreeMap<u16, SdoObject>, String> {

    let mut config = Ini::new();
    if let Ok(map) = config.load(eds_file) {

        let mut sdo_groups: BTreeMap<u16, SdoObject> = BTreeMap::new();

        for(section, properties) in &map{
            if section.contains("sub"){
                if let (Some(index_str), Some(sub_index_str)) =
                    (section.split("sub").next(), section.split("sub").nth(1))
                {
                    if let (Ok(index), Ok(sub_index)) =
                        (u16::from_str_radix(index_str, 16), sub_index_str.parse::<u8>())
                    {
                        // Safely get the "AccessType" and check if it's readable.
                        if let Some(Some(access)) = properties.get("accesstype") {
                            if access == "ro" || access == "rw" {
                                let sub_name = properties.get("parametername")
                                    .and_then(|opt| opt.as_ref()).map(|s| s.as_str()).unwrap_or("").to_string();

                                let data_type = properties.get("datatype")
                                    .and_then(|opt| opt.as_ref()).map(|s| s.as_str()).unwrap_or("").to_string();

                                let parent_sdo = sdo_groups.entry(index).or_insert_with(|| {
                                    let object_name = map.get(index_str)
                                        .and_then(|props| props.get("parametername"))
                                        .and_then(|opt| opt.as_ref()).map(|s| s.as_str()).unwrap_or("Unnamed Object").to_string();
                                    SdoObject {
                                        index,
                                        name: object_name,
                                        sub_objects: BTreeMap::new(),
                                    }
                                });

                                let sub_object = SdoSubObject{sub_index, name: sub_name, data_type};
                                parent_sdo.sub_objects.insert(sub_index, sub_object);


                            }
                        }
                    }
                }
            }
        }
        return Ok(sdo_groups);
    }
    Err("Failed to parse EDS file".to_string())
}