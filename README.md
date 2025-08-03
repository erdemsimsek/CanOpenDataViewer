# CANopen Real-time Monitor & Plotter


A high-performance, real-time CANopen diagnostics tool written in Rust, featuring dynamic plotting, configurable SDO polling, and selective TPDO monitoring.


## Core Features

* **Real-time Plotting:** Visualize numeric TPDO and SDO data as it arrives from the CAN bus using a smooth, high-performance plot.
* **Configurable SDO Polling:** Select any SDO from a device's Object Dictionary, set a custom polling rate for each, and see the values plotted or logged in real-time.
* **Selective TPDO Monitoring:** The UI automatically lists all available Transmit-PDOs from a device profile. Simply check the ones you want to monitor.
* **Intelligent Data Handling:**
    * **Numeric** PDO data is automatically sent to the real-time plot.
    * **String** or other non-numeric PDO data is displayed in the live log window.
* **Device Profile Parsing:** Load device capabilities and its Object Dictionary directly from standard `EDS` or `XDD` files.
* **Concurrent & Performant:** Built in Rust to be memory-safe and highly performant, ensuring the UI never freezes, even under heavy bus load.
* **Cross-Platform:** Built with `egui`, allowing it to run on Linux, Windows, and macOS.

## Design Philosophy

This tool is built with a few core principles in mind, designed to create a robust and intuitive experience for developers and engineers working with CANopen.

### 1. Performance and Safety First

The choice of **Rust** is deliberate. For a tool that interacts with hardware and handles high-frequency real-time data, Rust's guarantees of memory safety and its "fearless concurrency" are paramount. This eliminates entire classes of bugs common in systems-level C/C++ applications and provides performance that is critical for real-time monitoring.

### 2. A Non-Blocking, Responsive UI

The application is architected around a **multi-threaded model**:
* A dedicated **CAN Thread** handles all blocking I/O with the CAN bus. It listens for messages, sends SDO requests, and parses data. This thread is allowed to block and wait for hardware.
* The **Main UI Thread** runs the `egui` event loop and is responsible only for drawing the UI and reacting to user input. It **never blocks**.

Communication between these threads is handled by lock-free **mpsc channels**, ensuring that data flows from the hardware to the UI efficiently and safely without ever compromising the responsiveness of the interface.

### 3. Immediate Mode for Dynamic Data

The UI is built with **`egui`**, an immediate mode GUI framework. This paradigm is a perfect match for a real-time diagnostics tool. Instead of building a complex tree of UI objects and managing their state, the UI is simply redrawn every frame based on the latest available data. This dramatically simplifies the logic for updating plots and logs, making the code easier to reason about and extend.

### 4. Data-Driven Display

The tool is designed to be "smart" about the data it receives. The core philosophy is to **visualize what can be visualized, and log what cannot**. By inspecting the data type of an object from the device profile, the application automatically routes it to the most appropriate display:
* **Numbers, floats, integers?** These are best understood on a graph over time. They are sent to the plotter.
* **Strings, byte arrays?** These have no simple numeric value. They are sent to the log viewer with a timestamp.

This ensures the user is always presented with information in its most useful form.

## How It Works

1.  **Load Profile:** The user starts by loading an `EDS` or `XDD` file for the target device.
2.  **Populate UI:** The application parses the file and populates the UI with two key lists: all available SDOs and all available TPDOs.
3.  **Configure Monitoring:**
    * For **TPDOs**, the user simply checks a box next to each PDO they wish to monitor.
    * For **SDOs**, the user selects an SDO and enters a polling interval in milliseconds (e.g., `250ms`).
4.  **Start Session:** The user clicks "Connect". The CAN thread starts, begins polling the configured SDOs at their specified rates, and listens for all incoming TPDOs.
5.  **View Data:** As messages arrive, the application checks if they are on the user's monitoring list and handles them according to the design philosophy: plotting numeric data and logging everything else.

## Technology Stack

* **Language:** [Rust](https://www.rust-lang.org/)
* **UI Framework:** [egui](https://github.com/emilk/egui) & [eframe](https://github.com/emilk/eframe_template)
* **Plotting:** [egui_plot](https://crates.io/crates/egui_plot)
* **CAN Interface:** [socketcan](https://crates.io/crates/socketcan) (for Linux)
* **CANopen Protocol:** [canopen](https://crates.io/crates/canopen)
* **Device Profile Parsing:** [configparser](https://crates.io/crates/rust-ini) (for EDS)
* **Logging:** [csv](https://crates.io/crates/csv)

## Getting Started

### Prerequisites

* Rust toolchain (`rustup`, `cargo`)
* On Linux: A SocketCAN interface (e.g., `vcan0` for virtual testing, or a physical CAN adapter).
    * To set up a virtual CAN bus:
        ```bash
        sudo modprobe vcan
        sudo ip link add dev vcan0 type vcan
        sudo ip link set up vcan0
        ```

### Building and Running

1.  Clone the repository:
    ```bash
    git clone [https://github.com/erdemsimsek/CanOpenDataViewer.git](https://github.com/erdemsimsek/CanOpenDataViewer.git)
    cd CanOpenDataViewer
    ```

2.  Build and run the application:
    ```bash
    cargo run --release
    ```

## Roadmap

* []

## Contributing

Contributions are welcome! Please feel free to open an issue or submit a pull request.
