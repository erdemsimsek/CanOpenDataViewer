// main.rs

mod communication;
mod canopen;
mod config;
mod logging;

// Version information embedded at compile time
const APP_VERSION: &str = env!("APP_VERSION");
const GIT_HASH: &str = env!("GIT_HASH");
const GIT_BRANCH: &str = env!("GIT_BRANCH");
const GIT_DIRTY: &str = env!("GIT_DIRTY");
const BUILD_TIME: &str = env!("BUILD_TIME");

use std::collections::{BTreeMap, HashMap, VecDeque, HashSet};
use communication::{Command, Update, SdoAddress, SdoObject, TpdoData};
use canopen_common::SdoDataType;
use config::AppConfig;
use logging::{Logger, LogEvent};

use eframe::{egui, NativeOptions, egui::Color32, egui::ColorImage};
use std::process::Command as process_command;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, Receiver};
use egui_plot::{Plot, PlotPoints, Line, Legend};
use chrono::{Local, DateTime};
use std::sync::Arc;

const PLOT_BUFFER_SIZE: usize = 500;

enum AppView {
    SelectInterface,
    SelectNodeId,
    SelectEDSFile,
    Main
}

#[derive(Debug, Clone, PartialEq)]
enum SidebarTab {
    SDO,
    TPDO,
}

#[derive(Debug, Clone)]
pub enum SubscriptionStatus {
    Active,       // Currently receiving data
    Error(String), // Error occurred (with error message)
    Idle,         // Subscribed but no recent data
}

#[derive(Debug, Clone)]
struct SdoSubscription{
    interval_ms: u64,
    plot_data: VecDeque<[f64; 2]>, // [timestamp_seconds, value]
    data_type: SdoDataType,
    last_value: Option<String>,
    last_timestamp: Option<DateTime<Local>>,
    status: SubscriptionStatus,
    paused: bool,
    start_time: DateTime<Local>, // Reference point for relative timestamps
}

// Identifier for a specific field within a TPDO
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct TpdoFieldId {
    tpdo_number: u8,
    field_name: String,  // e.g., "Temperature", "Pressure", "Status"
}

#[derive(Debug, Clone)]
struct TpdoFieldSubscription {
    plot_data: VecDeque<[f64; 2]>, // [timestamp_seconds, value]
    last_value: Option<String>,
    last_timestamp: Option<DateTime<Local>>,
    start_time: DateTime<Local>,
}

struct ScreenshotInfo {
    filename: String,
    rect: egui::Rect,
}

struct MyApp {
    current_view: AppView,
    available_can_interfaces: Vec<String>,
    selected_can_interface: Option<String>,
    selected_node_id: Option<u8>,
    node_id_str : String,
    eds_file_path : Option<PathBuf>,

    command_tx: Option<Sender<Command>>,
    update_rx: Option<Receiver<Update>>,

    connection_status: bool,
    connection_requested: bool,

    sdo_requested : bool,
    sdo_data : Option<BTreeMap<u16, SdoObject>>,

    // Storing the state of all active subscriptions
    subscriptions : HashMap<SdoAddress, SdoSubscription>,

    // Managing the state of the pop-up configuration modal
    modal_open_for: Option<SdoAddress>,
    modal_interval_str: String,

    sdo_search_query: String,
    tpdo_search_query: String,
    sidebar_tab: SidebarTab,

    // Error reporting
    error_message: Option<String>,

    // Configuration and logging
    config: AppConfig,
    logger: Logger,

    // UI state
    show_about_dialog: bool,

    // TPDO Phase 1 - Simple display
    tpdo_data: Vec<TpdoData>,  // Store recent TPDO messages
    tpdo_discovery_requested: bool,
    discovered_tpdos: Vec<communication::TpdoConfig>,  // Discovered TPDO configurations
    active_tpdos: std::collections::HashSet<u8>,  // Set of TPDO numbers currently running

    // TPDO field plotting
    tpdo_field_subscriptions: HashMap<TpdoFieldId, TpdoFieldSubscription>,
}


impl Default for MyApp {
    fn default() -> Self {
        // Load configuration from file
        let config = AppConfig::load();

        // Initialize logger
        let mut logger = Logger::new();
        if config.enable_logging {
            if let Some(log_dir) = config.get_log_directory() {
                if let Err(e) = logger.enable(log_dir) {
                    eprintln!("Failed to enable logging: {}", e);
                }
            }
        }

        // Pre-populate fields from loaded config
        let selected_can_interface = if config.can_interface.is_empty() {
            None
        } else {
            Some(config.can_interface.clone())
        };

        let (selected_node_id, node_id_str) = if config.node_id > 0 && config.node_id <= 127 {
            (Some(config.node_id), config.node_id.to_string())
        } else {
            (None, String::new())
        };

        let eds_file_path = config.eds_file_path.as_ref().map(PathBuf::from);

        Self {
            current_view: AppView::SelectInterface,
            available_can_interfaces: get_can_interfaces(),
            selected_can_interface,
            selected_node_id,
            node_id_str,
            eds_file_path,

            command_tx: None,
            update_rx: None,

            connection_status: false,
            connection_requested: false,

            sdo_requested: false,
            sdo_data: None,

            subscriptions: HashMap::new(),

            modal_open_for: None,
            modal_interval_str: String::new(),

            sdo_search_query: String::new(),
            tpdo_search_query: String::new(),
            sidebar_tab: SidebarTab::SDO,

            error_message: None,

            config,
            logger,

            show_about_dialog: false,

            tpdo_data: Vec::new(),
            tpdo_discovery_requested: false,
            discovered_tpdos: Vec::new(),
            active_tpdos: HashSet::new(),

            tpdo_field_subscriptions: HashMap::new(),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {

        if let Some(update) = self.update_rx.as_mut().and_then(|rx| rx.try_recv().ok()) {
            match update{
                Update::SdoList(map) => {
                    self.sdo_data = Some(map);
                },

                Update::SdoData { address, value } => {
                    // Log SDO data
                    self.logger.log(LogEvent::SdoData {
                        index: address.index,
                        sub_index: address.sub_index,
                        value: value.clone(),
                    });

                    // Update subscription metadata
                    if let Some(subscription) = self.subscriptions.get_mut(&address) {
                        let now = Local::now();
                        subscription.last_value = Some(value.clone());
                        subscription.last_timestamp = Some(now);
                        subscription.status = SubscriptionStatus::Active;

                        // Only add to plot data if not paused
                        if !subscription.paused {
                            // Try to parse the incoming string value into a number for plotting.
                            if let Ok(number_value) = value.parse::<f64>() {
                                if subscription.plot_data.len() >= PLOT_BUFFER_SIZE {
                                    subscription.plot_data.pop_front();
                                }

                                // Calculate seconds since start time for X-axis
                                let elapsed_seconds = (now - subscription.start_time).num_milliseconds() as f64 / 1000.0;
                                subscription.plot_data.push_back([elapsed_seconds, number_value]);
                            }
                        }
                    }
                }
                Update::ConnectionFailed(error) => {
                    // Log connection failure
                    self.logger.log(LogEvent::ConnectionFailed(error.clone()));

                    self.error_message = Some(format!("Connection Error: {}", error));
                    self.connection_status = false;
                }
                Update::ConnectionStatus(is_alive) => {
                    // Log connection status change
                    self.logger.log(LogEvent::ConnectionStatus(is_alive));

                    self.connection_status = is_alive;
                }
                Update::SdoReadError { address, error } => {
                    // Log SDO error
                    self.logger.log(LogEvent::SdoError {
                        index: address.index,
                        sub_index: address.sub_index,
                        error: error.clone(),
                    });

                    // Update subscription status to error
                    if let Some(subscription) = self.subscriptions.get_mut(&address) {
                        subscription.status = SubscriptionStatus::Error(error.clone());
                    }

                    self.error_message = Some(format!("SDO Read Error [{:#06X}:{:02X}]: {}", address.index, address.sub_index, error));
                }
                Update::TpdoData(tpdo_data) => {
                    // Log TPDO data
                    self.logger.log(LogEvent::TpdoData {
                        tpdo_number: tpdo_data.tpdo_number,
                        values: tpdo_data.values.clone(),
                    });

                    // Store TPDO data (keep last 50 messages)
                    let now = tpdo_data.timestamp;

                    // Process each field in the TPDO for plotting
                    for (field_name, value_str) in &tpdo_data.values {
                        let field_id = TpdoFieldId {
                            tpdo_number: tpdo_data.tpdo_number,
                            field_name: field_name.clone(),
                        };

                        // Try to parse the value as a number
                        if let Ok(numeric_value) = value_str.parse::<f64>() {
                            // Get or create subscription for this field
                            let subscription = self.tpdo_field_subscriptions
                                .entry(field_id.clone())
                                .or_insert_with(|| TpdoFieldSubscription {
                                    plot_data: VecDeque::new(),
                                    last_value: None,
                                    last_timestamp: None,
                                    start_time: now,
                                });

                            // Update last value and timestamp
                            subscription.last_value = Some(value_str.clone());
                            subscription.last_timestamp = Some(now);

                            // Add to plot data
                            if subscription.plot_data.len() >= PLOT_BUFFER_SIZE {
                                subscription.plot_data.pop_front();
                            }

                            // Calculate seconds since start time for X-axis
                            let elapsed_seconds = (now - subscription.start_time).num_milliseconds() as f64 / 1000.0;
                            subscription.plot_data.push_back([elapsed_seconds, numeric_value]);
                        }
                    }

                    self.tpdo_data.push(tpdo_data);
                    if self.tpdo_data.len() > 50 {
                        self.tpdo_data.remove(0);
                    }
                }
                Update::TpdosDiscovered(tpdos) => {
                    self.discovered_tpdos = tpdos;
                }
                _ => {

                }
            }
        }

        let events = ctx.input(|i| i.events.clone());
        for event in &events {
            if let egui::Event::Screenshot { image, user_data, .. } = event {
                if let Some(info) = user_data.data.as_ref().and_then(|ud| {
                    ud.downcast_ref::<Arc<ScreenshotInfo>>().map(|arc| arc.as_ref())
                }) {
                    self.save_screenshot(image, info);
                }
            }
        }

        // This creates a central panel, which is a window that fills the entire screen.
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_view {
                AppView::SelectInterface => self.draw_interface_view(ui),
                AppView::SelectNodeId => self.draw_node_id_view(ui),
                AppView::SelectEDSFile => self.draw_eds_file_view(ui),
                AppView::Main => self.draw_main_view(ui),
            }
        });

        ctx.request_repaint();
    }
}

impl MyApp {
    /// Draws the UI for selecting the CAN interface, with centered content.
    /// Draws the UI for selecting the CAN interface using a centered window.
    fn draw_interface_view(&mut self, ui: &mut egui::Ui) {
        egui::Window::new("Interface Selection")
            .title_bar(false) // Hide the title bar for a panel look
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0)) // Anchor to the exact center
            .show(ui.ctx(), |ui| {
                // Inside the window, we can use a simpler layout.
                // This layout just centers widgets horizontally.
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_width(300.0); // Give the panel a fixed width
                    ui.heading("Step 1: Select CAN Interface");
                    ui.add_space(20.0); // Spacers will now work reliably

                    if self.available_can_interfaces.is_empty() {
                        ui.label("No CAN interfaces found.");
                        ui.add_space(10.0);
                        if ui.button("Refresh").clicked() {
                            self.available_can_interfaces = get_can_interfaces();
                        }
                    } else {
                        let selected_text = self.selected_can_interface.as_deref().unwrap_or("Click to select...");
                        egui::ComboBox::from_label("") // Label can be empty if it's clear from context
                            .selected_text(selected_text)
                            .show_ui(ui, |ui| {
                                for interface in &self.available_can_interfaces {
                                    ui.selectable_value(&mut self.selected_can_interface, Some(interface.clone()), interface);
                                }
                            });

                        ui.add_space(20.0);

                        let is_next_enabled = self.selected_can_interface.is_some();
                        if ui.add_enabled(is_next_enabled, egui::Button::new("Next ➡")).clicked() {
                            self.current_view = AppView::SelectNodeId;
                        }
                    }
                });
            });
    }

    fn draw_node_id_view(&mut self, ui: &mut egui::Ui) {
        egui::Window::new("Node ID Selection")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                // Use a simple layout that centers widgets horizontally.
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_width(300.0); // Keep the panel width consistent
                    ui.heading("Step 2: Enter Node ID");
                    ui.add_space(10.0);

                    // Show the previously selected interface for context.
                    if let Some(interface) = &self.selected_can_interface {
                        ui.label(format!("Interface: {}", interface));
                    }
                    ui.add_space(10.0);

                    // Input for the Node ID.
                    ui.horizontal(|ui| {
                        ui.label("Node ID (1-127):");
                        let response = ui.add(egui::TextEdit::singleline(&mut self.node_id_str).desired_width(50.0));

                        if response.changed() {
                            self.selected_node_id = self.node_id_str.parse::<u8>().ok().filter(|&id| (1..=127).contains(&id));
                        }
                    });

                    // Show a validation message if the input is invalid.
                    if self.selected_node_id.is_none() && !self.node_id_str.is_empty() {
                        ui.colored_label(egui::Color32::RED, "Invalid ID");
                    }
                    ui.add_space(20.0);

                    // Navigation buttons.
                    ui.horizontal(|ui| {
                        if ui.button("⬅ Back").clicked() {
                            self.current_view = AppView::SelectInterface;
                        }

                        let is_start_enabled = self.selected_node_id.is_some();
                        if ui.add_enabled(is_start_enabled, egui::Button::new("Next ➡")).clicked() {
                            self.current_view = AppView::SelectEDSFile;
                        }
                    });
                });
            });
    }

    /// Draws the UI for selecting an EDS file using a centered window.
    fn draw_eds_file_view(&mut self, ui: &mut egui::Ui) {
        egui::Window::new("EDS File Selection")
            .title_bar(false)
            .resizable(false)
            .collapsible(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ui.ctx(), |ui| {
                ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
                    ui.set_width(350.0); // A bit wider for file paths
                    ui.heading("Step 3: Select EDS File");
                    ui.add_space(10.0);

                    // Display the currently selected file path
                    let file_path_text = if let Some(path) = &self.eds_file_path {
                        path.display().to_string()
                    } else {
                        "No file selected".to_string()
                    };
                    ui.label(file_path_text);
                    ui.add_space(10.0);

                    // Button to open the native file dialog
                    if ui.button("Browse...").clicked() {
                        // Use rfd to pick a file
                        let file = rfd::FileDialog::new()
                            .add_filter("CANopen EDS", &["eds"]) // Filter for .eds files
                            .pick_file();

                        // Store the result
                        self.eds_file_path = file;
                    }
                    ui.add_space(20.0);

                    // Navigation buttons
                    ui.horizontal(|ui| {
                        if ui.button("⬅ Back").clicked() {
                            self.current_view = AppView::SelectNodeId;
                        }
                        if ui.button("🚀Start").clicked() {
                            // Update and save configuration
                            self.config.can_interface = self.selected_can_interface.clone().unwrap();
                            self.config.node_id = self.selected_node_id.unwrap();
                            self.config.eds_file_path = self.eds_file_path.as_ref().map(|p| p.display().to_string());

                            if let Err(e) = self.config.save() {
                                eprintln!("Failed to save configuration: {}", e);
                            }

                            let (command_tx, command_rx) = std::sync::mpsc::channel();
                            let (update_tx, update_rx) = std::sync::mpsc::channel();

                            self.command_tx = Some(command_tx);
                            self.update_rx = Some(update_rx);

                            let can_interface = self.selected_can_interface.clone().unwrap();
                            let node_id = self.selected_node_id.unwrap();
                            let eds_file_path = self.eds_file_path.clone();

                            std::thread::spawn(move || {
                                communication::communication_thread_main(
                                    command_rx,
                                    update_tx,
                                    can_interface,
                                    node_id,
                                    eds_file_path,
                                );
                            });
                            self.current_view = AppView::Main;
                        }
                    });
                });
            });
    }

    /// Draws the main application view.
    fn draw_main_view(&mut self, ui: &mut egui::Ui) {
        // Request connection only once at startup
        if !self.connection_requested {
            if let Some(tx) = &self.command_tx {
                tx.send(Command::Connect).unwrap();
            }
            self.connection_requested = true;
        }

        if !self.sdo_requested {
            if let Some(tx) = &self.command_tx {
                tx.send(Command::FetchSdos).unwrap();
                self.sdo_requested = true;
            }
        }

        // Auto-discover TPDOs (but don't start them) once connected and SDOs fetched
        if !self.tpdo_discovery_requested && self.connection_status && self.sdo_data.is_some() {
            if let Some(tx) = &self.command_tx {
                let _ = tx.send(Command::DiscoverTpdos);
                self.tpdo_discovery_requested = true;
            }
        }

        // Top panel for status and error display
        egui::TopBottomPanel::top("status_panel").show_inside(ui, |ui| {
            ui.horizontal(|ui| {
                // Connection status indicator
                let status_color = if self.connection_status {
                    Color32::from_rgb(0, 200, 0) // Green
                } else {
                    Color32::from_rgb(200, 0, 0) // Red
                };
                let status_text = if self.connection_status { "● Connected" } else { "● Disconnected" };
                ui.colored_label(status_color, status_text);

                ui.separator();

                // Show interface and node ID info
                if let Some(interface) = &self.selected_can_interface {
                    ui.label(format!("Interface: {}", interface));
                }
                if let Some(node_id) = self.selected_node_id {
                    ui.label(format!("Node ID: {}", node_id));
                }

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // About button
                    if ui.button("ℹ About").clicked() {
                        self.show_about_dialog = true;
                    }

                    ui.separator();

                    // Logging controls on the right side
                    if self.logger.is_enabled() {
                        if ui.button("Open Log Folder").clicked() {
                            if let Some(log_path) = self.logger.log_file_path() {
                                if let Some(parent) = log_path.parent() {
                                    let _ = open::that(parent);
                                }
                            }
                        }

                        if let Some(log_path) = self.logger.log_file_path() {
                            ui.label(format!("📝 {}", log_path.file_name().unwrap_or_default().to_string_lossy()));
                        }
                    }

                    if ui.checkbox(&mut self.config.enable_logging, "Enable Logging").changed() {
                        if self.config.enable_logging {
                            if let Some(log_dir) = self.config.get_log_directory() {
                                if let Err(e) = self.logger.enable(log_dir) {
                                    self.error_message = Some(format!("Failed to enable logging: {}", e));
                                    self.config.enable_logging = false;
                                }
                            }
                        } else {
                            self.logger.disable();
                        }
                        // Save config when logging preference changes
                        let _ = self.config.save();
                    }
                });
            });

            // Error banner
            if let Some(error_msg) = self.error_message.clone() {
                ui.separator();
                ui.horizontal(|ui| {
                    ui.colored_label(Color32::from_rgb(255, 100, 100), format!("⚠ {}", error_msg));
                    if ui.button("✖").clicked() {
                        self.error_message = None; // Clear error on click
                    }
                });
            }
        });

        // Bottom panel for subscription management
        egui::TopBottomPanel::bottom("subscription_panel").show_inside(ui, |ui| {
            self.draw_subscription_management(ui);
        });

        // Creating panels. Left panel for SDO data, right panel for graphing.
        egui::SidePanel::left("sdo_list_panel").show_inside(ui, |ui| {
            self.draw_sdo_list(ui);
        });

        // The central panel will contain the plots
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.draw_plots(ui);
        });

        self.draw_subscription_modal(ui);
        self.draw_about_dialog(ui);
    }

    fn draw_sdo_list(&mut self, ui: &mut egui::Ui) {
        // Tabs at the top
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.sidebar_tab, SidebarTab::SDO, "SDO");
            ui.selectable_value(&mut self.sidebar_tab, SidebarTab::TPDO, "TPDO");
        });
        ui.separator();

        // Render content based on selected tab
        match self.sidebar_tab {
            SidebarTab::SDO => self.draw_sdo_tab_content(ui),
            SidebarTab::TPDO => self.draw_tpdo_tab_content(ui),
        }
    }

    fn draw_sdo_tab_content(&mut self, ui: &mut egui::Ui) {
        // Search box
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.sdo_search_query);
        });
        ui.separator();

        // Scrollable list of SDOs
        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(sdo_data) = &self.sdo_data {
                let query = self.sdo_search_query.to_lowercase();
                for (index, sdo_object) in sdo_data {
                    let object_name_matches = sdo_object.name.to_lowercase().contains(&query);
                    let index_matches = format!("{:#06X}", index).to_lowercase().contains(&query);
                    let any_sub_object_matches = sdo_object.sub_objects.values()
                        .any(|sub| sub.name.to_lowercase().contains(&query));

                    if query.is_empty() || object_name_matches || index_matches || any_sub_object_matches {
                        ui.collapsing(format!("{:#06X}: {}", index, &sdo_object.name), |ui| {
                            for (sub_index, sub_object) in &sdo_object.sub_objects {
                                let address = SdoAddress { index: *index, sub_index: *sub_index };
                                let button_text = format!("Sub {}: {}", sub_index, &sub_object.name);
                                if ui.button(button_text).clicked() {
                                    self.modal_open_for = Some(address.clone());
                                    if let Some(sub) = self.subscriptions.get(&address) {
                                        self.modal_interval_str = sub.interval_ms.to_string();
                                    } else {
                                        self.modal_interval_str = "100".to_string();
                                    }
                                }
                            }
                        });
                    }
                }
            } else {
                ui.label("Fetching SDO list...");
            }
        });
    }

    fn draw_tpdo_tab_content(&mut self, ui: &mut egui::Ui) {
        // Search box
        ui.horizontal(|ui| {
            ui.label("Search:");
            ui.text_edit_singleline(&mut self.tpdo_search_query);
        });
        ui.separator();

        // Scrollable list of TPDOs
        egui::ScrollArea::vertical().show(ui, |ui| {
            if !self.connection_status {
                ui.label("Waiting for connection...");
            } else if self.discovered_tpdos.is_empty() {
                ui.label("Discovering TPDOs from device and EDS...");
                ui.label("This may take a few seconds.");
            } else {
                // Show discovered TPDOs with start/stop controls
                let query = self.tpdo_search_query.to_lowercase();

                for config in &self.discovered_tpdos.clone() {
                    let tpdo_num = config.tpdo_number;
                    let is_active = self.active_tpdos.contains(&tpdo_num);

                    // Check if this TPDO matches the search query
                    let tpdo_name = format!("TPDO {}", tpdo_num);
                    let name_matches = tpdo_name.to_lowercase().contains(&query);
                    let any_field_matches = config.mapped_objects.iter()
                        .any(|obj| obj.name.to_lowercase().contains(&query));

                    if query.is_empty() || name_matches || any_field_matches {
                        ui.collapsing(format!("TPDO {} (COB-ID: 0x{:03X})", tpdo_num, config.cob_id), |ui| {
                            // Show last update time if active and has data
                            if is_active {
                                if let Some(latest_tpdo) = self.tpdo_data.iter()
                                    .rev()
                                    .find(|tpdo| tpdo.tpdo_number == tpdo_num)
                                {
                                    ui.label(format!("Last update: {}", latest_tpdo.timestamp.format("%H:%M:%S%.3f")));
                                } else {
                                    ui.label("Status: Active (waiting for data...)");
                                }
                            } else {
                                ui.label("Status: Stopped");
                            }

                            ui.separator();

                            // Show mapped objects and their current values
                            ui.label(format!("Mapped objects ({}):", config.mapped_objects.len()));
                            ui.add_space(5.0);

                            // Get the latest TPDO data for this TPDO if active
                            let latest_values = if is_active {
                                self.tpdo_data.iter()
                                    .rev()
                                    .find(|tpdo| tpdo.tpdo_number == tpdo_num)
                                    .map(|tpdo| &tpdo.values)
                            } else {
                                None
                            };

                            for obj in &config.mapped_objects {
                                ui.horizontal(|ui| {
                                    ui.label(format!("  • {}:", obj.name));

                                    // Show current value if available
                                    if let Some(values) = latest_values {
                                        if let Some((_, value)) = values.iter().find(|(name, _)| name == &obj.name) {
                                            ui.label(value);
                                        } else {
                                            ui.label("—");
                                        }
                                    } else {
                                        ui.label("—");
                                    }
                                });
                            }

                            ui.add_space(10.0);
                            ui.separator();

                            // Start button (stop is in Active Subscriptions panel)
                            ui.horizontal(|ui| {
                                if !is_active {
                                    if ui.button("▶ Start").clicked() {
                                        // Send command to start listener
                                        if let Some(tx) = &self.command_tx {
                                            let _ = tx.send(Command::StartTpdoListener(config.clone()));
                                            self.active_tpdos.insert(tpdo_num);
                                        }
                                    }
                                } else {
                                    ui.label("(Use Active Subscriptions panel below to stop)");
                                }
                            });
                        });
                    }
                }

                if self.discovered_tpdos.is_empty() {
                    ui.add_space(10.0);
                    ui.label("No TPDOs found. The device may not have any configured.");
                }
            }
        });
    }

    fn draw_plots(&mut self, ui: &mut egui::Ui) {
        ui.heading("Plots");

        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.subscriptions.is_empty() && self.tpdo_field_subscriptions.is_empty() {
                ui.label("No active subscriptions. Select an SDO to start reading or enable TPDO plotting.");
            } else {

                // Draw SDO plots
                let mut addresses_to_clear = Vec::new();
                let mut addresses_to_export = Vec::new();

                for (address, subscription) in &self.subscriptions {
                    // 1. Use a Frame to visually group each plot and its title.
                    let mut capture_clicked = false;
                    let mut plot_title = String::new();

                    let frame_response = egui::Frame::group(ui.style()).show(ui, |ui| {
                        let plot_id = format!("sdo_plot_{:x}_{}", address.index, address.sub_index);

                        // Get human-readable name from EDS
                        let field_name = self.sdo_data.as_ref()
                            .and_then(|sdo_map| sdo_map.get(&address.index))
                            .and_then(|sdo_object| sdo_object.sub_objects.get(&address.sub_index))
                            .map(|sub_object| sub_object.name.clone())
                            .unwrap_or_else(|| format!("0x{:04X}:{:02X}", address.index, address.sub_index));

                        plot_title = format!("SDO - {} ({:#06X}:{})", field_name, address.index, address.sub_index);

                        // Add a title for the individual plot.
                        ui.label(&plot_title);
                        ui.separator();

                        Plot::new(plot_id)
                            .legend(egui_plot::Legend::default())
                            .view_aspect(2.0)
                            .allow_scroll(false)
                            .height(350.0)
                            .width(ui.available_width())
                            .x_axis_label("Time (seconds)")
                            .y_axis_label("Value")
                            .legend(Legend::default())
                            .show(ui, |plot_ui| {
                                // 2. Generate a unique color for the line based on its address.
                                let color = Color32::from_rgb(
                                    (address.index as u8).wrapping_mul(20),
                                    (address.sub_index as u8).wrapping_mul(40),
                                    (address.index as u8 ^ address.sub_index as u8).wrapping_mul(30),
                                );

                                let points_vec: Vec<[f64; 2]> = subscription.plot_data.iter().cloned().collect();

                                let line = Line::new(PlotPoints::from(points_vec))
                                    .name(&field_name)  // Use field name in legend (without hex address)
                                    .color(color);

                                plot_ui.line(line);
                            });

                        ui.horizontal(|ui| {
                            if ui.button("📸 Capture Plot").clicked() {
                                capture_clicked = true;
                            }

                            if ui.button("🗑 Clear").clicked() {
                                addresses_to_clear.push(address.clone());
                            }

                            if ui.button("💾 Export to CSV").clicked() {
                                addresses_to_export.push(address.clone());
                            }
                        });
                    });

                    // Handle capture after we have the frame rect
                    if capture_clicked {
                        let now = Local::now();
                        let timestamp = now.format("%Y-%m-%d %H:%M:%S");
                        let info = ScreenshotInfo{
                            filename: format!("{}_{}.png", plot_title.replace(":", "_"), timestamp),
                            rect: frame_response.response.rect,
                        };

                        let user_data = egui::UserData::new(Arc::new(info));
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(user_data));
                    }
                }

                for address in addresses_to_clear {
                    if let Some(subscription) = self.subscriptions.get_mut(&address) {
                        subscription.start_time = Local::now();
                        subscription.plot_data.clear();
                    }
                }

                for address in addresses_to_export {
                    self.export_plot_data_to_csv(&address);
                }

                // Draw TPDO field plots
                let mut tpdo_fields_to_clear = Vec::new();
                let mut tpdo_fields_to_export = Vec::new();

                for (field_id, subscription) in &self.tpdo_field_subscriptions {
                    let mut capture_clicked = false;
                    let mut plot_title = String::new();

                    let frame_response = egui::Frame::group(ui.style()).show(ui, |ui| {
                        let plot_id = format!("tpdo_plot_{}_{}", field_id.tpdo_number, field_id.field_name);
                        plot_title = format!("TPDO {} - {}", field_id.tpdo_number, field_id.field_name);

                        ui.label(&plot_title);
                        ui.separator();

                        Plot::new(plot_id)
                            .legend(egui_plot::Legend::default())
                            .view_aspect(2.0)
                            .allow_scroll(false)
                            .height(350.0)
                            .width(ui.available_width())
                            .x_axis_label("Time (seconds)")
                            .y_axis_label("Value")
                            .legend(Legend::default())
                            .show(ui, |plot_ui| {
                                // Generate a unique color for the line based on TPDO number and field name
                                let hash = field_id.tpdo_number as u32 * 100 + field_id.field_name.len() as u32;
                                let color = Color32::from_rgb(
                                    ((hash * 37) % 256) as u8,
                                    ((hash * 73) % 256) as u8,
                                    ((hash * 151) % 256) as u8,
                                );

                                let points_vec: Vec<[f64; 2]> = subscription.plot_data.iter().cloned().collect();

                                let line = Line::new(PlotPoints::from(points_vec))
                                    .name(&plot_title)
                                    .color(color);

                                plot_ui.line(line);
                            });

                        ui.horizontal(|ui| {
                            if ui.button("📸 Capture Plot").clicked() {
                                capture_clicked = true;
                            }

                            if ui.button("🗑 Clear").clicked() {
                                tpdo_fields_to_clear.push(field_id.clone());
                            }

                            if ui.button("💾 Export to CSV").clicked() {
                                tpdo_fields_to_export.push(field_id.clone());
                            }
                        });
                    });

                    // Handle capture after we have the frame rect
                    if capture_clicked {
                        let now = Local::now();
                        let timestamp = now.format("%Y-%m-%d %H:%M:%S");
                        let info = ScreenshotInfo{
                            filename: format!("{}_{}.png", plot_title.replace(":", "_").replace(" - ", "_"), timestamp),
                            rect: frame_response.response.rect,
                        };

                        let user_data = egui::UserData::new(Arc::new(info));
                        ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(user_data));
                    }
                }

                // Clear TPDO field plots
                for field_id in tpdo_fields_to_clear {
                    if let Some(subscription) = self.tpdo_field_subscriptions.get_mut(&field_id) {
                        subscription.start_time = Local::now();
                        subscription.plot_data.clear();
                    }
                }

                // Export TPDO field plots
                for field_id in tpdo_fields_to_export {
                    self.export_tpdo_plot_data_to_csv(&field_id);
                }
            }
        });
    }

    fn draw_subscription_management(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Active Subscriptions");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Stop All button
                let stop_all_enabled = !self.subscriptions.is_empty() || !self.active_tpdos.is_empty();
                if ui.add_enabled(stop_all_enabled, egui::Button::new("🛑 Stop All")).clicked() {
                    // Send unsubscribe commands for all active SDO subscriptions
                    if let Some(tx) = &self.command_tx {
                        for address in self.subscriptions.keys() {
                            let _ = tx.send(Command::Unsubscribe(address.clone()));
                        }
                        // Stop all TPDO listeners
                        for tpdo_num in &self.active_tpdos.clone() {
                            let _ = tx.send(Command::StopTpdoListener(*tpdo_num));
                        }
                    }
                    self.subscriptions.clear();
                    self.active_tpdos.clear();
                    // Clear TPDO field subscriptions
                    self.tpdo_field_subscriptions.clear();
                }

                // Subscription statistics
                let active_sdo_count = self.subscriptions.iter()
                    .filter(|(_, sub)| matches!(sub.status, SubscriptionStatus::Active))
                    .count();
                let error_count = self.subscriptions.iter()
                    .filter(|(_, sub)| matches!(sub.status, SubscriptionStatus::Error(_)))
                    .count();
                let active_tpdo_count = self.active_tpdos.len();

                ui.label(format!("SDO: {} | TPDO: {} | Active: {} | Errors: {}",
                    self.subscriptions.len(), active_tpdo_count, active_sdo_count, error_count));
            });
        });

        ui.separator();

        if self.subscriptions.is_empty() && self.active_tpdos.is_empty() {
            ui.label("No active subscriptions. Select an SDO or start a TPDO to begin monitoring.");
        } else {
            egui::ScrollArea::horizontal().show(ui, |ui| {
                egui::Grid::new("subscription_grid")
                    .num_columns(7)
                    .spacing([10.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        // Header row
                        ui.label("Status");
                        ui.label("Address");
                        ui.label("Data Type");
                        ui.label("Interval");
                        ui.label("Last Value");
                        ui.label("Last Update");
                        ui.label("Actions");
                        ui.end_row();

                        // Data rows - SDO subscriptions
                        let mut sdo_to_remove = Vec::new();
                        for (address, subscription) in &self.subscriptions {
                            // Status indicator with color
                            match &subscription.status {
                                SubscriptionStatus::Active => {
                                    ui.colored_label(Color32::from_rgb(0, 200, 0), "🟢 SDO");
                                },
                                SubscriptionStatus::Error(err) => {
                                    ui.colored_label(Color32::from_rgb(200, 0, 0), "🔴 SDO")
                                        .on_hover_text(err);
                                },
                                SubscriptionStatus::Idle => {
                                    ui.colored_label(Color32::from_rgb(200, 200, 0), "🟡 SDO");
                                },
                            };

                            // Address
                            ui.label(format!("{:#06X}:{:02X}", address.index, address.sub_index));

                            // Data type
                            ui.label(format!("{:?}", subscription.data_type));

                            // Interval
                            ui.label(format!("{} ms", subscription.interval_ms));

                            // Last value (truncate if too long)
                            let value_text = subscription.last_value.as_ref()
                                .map(|v| if v.len() > 20 { format!("{}...", &v[..17]) } else { v.clone() })
                                .unwrap_or_else(|| "—".to_string());
                            ui.label(value_text);

                            // Last timestamp
                            let timestamp_text = subscription.last_timestamp.as_ref()
                                .map(|t| t.format("%H:%M:%S").to_string())
                                .unwrap_or_else(|| "—".to_string());
                            ui.label(timestamp_text);

                            // Actions (Stop button)
                            if ui.button("🛑 Stop").clicked() {
                                if let Some(tx) = &self.command_tx {
                                    let _ = tx.send(Command::Unsubscribe(address.clone()));
                                }
                                sdo_to_remove.push(address.clone());
                            }
                            ui.end_row();
                        }

                        // Data rows - TPDO subscriptions
                        let mut tpdo_to_remove = Vec::new();
                        for tpdo_num in &self.active_tpdos.clone() {
                            // Status
                            ui.colored_label(Color32::from_rgb(0, 200, 0), "🟢 TPDO");

                            // Address (TPDO number)
                            ui.label(format!("TPDO {}", tpdo_num));

                            // Data type - show the config
                            if let Some(config) = self.discovered_tpdos.iter().find(|c| c.tpdo_number == *tpdo_num) {
                                ui.label(format!("{} fields", config.mapped_objects.len()));
                            } else {
                                ui.label("—");
                            }

                            // Interval (TPDOs are event-driven, not polled)
                            ui.label("Event-driven");

                            // Last value - show summary of latest TPDO data
                            if let Some(latest) = self.tpdo_data.iter().rev().find(|t| t.tpdo_number == *tpdo_num) {
                                let summary = if latest.values.len() > 2 {
                                    format!("{} values", latest.values.len())
                                } else {
                                    latest.values.iter()
                                        .map(|(_, v)| v.as_str())
                                        .collect::<Vec<_>>()
                                        .join(", ")
                                };
                                ui.label(summary);
                            } else {
                                ui.label("—");
                            }

                            // Last timestamp
                            if let Some(latest) = self.tpdo_data.iter().rev().find(|t| t.tpdo_number == *tpdo_num) {
                                ui.label(latest.timestamp.format("%H:%M:%S").to_string());
                            } else {
                                ui.label("—");
                            }

                            // Actions (Stop button)
                            if ui.button("🛑 Stop").clicked() {
                                if let Some(tx) = &self.command_tx {
                                    let _ = tx.send(Command::StopTpdoListener(*tpdo_num));
                                }
                                tpdo_to_remove.push(*tpdo_num);
                            }
                            ui.end_row();
                        }

                        // Remove stopped SDO subscriptions
                        for address in sdo_to_remove {
                            self.subscriptions.remove(&address);
                        }

                        // Remove stopped TPDO subscriptions
                        for tpdo_num in tpdo_to_remove {
                            self.active_tpdos.remove(&tpdo_num);
                            // Clear field subscriptions for this TPDO
                            self.tpdo_field_subscriptions.retain(|field_id, _| field_id.tpdo_number != tpdo_num);
                        }
                    });
            });
        }
    }

    fn draw_subscription_modal(&mut self, ui: &mut egui::Ui) {
        if let Some(address) = self.modal_open_for.clone() {
            let mut is_open = true;
            egui::Window::new("Configure SDO Subscription")
                .open(&mut is_open)
                .show(ui.ctx(), |ui| {
                    ui.label(format!("Index: {:#06X}, Sub-Index: {}", address.index, address.sub_index));

                    // Check if we are already subscribed to this address
                    if self.subscriptions.contains_key(&address) {
                        // --- Show "Stop Reading" button ---
                        if ui.button("Stop Reading").clicked() {
                            if let Some(tx) = &self.command_tx {
                                tx.send(Command::Unsubscribe(address.clone())).unwrap();
                            }
                            self.subscriptions.remove(&address);
                            self.modal_open_for = None; // Close the modal
                        }
                    } else {
                        // --- Show interval input and "Start Reading" button ---
                        ui.horizontal(|ui| {
                            ui.label("Interval (ms):");
                            ui.text_edit_singleline(&mut self.modal_interval_str);
                        });
                        if ui.button("Start Reading").clicked() {
                            if let Ok(interval_ms) = self.modal_interval_str.parse::<u64>() {
                                // Look up the data type from the EDS
                                let data_type = self.sdo_data.as_ref()
                                    .and_then(|sdo_map| sdo_map.get(&address.index))
                                    .and_then(|sdo_object| sdo_object.sub_objects.get(&address.sub_index))
                                    .and_then(|sub_object| SdoDataType::from_eds_type(&sub_object.data_type))
                                    .unwrap_or(SdoDataType::Real32); // Fallback to Real32 if type unknown

                                if let Some(tx) = &self.command_tx {
                                    tx.send(Command::Subscribe {
                                        address: address.clone(),
                                        interval_ms,
                                        data_type: data_type.clone(),
                                    }).unwrap();
                                }
                                let now = Local::now();
                                self.subscriptions.insert(address.clone(), SdoSubscription {
                                    interval_ms,
                                    plot_data: VecDeque::new(),
                                    data_type,
                                    last_value: None,
                                    last_timestamp: None,
                                    status: SubscriptionStatus::Idle,
                                    paused: false,
                                    start_time: now,
                                });
                                self.modal_open_for = None; // Close the modal
                            }
                        }
                    }
                });

            // If the user closes the window with the 'X' button
            if !is_open {
                self.modal_open_for = None;
            }
        }
    }

    fn save_screenshot(&mut self, image: &Arc<ColorImage>, info: &ScreenshotInfo) {
        if let Some(path) = rfd::FileDialog::new().set_file_name(&info.filename).save_file() {
            // Crop the full screenshot to the plot's rectangle
            let region = image.region(&info.rect, None);

            // Convert to a format the `image` crate can save
            let image_buffer = image::RgbaImage::from_raw(
                region.width() as u32,
                region.height() as u32,
                region.as_raw().to_vec(),
            ).expect("Failed to create image buffer");

            if let Err(e) = image_buffer.save(path) {
                eprintln!("Failed to save screenshot: {}", e);
            }
        }
    }

    fn export_plot_data_to_csv(&mut self, address: &SdoAddress) {
        if let Some(subscription) = self.subscriptions.get(address) {
            let file_name = format!("plot_data_{:04X}_{:02X}.csv", address.index, address.sub_index);
            if let Some(path) = rfd::FileDialog::new().set_file_name(&file_name).save_file() {
                match csv::Writer::from_path(path) {
                    Ok(mut writer) => {
                        // Write header
                        if let Err(e) = writer.write_record(&["Sample No", "Value"]) {
                            eprintln!("Failed to write CSV header: {}", e);
                        }

                        // Write data
                        for point in &subscription.plot_data {
                            if let Err(e) = writer.write_record(&[point[0].to_string(), point[1].to_string()]) {
                                eprintln!("Failed to write CSV record: {}", e);
                            }
                        }

                        if let Err(e) = writer.flush() {
                            eprintln!("Failed to flush CSV file: {}", e);
                        }
                    },
                    Err(e) => {
                        eprintln!("Failed to create CSV file: {}", e);
                    }
                }
            }
        }
    }

    fn export_tpdo_plot_data_to_csv(&mut self, field_id: &TpdoFieldId) {
        if let Some(subscription) = self.tpdo_field_subscriptions.get(field_id) {
            let file_name = format!("plot_data_tpdo{}_{}.csv", field_id.tpdo_number, field_id.field_name);
            if let Some(path) = rfd::FileDialog::new().set_file_name(&file_name).save_file() {
                match csv::Writer::from_path(path) {
                    Ok(mut writer) => {
                        // Write header
                        if let Err(e) = writer.write_record(&["Time (seconds)", "Value"]) {
                            eprintln!("Failed to write CSV header: {}", e);
                        }

                        // Write data
                        for point in &subscription.plot_data {
                            if let Err(e) = writer.write_record(&[point[0].to_string(), point[1].to_string()]) {
                                eprintln!("Failed to write CSV record: {}", e);
                            }
                        }

                        if let Err(e) = writer.flush() {
                            eprintln!("Failed to flush CSV file: {}", e);
                        }
                    },
                    Err(e) => {
                        eprintln!("Failed to create CSV file: {}", e);
                    }
                }
            }
        }
    }

    fn draw_about_dialog(&mut self, ui: &mut egui::Ui) {
        if self.show_about_dialog {
            let mut is_open = true;
            egui::Window::new("About CANopen Data Viewer")
                .open(&mut is_open)
                .resizable(false)
                .collapsible(false)
                .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
                .show(ui.ctx(), |ui| {
                    ui.set_width(400.0);

                    ui.vertical_centered(|ui| {
                        ui.heading("CANopen Data Viewer");
                        ui.add_space(10.0);

                        // Version info
                        ui.label(format!("Version: {}", APP_VERSION));

                        if GIT_HASH != "unknown" {
                            let version_details = if GIT_DIRTY == "true" {
                                format!("Git: {} ({})*", GIT_HASH, GIT_BRANCH)
                            } else {
                                format!("Git: {} ({})", GIT_HASH, GIT_BRANCH)
                            };
                            ui.label(version_details);
                        }

                        ui.label(format!("Built: {}", BUILD_TIME));
                        ui.add_space(15.0);

                        // Description
                        ui.label("CANopen Real-time Monitor & Plotter");
                        ui.label("High-performance Rust application for CANopen diagnostics");
                        ui.add_space(10.0);

                        ui.separator();
                        ui.add_space(10.0);

                        // Features
                        ui.label("✓ Real-time SDO monitoring and plotting");
                        ui.label("✓ Comprehensive subscription management");
                        ui.label("✓ Plot export (PNG screenshots and CSV data)");
                        ui.label("✓ Automatic logging with timestamps");
                        ui.label("✓ Connection status monitoring");
                        ui.add_space(10.0);

                        ui.separator();
                        ui.add_space(10.0);

                        // System info
                        ui.label("Runtime: Rust stable");
                        ui.label(format!("Platform: {}", std::env::consts::OS));
                        ui.label(format!("Architecture: {}", std::env::consts::ARCH));
                    });
                });

            if !is_open {
                self.show_about_dialog = false;
            }
        }
    }

}


fn get_can_interfaces() -> Vec<String> {
    let output = match process_command::new("ip").arg("link").arg("show").output() {
        Ok(output) => output,
        Err(_) => {
            // If the command fails (e.g., on Windows), return an empty list.
            return Vec::new();
        }
    };

    let output_str = String::from_utf8_lossy(&output.stdout);

    // Parse the output to find lines containing "can"
    output_str
        .lines()
        .filter_map(|line| {
            if line.contains(": can") {
                // The interface name is typically the second word
                line.split_whitespace().nth(1).map(|s| s.trim_end_matches(':').to_string())
            } else {
                None
            }
        })
        .collect()
}

fn main() -> Result<(), eframe::Error> {

    let native_options = NativeOptions::default();
    eframe::run_native(
        "CANopen Data Plotter",
        native_options,
        Box::new(|_cc| Ok(Box::new(MyApp::default()))),
    )
}