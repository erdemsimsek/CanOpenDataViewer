# Examples and Test Files

This directory contains example files for testing the CANopen Data Viewer.

## Files

### Example.eds

A sample EDS (Electronic Data Sheet) file for testing the viewer application.

**Usage:**

1. Start the mock CANopen node:
   ```bash
   cargo run -p mock-canopen-node --release -- --interface vcan0 --node-id 4
   ```

2. Start the viewer:
   ```bash
   cargo run -p canopen-viewer --release
   ```

3. In the viewer GUI:
   - Select interface: `vcan0`
   - Enter Node ID: `4`
   - **Select this EDS file** when prompted
   - Click "Start"

The EDS file helps the viewer understand which SDO objects are available on the node.

## Note

You can also run the viewer **without** an EDS file - it will still work, but you'll need to manually know which SDO indices to subscribe to. The mock node has objects at:
- 0x1000:00, 0x1001:00, 0x1008:00, 0x1018:01
- 0x2000:01, 0x2000:02, 0x2001:01
- 0x2002:01, 0x2002:02
- 0x2003:01, 0x2003:02
- 0x2004:01, 0x2005:01

See `mock-canopen-node/README.md` for the complete list.
