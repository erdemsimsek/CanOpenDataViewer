// main.rs

use eframe::{egui, NativeOptions};
use std::process::Command;


enum AppView {
    SelectInterface,
    SelectNodeId,
    Main
}
struct MyApp {
    current_view: AppView,
    available_can_interfaces: Vec<String>,
    selected_can_interface: Option<String>,
    selected_node_id: Option<u8>,
    node_id_str : String,
}


impl Default for MyApp {
    fn default() -> Self {
        Self {
            current_view: AppView::SelectInterface,
            available_can_interfaces: get_can_interfaces(),
            selected_can_interface: None,
            selected_node_id: None,
            node_id_str: String::new(),
        }
    }
}

impl eframe::App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // This creates a central panel, which is a window that fills the entire screen.
        egui::CentralPanel::default().show(ctx, |ui| {
            match self.current_view {
                AppView::SelectInterface => self.draw_interface_view(ui),
                AppView::SelectNodeId => self.draw_node_id_view(ui),
                AppView::Main => self.draw_main_view(ui)
            }
        });
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
                        if ui.add_enabled(is_start_enabled, egui::Button::new("Start")).clicked() {
                            self.current_view = AppView::Main;
                        }
                    });
                });
            });
    }

    /// Draws the main application view.
    fn draw_main_view(&mut self, ui: &mut egui::Ui) {
        ui.heading("Main Application");
        ui.add_space(20.0);

        // Safely unwrap and display the final selections.
        if let (Some(interface), Some(node_id)) = (&self.selected_can_interface, self.selected_node_id) {
            ui.label("Successfully configured! ✅");
            ui.add_space(10.0);
            ui.label(format!("Listening on interface: {}", interface));
            ui.label(format!("Targeting Node ID: {}", node_id));
            ui.add_space(20.0);
            ui.label("This is where the data plots and tables will go.");
        }
    }
}


fn get_can_interfaces() -> Vec<String> {
    let output = match Command::new("ip").arg("link").arg("show").output() {
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