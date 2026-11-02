# Zellij Socktop Plugin

A Zellij plugin that displays real-time system metrics from a socktop agent.

Built against `zellij-tile` 0.45, so it needs a Zellij 0.45.x runtime (plugins
are version-checked). Zellij plugins target `wasm32-unknown-unknown`.

## Quick Start

1. **Build the plugin:**
   ```bash
   cargo build --target wasm32-unknown-unknown --release
   ```

2. **Install in Zellij:**
   ```bash
   # Copy the WASM file to Zellij plugins directory
   mkdir -p ~/.config/zellij/plugins
   cp target/wasm32-unknown-unknown/release/zellij_socktop_plugin.wasm ~/.config/zellij/plugins/socktop.wasm
   cp plugin.yaml ~/.config/zellij/plugins/
   ```

3. **Use in Zellij layout:**
   ```yaml
   # ~/.config/zellij/layouts/socktop.yaml
   template:
     direction: Horizontal
     parts:
       - direction: Vertical
         borderless: true
         split_size:
           Fixed: 1
         run:
           plugin:
             location: "file:~/.config/zellij/plugins/socktop.wasm"
             configuration:
               server_url: "ws://localhost:3000/ws"
       - direction: Vertical
   ```

4. **Launch Zellij with the layout:**
   ```bash
   zellij --layout socktop
   ```

## Plugin Features

- **Real-time Metrics**: Displays CPU and memory usage
- **Auto-refresh**: Updates every 2 seconds
- **Reconnection**: Press 'r' to reconnect to socktop agent
- **Configurable**: Set custom server URL in plugin config
- **Error Handling**: Shows connection status and errors

## Configuration Options

- `server_url`: WebSocket URL for socktop agent (default: `ws://localhost:3000/ws`)

## Controls

- **`r`** - Reconnect to socktop agent
- Plugin updates automatically every 2 seconds

## Development Notes

This is a scaffold implementation. To make it fully functional:

1. **Async Operations**: Zellij plugins have limitations with async operations. You may need to:
   - Use a different async runtime or approach
   - Handle WebSocket connections in a background thread
   - Use message passing between threads

2. **Error Handling**: Add more robust error handling for:
   - Network connectivity issues
   - Invalid server URLs
   - Agent unavailability

3. **UI Improvements**: 
   - Add more detailed metrics display
   - Implement scrolling for large datasets
   - Add color coding for status indicators

4. **Performance**: 
   - Implement caching to reduce agent requests
   - Add configurable update intervals
   - Optimize WASM binary size

## Dependencies

- `zellij-tile`: Zellij plugin framework
- `socktop_connector`: WebSocket connector with WASM support
- `serde`: JSON serialization
- `chrono`: Time handling (WASM-compatible)

## Building

```bash
# Add WASM target
rustup target add wasm32-unknown-unknown

# Build for WASM
cargo build --target wasm32-unknown-unknown --release

# The plugin will be at: target/wasm32-unknown-unknown/release/zellij_socktop_plugin.wasm
```
