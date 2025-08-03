use std::sync::mpsc::{Receiver, Sender};
use std::path::PathBuf;
use configparser::ini::Ini;
use std::collections::{BTreeMap, HashMap};
use rand::Rng;
use tokio::{sync::{mpsc as tokio_mpsc, oneshot}, task::JoinHandle};


#[derive(Debug, Clone)]
pub struct SdoSubObject {
    pub sub_index: u8,
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone)]
pub struct SdoObject {
    pub index: u16,
    pub name: String,
    /// We use a BTreeMap to keep sub-objects automatically sorted by their sub_index (the u8 key).
    pub sub_objects: BTreeMap<u8, SdoSubObject>,
}

#[derive(Debug, Clone, Eq, Hash, PartialEq)]
pub struct SdoAddress {
    pub index: u16,
    pub sub_index: u8,
}

#[derive(Debug)]
pub enum Command {
    FetchSdos,
    Connect,
    Subscribe {
        address: SdoAddress,
        interval_ms: u64,
    },
    Unsubscribe(SdoAddress),
}

#[derive(Debug)]
pub enum Update {
    SdoList(BTreeMap<u16, SdoObject>),
    ConnectionSuccess(BTreeMap<u16, SdoObject>),
    ConnectionFailed(String),
    SdoData {
        address: SdoAddress,
        value: String,
    },
}

async fn simulation_task(address: SdoAddress, interval_ms: u64, update_tx: Sender<Update>) {
    println!("Starting simulation task for address {:?} with interval {} ms", &address, interval_ms);
    let mut interval = tokio::time::interval(std::time::Duration::from_millis(interval_ms));


    loop {
        interval.tick().await;
        let mut rng = rand::thread_rng();
        let random_value = rng.gen_range(0..100);
        let _ = update_tx.send(Update::SdoData {
            address: address.clone(),
            value: format!("{}", random_value),
        });
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

    for command in command_rx {
        match command {
            Command::Connect => {},
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
            Command::Subscribe { address, interval_ms } => {
                println!("Subscribing to address {:?} with interval {} ms", &address, interval_ms);
                let update_tx_clone = update_tx.clone();
                let subscription_handle = rt.spawn(simulation_task(address.clone(), interval_ms, update_tx_clone));
                subscription_handles.insert(address, subscription_handle);
            },
            Command::Unsubscribe(address) => {
                println!("Unsubscribing from address {:?}", &address);
                let subscription_handle = subscription_handles.remove(&address);
                if let Some(subscription_handle) = subscription_handle {
                    subscription_handle.abort();
                }
            }
            _ => {}
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