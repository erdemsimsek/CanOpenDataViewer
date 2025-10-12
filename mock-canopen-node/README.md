# Mock CANopen Node

A simulated CANopen device for testing the CANopen Data Viewer without real hardware.

## Features

- Responds to SDO upload requests on the CAN bus
- Configurable object dictionary with 14+ test objects
- Supports both static and dynamic values
- Simulates realistic changing sensor data (temperature, pressure, voltage, etc.)
- Perfect for development, testing, and CI/CD

## Usage

### Basic Usage

```bash
# Run with default settings (vcan0, node ID 4)
cargo run -p mock-canopen-node --release

# Custom interface and node ID
cargo run -p mock-canopen-node --release -- --interface can0 --node-id 10
```

### Command Line Arguments

- `--interface <name>`: CAN interface to use (default: `vcan0`)
- `--node-id <id>`: CANopen node ID 1-127 (default: `4`)

## Available Test Objects

The mock node provides the following SDO objects:

### Standard CANopen Objects

| Index | Sub | Type | Access | Value | Description |
|-------|-----|------|--------|-------|-------------|
| 0x1000 | 0x00 | UInt32 | RO | 0x00000191 | Device Type |
| 0x1001 | 0x00 | UInt8 | RO | 0x00 | Error Register |
| 0x1008 | 0x00 | String | RO | "MockCANopenNode" | Device Name |
| 0x1018 | 0x01 | UInt32 | RO | 0x00000001 | Vendor ID |

### Dynamic Sensor Simulations

| Index | Sub | Type | Range | Description |
|-------|-----|------|-------|-------------|
| 0x2000 | 0x01 | Real32 | 20.0 - 30.0 | Temperature (°C) |
| 0x2000 | 0x02 | Real32 | 95.0 - 105.0 | Pressure (kPa) |
| 0x2001 | 0x01 | UInt32 | 0+ (incrementing) | Counter |
| 0x2002 | 0x01 | Real32 | 11.5 - 12.5 | Voltage (V) |
| 0x2002 | 0x02 | Real32 | 0.5 - 5.0 | Current (A) |
| 0x2004 | 0x01 | Int32 | 1000 - 3000 | RPM |
| 0x2005 | 0x01 | Int32 | 0+ (incrementing) | Position |

### Static Status Words

| Index | Sub | Type | Value | Description |
|-------|-----|------|-------|-------------|
| 0x2003 | 0x01 | UInt16 | 0x0031 | Status Word |
| 0x2003 | 0x02 | UInt16 | 0x000F | Control Word |

## Example Session

```bash
$ cargo run -p mock-canopen-node --release -- --interface vcan0 --node-id 4

🤖 Mock CANopen Node Starting...
   Interface: vcan0
   Node ID: 4

✓ CAN socket opened successfully
✓ Object dictionary loaded with 14 objects

📋 Available SDO Objects:
  0x1000:00 - Static UInt32
  0x1001:00 - Static UInt8
  0x1008:00 - Static VisibleString
  0x1018:01 - Static UInt32
  0x2000:01 - Dynamic Real32
  0x2000:02 - Dynamic Real32
  0x2001:01 - Dynamic UInt32
  0x2002:01 - Dynamic Real32
  0x2002:02 - Dynamic Real32
  0x2003:01 - Static UInt16
  0x2003:02 - Static UInt16
  0x2004:01 - Dynamic Int32
  0x2005:01 - Dynamic Int32

🚀 Mock node is running!
   Waiting for SDO requests on COB-ID 0x604...
   Press Ctrl+C to stop

📥 SDO Upload Request: Index=0x2000, SubIndex=0x01
📤 SDO Response: Value=24.73 (type=Real32)
📥 SDO Upload Request: Index=0x2001, SubIndex=0x01
📤 SDO Response: Value=42 (type=UInt32)
...
```

## Architecture

### Components

1. **main.rs**: Entry point and CAN message loop
2. **object_dictionary.rs**: Object dictionary implementation with static/dynamic values
3. **sdo_server.rs**: SDO protocol handling (request parsing, response generation)

### How It Works

1. Listens for CAN frames on COB-ID `0x600 + node_id` (SDO upload requests)
2. Parses the SDO request (index, subindex)
3. Looks up the value in the object dictionary
4. Generates appropriate SDO response frame
5. Sends response on COB-ID `0x580 + node_id`

### SDO Protocol

The mock node implements the **expedited SDO upload protocol** (for data ≤ 4 bytes):

**Request** (from client):
```
COB-ID: 0x600 + node_id
Data:   [0x40, index_lo, index_hi, subindex, 0, 0, 0, 0]
```

**Response** (from mock node):
```
COB-ID: 0x580 + node_id
Data:   [cmd, index_lo, index_hi, subindex, data[0], data[1], data[2], data[3]]
```

Where `cmd` encodes the data size:
- `0x4F`: 1 byte of data
- `0x4B`: 2 bytes of data
- `0x47`: 3 bytes of data
- `0x43`: 4 bytes of data

## Customization

### Adding New Objects

Edit `src/object_dictionary.rs` in the `add_test_objects()` method:

```rust
// Add a static value
self.add_static(
    0x3000,  // index
    0x01,    // subindex
    vec![0x42, 0x00],  // data bytes
    SdoDataType::UInt16,
);

// Add a dynamic (changing) value
self.add_dynamic(
    0x3001,  // index
    0x01,    // subindex
    || {
        let mut rng = rand::rng();
        let value: f32 = rng.gen_range(0.0..100.0);
        value.to_le_bytes().to_vec()
    },
    SdoDataType::Real32,
);
```

### Supported Data Types

- `UInt8`, `UInt16`, `UInt32`
- `Int8`, `Int16`, `Int32`
- `Real32` (f32)
- `VisibleString`
- `OctetString`

## Testing with the Viewer

1. **Terminal 1**: Start the mock node
   ```bash
   cargo run -p mock-canopen-node --release -- --interface vcan0 --node-id 4
   ```

2. **Terminal 2**: Start the viewer
   ```bash
   cargo run -p canopen-viewer --release
   ```

3. In the viewer GUI:
   - Select interface: `vcan0`
   - Enter node ID: `4`
   - Click "Start"
   - Subscribe to any of the test SDOs (e.g., 0x2000:01 for temperature)

You should see the mock node logging requests and the viewer plotting the simulated data in real-time!

## Limitations

- Only supports **expedited SDO uploads** (data ≤ 4 bytes)
- No segmented transfer support
- No SDO download (write) support
- No PDO support
- No NMT/heartbeat support

These limitations are intentional - the mock node is designed to be simple and sufficient for testing the viewer's core functionality.

## License

Same as parent project.
