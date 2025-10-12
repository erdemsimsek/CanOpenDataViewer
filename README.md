# CANopen Real-time Monitor & Plotter

A high-performance, real-time CANopen diagnostics tool written in Rust, featuring dynamic plotting and configurable SDO polling.

## 📁 Project Structure

This is a Rust workspace containing multiple crates:

```
CanOpenDataViewer/
├── canopen-viewer/          # Main GUI application
├── canopen-common/          # Shared CANopen protocol library
├── mock-canopen-node/       # Mock CANopen node for testing
└── examples/                # Example binaries and EDS files
```

- **canopen-viewer**: The main application with GUI built using `egui`
- **canopen-common**: Shared library for SDO protocol (used by both viewer and mock node)
- **mock-canopen-node**: Simulated CANopen device for testing without real hardware


## Core Features

* **Real-time Plotting:** Visualize numeric TPDO and SDO data as it arrives from the CAN bus using a smooth, high-performance plot.
* **Configurable SDO Polling:** Select any SDO from a device's Object Dictionary, set a custom polling rate for each, and see the values plotted or logged in real-time.
* **Node Health Monitoring:** Automatic health checks verify that the CANopen node is alive by periodically reading the mandatory Device Type object (0x1000:00). Detects node disconnection within 4-6 seconds and updates the UI accordingly.
* **Connection Status & Error Reporting:** Clear visual indicators show whether the node is connected (green) or disconnected (red). All connection failures and SDO read errors are displayed in dismissible error banners with detailed messages.
* **Configuration Persistence:** Automatically saves and restores your last used settings (CAN interface, Node ID, EDS file path, logging preferences). No need to re-enter configuration on every startup.
* **Automatic File Logging:** Optionally log all SDO data, connection events, and errors to CSV files with timestamps. Logs are saved to `~/.local/share/canopen-viewer/logs/` by default. Enable/disable logging via the checkbox in the top panel, and open the log folder with one click.
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
4.  **Start Session:** The user clicks "Start". The CAN thread establishes the connection and begins health monitoring.
5.  **Monitor Connection:** The application automatically checks node health every 2 seconds by reading the Device Type object (0x1000:00). The UI displays:
    * **Green "● Connected"** when the node responds to health checks
    * **Red "● Disconnected"** when the node stops responding (after 2 consecutive failures)
    * **Error banners** for connection failures or SDO read errors (click "✖" to dismiss)
6.  **View Data:** As messages arrive, the application checks if they are on the user's monitoring list and handles them according to the design philosophy: plotting numeric data and logging everything else.

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
    git clone https://github.com/erdemsimsek/CanOpenDataViewer.git
    cd CanOpenDataViewer
    ```

2.  Build the workspace:
    ```bash
    # Build everything
    cargo build --workspace --release

    # Or build just the viewer
    cargo build -p canopen-viewer --release
    ```

3.  Run the mock node (for testing without hardware):
    ```bash
    # Terminal 1: Start mock CANopen node
    cargo run -p mock-canopen-node --release -- --interface vcan0 --node-id 4
    ```

4.  Run the viewer:
    ```bash
    # Terminal 2: Start the viewer application
    cargo run -p canopen-viewer --release
    ```

    Then in the GUI:
    - **First time:** Select CAN interface (`vcan0`), enter Node ID (`4`), and select EDS file (`examples/mock_node.eds`)
    - **Subsequent times:** Your last configuration will be automatically loaded - just click through the steps
    - Click "Start" - your settings will be saved automatically
    - **You should see:** Green "● Connected" status in the top panel
    - **Optional:** Enable logging with the "Enable Logging" checkbox (top-right) to record all events to CSV
    - Subscribe to SDOs via the "Subscribe to SDO" button
    - **Try it:** Stop the mock node (Ctrl+C in Terminal 1) and watch the status change to red "● Disconnected" within 4-6 seconds

## Troubleshooting

### "Network is down (os error 100)" or Connection Failed

**Problem:** Error banner appears immediately after clicking "Start" with message about network being down.

**Solution:**
1. Check if the CAN interface exists and is UP:
   ```bash
   ip link show vcan0
   ```

2. If the interface doesn't exist or is DOWN, set it up:
   ```bash
   sudo modprobe vcan
   sudo ip link add dev vcan0 type vcan
   sudo ip link set up vcan0
   ```

3. Verify the interface is UP:
   ```bash
   ip link show vcan0
   # Should show: "vcan0: <NOARP,UP,LOWER_UP> ..."
   ```

### Connection Shows "Disconnected" Even Though Mock Node is Running

**Problem:** The status indicator shows red "● Disconnected" even when the mock node is running.

**Possible causes:**
1. **Wrong Node ID:** Ensure the Node ID in the viewer matches the mock node's `--node-id` parameter (default: 4)
2. **Wrong Interface:** Ensure both applications are using the same CAN interface (e.g., `vcan0`)
3. **Mock Node Crashed:** Check Terminal 1 for errors in the mock node output
4. **Node Missing 0x1000:00:** The health check reads Device Type (0x1000:00), which must exist in the node's object dictionary

### SDO Read Errors

**Problem:** Error banner shows "SDO Read Error: 0xXXXX:YY - request failed: sdo request timeout"

**Possible causes:**
1. **Object doesn't exist:** The SDO address may not be implemented in the node
2. **Node stopped responding:** Check if the mock node is still running
3. **Wrong data type:** The data type selected in the subscription doesn't match the object's actual type in the EDS file

**Solution:**
- Verify the object exists in the EDS file and is marked as readable (`accesstype=ro` or `accesstype=rw`)
- Check that the mock node is still running and hasn't crashed
- Ensure the data type matches what's specified in the EDS file

### Application Freezes or Unresponsive

**Problem:** The UI becomes unresponsive or freezes.

**This shouldn't happen!** The application is designed with a non-blocking UI architecture. If you encounter this:
1. Check CPU usage - the application should use minimal CPU when idle
2. Report the issue with steps to reproduce at: https://github.com/erdemsimsek/CanOpenDataViewer/issues

### Logging Issues

**Problem:** Logging checkbox doesn't work or logs aren't being created.

**Solution:**
1. Check that the log directory is writable: `~/.local/share/canopen-viewer/logs/`
2. If the directory doesn't exist, the application will try to create it automatically
3. Click "Open Log Folder" button to view the log directory in your file manager
4. Log files are named: `canopen_log_YYYYMMDD_HHMMSS.csv`

**Log File Format:**
- CSV format with headers: `Timestamp, Event Type, Address, Value, Message`
- Event types: `SDO_DATA`, `SDO_ERROR`, `CONNECTION_FAILED`, `CONNECTION_STATUS`
- Open with any spreadsheet application (Excel, LibreOffice Calc, etc.)

**Configuration File Location:**
- Configuration is saved to: `~/.config/canopen-viewer/config.toml`
- You can manually edit this file if needed
- Fields: `can_interface`, `node_id`, `eds_file_path`, `enable_logging`, `log_directory`

## Roadmap

* []

## Contributing

Contributions are welcome! Please feel free to open an issue or submit a pull request.
