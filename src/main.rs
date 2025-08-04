// main.rs

mod communication;

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::ops::Deref;
use communication::{Command, Update, SdoAddress, SdoObject};

use eframe::{egui, NativeOptions, egui::Color32, egui::ColorImage};
use std::process::Command as process_command;
use std::path::PathBuf;
use std::sync::mpsc::{Sender, Receiver};
use egui_plot::{Plot, PlotPoints, Line, Legend};
use chrono::Local;
use std::sync::Arc;

const PLOT_BUFFER_SIZE: usize = 500;

enum AppView {
    SelectInterface,
    SelectNodeId,
    SelectEDSFile,
    Main
}

#[derive(Debug, Clone)]
struct SdoSubscription{
    interval_ms: u64,
    plot_data: VecDeque<[f64; 2]>,
}
struct ScreenshotInfo {
    filename: String,
    rect: egui::Rect,
}

impl ScreenshotInfo {
    fn new(file_name: String, rect: egui::Rect) -> Self {
        Self {
            filename: file_name,
            rect,
        }
    }
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

    sdo_requested : bool,
    sdo_data : Option<BTreeMap<u16, SdoObject>>,

    // Storing the state of all active subscriptions
    subscriptions : HashMap<SdoAddress, SdoSubscription>,

    // Managing the state of the pop-up configuration modal
    modal_open_for: Option<SdoAddress>,
    modal_interval_str: String,

    sdo_search_query: String
}


impl Default for MyApp {
    fn default() -> Self {
        Self {
            current_view: AppView::SelectInterface,
            available_can_interfaces: get_can_interfaces(),
            selected_can_interface: None,
            selected_node_id: None,
            node_id_str: String::new(),
            eds_file_path: None,

            command_tx: None,
            update_rx: None,

            sdo_requested: false,
            sdo_data: None,

            subscriptions: HashMap::new(),

            modal_open_for: None,
            modal_interval_str: String::new(),

            sdo_search_query: String::new()
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
                    // 1. Try to parse the incoming string value into a number.
                    if let Ok(number_value) = value.parse::<f64>() {
                        // 2. Find the subscription this data belongs to.
                        if let Some(subscription) = self.subscriptions.get_mut(&address) {

                            if subscription.plot_data.len() >= PLOT_BUFFER_SIZE {
                                subscription.plot_data.pop_front();
                            }
                            let time = subscription.plot_data.back().map_or(0.0, |p| p[0] + 1.0);
                            subscription.plot_data.push_back([time, number_value]);
                        }
                    }
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
                        if ui.add_enabled(is_next_enabled, egui::Button::new("Next →")).clicked() {
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
                        if ui.button("← Back").clicked() {
                            self.current_view = AppView::SelectInterface;
                        }

                        let is_start_enabled = self.selected_node_id.is_some();
                        if ui.add_enabled(is_start_enabled, egui::Button::new("Next →")).clicked() {
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
                        if ui.button("← Back").clicked() {
                            self.current_view = AppView::SelectNodeId;
                        }
                        if ui.button("Start").clicked() {
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
        if !self.sdo_requested {
            if let Some(tx) = &self.command_tx {
                tx.send(Command::FetchSdos).unwrap();
                self.sdo_requested = true;
            }
        }

        // Creating panels. Left panel for SDO data, right panel for graphing.
        egui::SidePanel::left("sdo_list_panel").show_inside(ui, |ui| {
            self.draw_sdo_list(ui);
        });

        // The central panel will contain the plots
        egui::CentralPanel::default().show_inside(ui, |ui| {
            self.draw_plots(ui);
        });

        self.draw_subscription_modal(ui);
    }

    fn draw_sdo_list(&mut self, ui: &mut egui::Ui) {
        ui.heading("SDO List");

        ui.horizontal(|ui| {
           ui.label("Search:");
            ui.text_edit_singleline(&mut self.sdo_search_query);
        });
        ui.separator();

        egui::ScrollArea::vertical().show(ui, |ui| {
            if let Some(sdo_data) = &self.sdo_data {
                let query = self.sdo_search_query.to_lowercase();
                for (index, sdo_object) in sdo_data {
                    let object_name_matches = sdo_object.name.to_lowercase().contains(&query);
                    let any_sub_object_matches = sdo_object.sub_objects.values()
                        .any(|sub| sub.name.to_lowercase().contains(&query));

                    if query.is_empty() || object_name_matches || any_sub_object_matches {
                        ui.collapsing(format!("{:#06X}: {}", index, &sdo_object.name), |ui| {
                            for (sub_index, sub_object) in &sdo_object.sub_objects {
                                let address = SdoAddress { index: *index, sub_index: *sub_index };
                                // Change the label to a button
                                let button_text = format!("Sub {}: {}", sub_index, &sub_object.name);
                                if ui.button(button_text).clicked() {
                                    // When clicked, open the modal for this specific SDO sub-object
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

    fn draw_plots(&mut self, ui: &mut egui::Ui) {
        ui.heading("Plots");

        egui::ScrollArea::vertical().show(ui, |ui| {
            if self.subscriptions.is_empty() {
                ui.label("No active subscriptions. Select an SDO to start reading.");
            } else {
                for (address, subscription) in &self.subscriptions {
                    // 1. Use a Frame to visually group each plot and its title.
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        let plot_id = format!("sdo_plot_{:x}_{}", address.index, address.sub_index);
                        let plot_title = format!("SDO {:#06X}:{}", address.index, address.sub_index);

                        // Add a title for the individual plot.
                        ui.label(&plot_title);
                        ui.separator();

                        let plot_response = Plot::new(plot_id)
                            .legend(egui_plot::Legend::default())
                            .view_aspect(2.0)
                            .allow_scroll(false)
                            .height(350.0)
                            .width(ui.available_width())
                            .x_axis_label("Sample No")
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
                                    .name(&plot_title)
                                    .color(color);

                                plot_ui.line(line);
                            });

                        if ui.button("Save Plot").clicked() {
                            let now = Local::now();
                            let timestamp = now.format("%Y-%m-%d %H:%M:%S");
                            let info = ScreenshotInfo{
                                filename: format!("{}_{}.png", plot_title.replace(":", "_"), timestamp),
                                // rect: plot_response.response.rect,
                                rect: ui.min_rect(),
                            };

                            let user_data = egui::UserData::new(Arc::new(info));
                            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Screenshot(user_data));
                        }
                    });
                }
            }
        });
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
                                if let Some(tx) = &self.command_tx {
                                    tx.send(Command::Subscribe { address: address.clone(), interval_ms }).unwrap();
                                }
                                self.subscriptions.insert(address.clone(), SdoSubscription {
                                    interval_ms,
                                    plot_data: VecDeque::new(),
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