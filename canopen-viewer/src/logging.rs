use std::path::PathBuf;
use std::fs::{self, File};
use std::sync::{Arc, Mutex};
use chrono::Local;
use csv::Writer;

#[derive(Debug, Clone)]
pub enum LogEvent {
    SdoData {
        index: u16,
        sub_index: u8,
        value: String,
    },
    SdoError {
        index: u16,
        sub_index: u8,
        error: String,
    },
    TpdoData {
        tpdo_number: u8,
        values: Vec<(String, String)>,
    },
    #[allow(dead_code)]  // Reserved for future use
    ConnectionSuccess,
    ConnectionFailed(String),
    ConnectionStatus(bool),
}

pub struct Logger {
    writer: Arc<Mutex<Option<Writer<File>>>>,
    enabled: bool,
    log_file_path: Option<PathBuf>,
}

impl Logger {
    /// Create a new logger (disabled by default)
    pub fn new() -> Self {
        Self {
            writer: Arc::new(Mutex::new(None)),
            enabled: false,
            log_file_path: None,
        }
    }

    /// Enable logging and create a new log file
    pub fn enable(&mut self, log_directory: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
        // Create log directory if it doesn't exist
        fs::create_dir_all(&log_directory)?;

        // Generate log file name with timestamp
        let timestamp = Local::now().format("%Y%m%d_%H%M%S");
        let log_filename = format!("canopen_log_{}.csv", timestamp);
        let log_path = log_directory.join(log_filename);

        // Create CSV writer
        let file = File::create(&log_path)?;
        let mut writer = Writer::from_writer(file);

        // Write CSV header
        writer.write_record(&["Timestamp", "Event Type", "Address", "Value", "Message"])?;
        writer.flush()?;

        // Store writer and update state
        *self.writer.lock().unwrap() = Some(writer);
        self.enabled = true;
        self.log_file_path = Some(log_path.clone());

        println!("✓ Logging enabled: {:?}", log_path);
        Ok(())
    }

    /// Disable logging and close the file
    pub fn disable(&mut self) {
        *self.writer.lock().unwrap() = None;
        self.enabled = false;
        println!("✓ Logging disabled");
    }

    /// Check if logging is enabled
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    /// Get the current log file path
    pub fn log_file_path(&self) -> Option<PathBuf> {
        self.log_file_path.clone()
    }

    /// Log an event
    pub fn log(&self, event: LogEvent) {
        if !self.enabled {
            return;
        }

        let timestamp = Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string();

        let (event_type, address, value, message) = match event {
            LogEvent::SdoData { index, sub_index, value } => (
                "SDO_DATA".to_string(),
                format!("{:04X}:{:02X}", index, sub_index),
                value,
                String::new(),
            ),
            LogEvent::SdoError { index, sub_index, error } => (
                "SDO_ERROR".to_string(),
                format!("{:04X}:{:02X}", index, sub_index),
                String::new(),
                error,
            ),
            LogEvent::TpdoData { tpdo_number, values } => {
                let fields = values.iter()
                    .map(|(name, val)| format!("{}={}", name, val))
                    .collect::<Vec<_>>()
                    .join(", ");
                (
                    "TPDO_DATA".to_string(),
                    format!("TPDO{}", tpdo_number),
                    fields,
                    String::new(),
                )
            },
            LogEvent::ConnectionSuccess => (
                "CONNECTION_SUCCESS".to_string(),
                String::new(),
                String::new(),
                "Successfully connected to CANopen node".to_string(),
            ),
            LogEvent::ConnectionFailed(err) => (
                "CONNECTION_FAILED".to_string(),
                String::new(),
                String::new(),
                err,
            ),
            LogEvent::ConnectionStatus(is_alive) => (
                "CONNECTION_STATUS".to_string(),
                String::new(),
                if is_alive { "Connected" } else { "Disconnected" }.to_string(),
                String::new(),
            ),
        };

        // Write to CSV
        if let Ok(mut writer_guard) = self.writer.lock() {
            if let Some(writer) = writer_guard.as_mut() {
                if let Err(e) = writer.write_record(&[&timestamp, &event_type, &address, &value, &message]) {
                    eprintln!("Failed to write log entry: {}", e);
                }
                if let Err(e) = writer.flush() {
                    eprintln!("Failed to flush log file: {}", e);
                }
            }
        }
    }
}

impl Default for Logger {
    fn default() -> Self {
        Self::new()
    }
}
