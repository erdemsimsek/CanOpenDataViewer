use std::net::Shutdown::Read;
use std::sync::mpsc::{Receiver, Sender};
use std::path::PathBuf;
use canopen_rust::{data_type, debug};
use configparser::ini::Ini;
use std::collections::HashSet;
use std::collections::BTreeMap;


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

#[derive(Debug, Clone)]
pub struct SdoAddress {
    pub index: u16,
    pub sub_index: u8,
}

#[derive(Debug)]
pub enum Command {
    Connect,
    Subscribe {
        address: SdoAddress,
        interval_ms: u64,
    },
    Unsubscribe(SdoAddress),
}

#[derive(Debug)]
pub enum Update {
    ConnectionSuccess(BTreeMap<u16, SdoObject>),
    ConnectionFailed(String),
    SdoData {
        address: SdoAddress,
        value: String,
    },
}

// Make the main function for the thread public as well.
pub fn communication_thread_main(
    command_rx: Receiver<Command>,
    update_tx: Sender<Update>,
    can_interface: String,
    node_id: u8,
    eds_file: Option<PathBuf>,
) {

    match command_rx.recv() {
        Ok(Command::Connect) => {

            if let Some(path) = eds_file {
                match search_for_readable_sdo(path) {
                    Ok(sdo_groups) => {

                        let _ = update_tx.send(Update::ConnectionSuccess(sdo_groups));
                    },
                    Err(err) => {
                        let _ = update_tx.send(Update::ConnectionFailed(err));
                    }
                }
            }
            else {
                println!("No EDS file provided");

            }
        },
        _ => {}
    }

    loop {

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