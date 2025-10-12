//! Object Dictionary for the mock CANopen node
//!
//! This module defines the simulated object dictionary with test data.

use std::collections::HashMap;
use canopen_common::SdoDataType;
use rand::Rng;

/// Represents a single entry in the object dictionary
pub enum ObjectEntry {
    /// Static value that doesn't change
    Static(Vec<u8>, SdoDataType),
    /// Dynamic value generated on each read
    Dynamic(Box<dyn Fn() -> Vec<u8> + Send + Sync>, SdoDataType),
}

/// Object dictionary mapping (index, subindex) to values
pub struct ObjectDictionary {
    entries: HashMap<(u16, u8), ObjectEntry>,
}

impl ObjectDictionary {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
        }
    }

    /// Add a static entry to the dictionary
    pub fn add_static(&mut self, index: u16, subindex: u8, data: Vec<u8>, data_type: SdoDataType) {
        self.entries.insert((index, subindex), ObjectEntry::Static(data, data_type));
    }

    /// Add a dynamic entry (value generated on each read)
    pub fn add_dynamic<F>(&mut self, index: u16, subindex: u8, generator: F, data_type: SdoDataType)
    where
        F: Fn() -> Vec<u8> + Send + Sync + 'static,
    {
        self.entries.insert(
            (index, subindex),
            ObjectEntry::Dynamic(Box::new(generator), data_type),
        );
    }

    /// Get an entry from the dictionary
    pub fn get(&self, index: u16, subindex: u8) -> Option<(Vec<u8>, SdoDataType)> {
        self.entries.get(&(index, subindex)).map(|entry| {
            match entry {
                ObjectEntry::Static(data, dtype) => (data.clone(), dtype.clone()),
                ObjectEntry::Dynamic(generator, dtype) => (generator(), dtype.clone()),
            }
        })
    }

    /// Get number of entries
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Print a summary of all objects
    pub fn print_summary(&self) {
        let mut indices: Vec<_> = self.entries.keys().collect();
        indices.sort();

        for (index, subindex) in indices {
            let entry_type = match &self.entries[&(*index, *subindex)] {
                ObjectEntry::Static(_, dtype) => format!("Static {:?}", dtype),
                ObjectEntry::Dynamic(_, dtype) => format!("Dynamic {:?}", dtype),
            };
            println!("  0x{:04X}:{:02X} - {}", index, subindex, entry_type);
        }
    }

    /// Add standard test objects for demonstration
    pub fn add_test_objects(&mut self) {
        // 0x1000:00 - Device Type (UInt32) - Static
        self.add_static(0x1000, 0x00, 0x00000191u32.to_le_bytes().to_vec(), SdoDataType::UInt32);

        // 0x1001:00 - Error Register (UInt8) - Static
        self.add_static(0x1001, 0x00, vec![0x00], SdoDataType::UInt8);

        // 0x1008:00 - Device Name (String) - Static
        let device_name = "MockCANopenNode";
        self.add_static(0x1008, 0x00, device_name.as_bytes().to_vec(), SdoDataType::VisibleString);

        // 0x1018:01 - Vendor ID (UInt32) - Static
        self.add_static(0x1018, 0x01, 0x00000001u32.to_le_bytes().to_vec(), SdoDataType::UInt32);

        // 0x2000:01 - Temperature Sensor (Real32) - Dynamic (simulated changing value)
        self.add_dynamic(
            0x2000,
            0x01,
            || {
                let mut rng = rand::rng();
                let temp: f32 = rng.random_range(20.0..30.0); // Random temperature between 20-30°C
                temp.to_le_bytes().to_vec()
            },
            SdoDataType::Real32,
        );

        // 0x2000:02 - Pressure Sensor (Real32) - Dynamic
        self.add_dynamic(
            0x2000,
            0x02,
            || {
                let mut rng = rand::rng();
                let pressure: f32 = rng.random_range(95.0..105.0); // Random pressure 95-105 kPa
                pressure.to_le_bytes().to_vec()
            },
            SdoDataType::Real32,
        );

        // 0x2001:01 - Counter (UInt32) - Dynamic (incrementing)
        use std::sync::atomic::{AtomicU32, Ordering};
        use std::sync::Arc;

        let counter = Arc::new(AtomicU32::new(0));
        let counter_clone = counter.clone();
        self.add_dynamic(
            0x2001,
            0x01,
            move || {
                let value = counter_clone.fetch_add(1, Ordering::SeqCst);
                value.to_le_bytes().to_vec()
            },
            SdoDataType::UInt32,
        );

        // 0x2002:01 - Voltage (Real32) - Dynamic
        self.add_dynamic(
            0x2002,
            0x01,
            || {
                let mut rng = rand::rng();
                let voltage: f32 = rng.random_range(11.5..12.5); // Random voltage 11.5-12.5V
                voltage.to_le_bytes().to_vec()
            },
            SdoDataType::Real32,
        );

        // 0x2002:02 - Current (Real32) - Dynamic
        self.add_dynamic(
            0x2002,
            0x02,
            || {
                let mut rng = rand::rng();
                let current: f32 = rng.random_range(0.5..5.0); // Random current 0.5-5.0A
                current.to_le_bytes().to_vec()
            },
            SdoDataType::Real32,
        );

        // 0x2003:01 - Status Word (UInt16) - Static
        self.add_static(0x2003, 0x01, 0x0031u16.to_le_bytes().to_vec(), SdoDataType::UInt16);

        // 0x2003:02 - Control Word (UInt16) - Static
        self.add_static(0x2003, 0x02, 0x000Fu16.to_le_bytes().to_vec(), SdoDataType::UInt16);

        // 0x2004:01 - RPM (Int32) - Dynamic (simulated motor speed)
        self.add_dynamic(
            0x2004,
            0x01,
            || {
                let mut rng = rand::rng();
                let rpm: i32 = rng.random_range(1000..3000); // Random RPM 1000-3000
                rpm.to_le_bytes().to_vec()
            },
            SdoDataType::Int32,
        );

        // 0x2005:01 - Position (Int32) - Dynamic (incrementing position)
        let position = Arc::new(AtomicU32::new(0));
        let position_clone = position.clone();
        self.add_dynamic(
            0x2005,
            0x01,
            move || {
                let value = position_clone.fetch_add(10, Ordering::SeqCst);
                (value as i32).to_le_bytes().to_vec()
            },
            SdoDataType::Int32,
        );
    }
}
