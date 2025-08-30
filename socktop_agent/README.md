# socktop_agent (server)

Lightweight on‑demand metrics WebSocket server for the socktop TUI.

Highlights:
- Collects system metrics only when requested (keeps idle CPU <1%)
- Optional TLS (self‑signed cert auto‑generated & pinned by client)
- JSON for fast metrics / disks; protobuf (optionally gzipped) for processes
- Accurate per‑process CPU% on Linux via /proc jiffies delta
- Optional GPU & temperature metrics (disable via env vars)
- Simple token auth (?token=...) support

Run (no TLS):
```
cargo install socktop_agent
socktop_agent --port 3000
```
Enable TLS:
```
SOCKTOP_ENABLE_SSL=1 socktop_agent --port 8443
# cert/key stored under $XDG_DATA_HOME/socktop_agent/tls
```
Environment toggles:
- SOCKTOP_AGENT_GPU=0      (disable GPU collection)
- SOCKTOP_AGENT_TEMP=0     (disable temperature)
- SOCKTOP_TOKEN=secret     (require token param from client)
- SOCKTOP_AGENT_METRICS_TTL_MS=250 (cache fast metrics window)
- SOCKTOP_AGENT_PROCESSES_TTL_MS=1000
- SOCKTOP_AGENT_DISKS_TTL_MS=1000

Systemd unit example & full docs:
https://github.com/jasonwitty/socktop

## WebSocket API Integration Guide

The socktop_agent exposes a WebSocket API that can be directly integrated with your own applications. This allows you to build custom monitoring dashboards or analysis tools using the agent's metrics.

### WebSocket Endpoint

```
ws://HOST:PORT/ws         # Without TLS
wss://HOST:PORT/ws        # With TLS
```

With authentication token (if configured):
```
ws://HOST:PORT/ws?token=YOUR_TOKEN
wss://HOST:PORT/ws?token=YOUR_TOKEN
```

### Communication Protocol

All communication uses JSON format for requests and responses, except for the process list which uses Protocol Buffers (protobuf) format with optional gzip compression.

#### Request Types

Send a JSON message with a `type` field to request specific metrics:

```json
{"type": "metrics"}       // Request fast-changing metrics (CPU, memory, network)
{"type": "disks"}         // Request disk information
{"type": "processes"}     // Request process list (returns protobuf)
```

#### Response Formats

1. **Fast Metrics** (JSON):

```json
{
  "cpu_total": 12.4,
  "cpu_per_core": [11.2, 15.7],
  "mem_total": 33554432,
  "mem_used": 18321408,
  "swap_total": 0,
  "swap_used": 0,
  "hostname": "myserver",
  "cpu_temp_c": 42.5,
  "networks": [{"name":"eth0","received":12345678,"transmitted":87654321}],
  "gpus": [{"name":"nvidia-0","usage":56.7,"memory_total":8589934592,"memory_used":1073741824,"temp_c":65.0}]
}
```

2. **Disks** (JSON):

```json
[
  {"name":"nvme0n1p2","total":512000000000,"available":320000000000},
  {"name":"sda1","total":1000000000000,"available":750000000000}
]
```

3. **Processes** (Protocol Buffers):

Processes are returned in Protocol Buffers format, optionally gzip-compressed for large process lists. The protobuf schema is:

```protobuf
syntax = "proto3";

message Process {
  uint32 pid = 1;
  string name = 2;
  float cpu_usage = 3;
  uint64 mem_bytes = 4;
}

message ProcessList {
  uint32 process_count = 1;
  repeated Process processes = 2;
}
```

### Example Integration (JavaScript/Node.js)

```javascript
const WebSocket = require('ws');

// Connect to the agent
const ws = new WebSocket('ws://localhost:3000/ws');

ws.on('open', function open() {
  console.log('Connected to socktop_agent');
  
  // Request metrics immediately on connection
  ws.send(JSON.stringify({type: 'metrics'}));
  
  // Set up regular polling
  setInterval(() => {
    ws.send(JSON.stringify({type: 'metrics'}));
  }, 1000);
  
  // Request processes every 3 seconds
  setInterval(() => {
    ws.send(JSON.stringify({type: 'processes'}));
  }, 3000);
});

ws.on('message', function incoming(data) {
  // Check if the response is JSON or binary (protobuf)
  try {
    const jsonData = JSON.parse(data);
    console.log('Received JSON data:', jsonData);
  } catch (e) {
    console.log('Received binary data (protobuf), length:', data.length);
    // Process binary protobuf data with a library like protobufjs
  }
});

ws.on('close', function close() {
  console.log('Disconnected from socktop_agent');
});
```

### Example Integration (Python)

```python
import json
import asyncio
import websockets

async def monitor_system():
    uri = "ws://localhost:3000/ws"
    async with websockets.connect(uri) as websocket:
        print("Connected to socktop_agent")
        
        # Request initial metrics
        await websocket.send(json.dumps({"type": "metrics"}))
        
        # Set up regular polling
        while True:
            # Request metrics
            await websocket.send(json.dumps({"type": "metrics"}))
            
            # Receive and process response
            response = await websocket.recv()
            
            # Check if response is JSON or binary (protobuf)
            try:
                data = json.loads(response)
                print(f"CPU: {data['cpu_total']}%, Memory: {data['mem_used']/data['mem_total']*100:.1f}%")
            except json.JSONDecodeError:
                print(f"Received binary data, length: {len(response)}")
                # Process binary protobuf data with a library like protobuf
            
            # Wait before next poll
            await asyncio.sleep(1)

asyncio.run(monitor_system())
```

### Notes for Integration

1. **Error Handling**: The WebSocket connection may close unexpectedly; implement reconnection logic in your client.

2. **Rate Limiting**: Avoid excessive polling that could impact the system being monitored. Recommended intervals:
   - Metrics: 500ms or slower
   - Processes: 2000ms or slower
   - Disks: 5000ms or slower

3. **Authentication**: If the agent is configured with a token, always include it in the WebSocket URL.

4. **Protocol Buffers Handling**: For processing the binary process list data, use a Protocol Buffers library for your language and the schema provided in the `proto/processes.proto` file.

5. **Compression**: Process lists may be gzip-compressed. Check if the response starts with the gzip magic bytes (`0x1f, 0x8b`) and decompress if necessary.

## LLM Integration Guide

If you're using an LLM to generate code for integrating with socktop_agent, this section provides structured information to help the model understand the API better.

### API Schema

```yaml
# WebSocket API Schema for socktop_agent
endpoint: ws://HOST:PORT/ws or wss://HOST:PORT/ws (with TLS)
authentication: 
  type: query parameter
  parameter: token
  example: ws://HOST:PORT/ws?token=YOUR_TOKEN

requests:
  - type: metrics
    format: JSON
    example: {"type": "metrics"}
    description: Fast-changing metrics (CPU, memory, network)
    
  - type: disks
    format: JSON
    example: {"type": "disks"}
    description: Disk information
    
  - type: processes
    format: JSON
    example: {"type": "processes"}
    description: Process list (returns protobuf)

responses:
  - request_type: metrics
    format: JSON
    schema:
      cpu_total: float # percentage of total CPU usage
      cpu_per_core: [float] # array of per-core CPU usage percentages
      mem_total: uint64 # total memory in bytes
      mem_used: uint64 # used memory in bytes
      swap_total: uint64 # total swap in bytes
      swap_used: uint64 # used swap in bytes
      hostname: string # system hostname
      cpu_temp_c: float? # CPU temperature in Celsius (optional)
      networks: [
        {
          name: string # network interface name
          received: uint64 # total bytes received
          transmitted: uint64 # total bytes transmitted
        }
      ]
      gpus: [
        {
          name: string # GPU device name
          usage: float # GPU usage percentage
          memory_total: uint64 # total GPU memory in bytes
          memory_used: uint64 # used GPU memory in bytes
          temp_c: float # GPU temperature in Celsius
        }
      ]?
  
  - request_type: disks
    format: JSON
    schema:
      [
        {
          name: string # disk name
          total: uint64 # total space in bytes
          available: uint64 # available space in bytes
        }
      ]
  
  - request_type: processes
    format: Protocol Buffers (optionally gzip-compressed)
    schema: See protobuf definition below
```

### Protobuf Schema (processes.proto)

```protobuf
syntax = "proto3";

message Process {
  uint32 pid = 1;
  string name = 2;
  float cpu_usage = 3;
  uint64 mem_bytes = 4;
}

message ProcessList {
  uint32 process_count = 1;
  repeated Process processes = 2;
}
```

### Step-by-Step Integration Pseudocode

```
1. Establish WebSocket connection to ws://HOST:PORT/ws
   - Add token if required: ws://HOST:PORT/ws?token=YOUR_TOKEN
   
2. For regular metrics updates:
   - Send: {"type": "metrics"}
   - Parse JSON response
   - Extract CPU, memory, network info
   
3. For disk information:
   - Send: {"type": "disks"}
   - Parse JSON response
   - Extract disk usage data
   
4. For process list:
   - Send: {"type": "processes"}
   - Check if response is binary
   - If starts with 0x1f, 0x8b bytes:
     - Decompress using gzip
   - Parse binary data using protobuf schema
   - Extract process information
   
5. Implement reconnection logic:
   - On connection close/error
   - Use exponential backoff
   
6. Respect rate limits:
   - metrics: ≥ 500ms interval
   - disks: ≥ 5000ms interval
   - processes: ≥ 2000ms interval
```

### Common Implementation Patterns

**Pattern 1: Periodic Polling**
```javascript
// Set up separate timers for different metric types
const metricsInterval = setInterval(() => ws.send(JSON.stringify({type: 'metrics'})), 500);
const disksInterval = setInterval(() => ws.send(JSON.stringify({type: 'disks'})), 5000);
const processesInterval = setInterval(() => ws.send(JSON.stringify({type: 'processes'})), 2000);

// Clean up on disconnect
ws.on('close', () => {
  clearInterval(metricsInterval);
  clearInterval(disksInterval);
  clearInterval(processesInterval);
});
```

**Pattern 2: Processing Binary Protobuf Data**
```javascript
// Using protobufjs
const root = protobuf.loadSync('processes.proto');
const ProcessList = root.lookupType('ProcessList');

ws.on('message', function(data) {
  if (typeof data !== 'string') {
    // Check for gzip compression
    if (data[0] === 0x1f && data[1] === 0x8b) {
      data = gunzipSync(data); // Use appropriate decompression library
    }
    
    // Decode protobuf
    const processes = ProcessList.decode(new Uint8Array(data));
    console.log(`Total processes: ${processes.process_count}`);
    processes.processes.forEach(p => {
      console.log(`PID: ${p.pid}, Name: ${p.name}, CPU: ${p.cpu_usage}%`);
    });
  }
});
```

**Pattern 3: Reconnection Logic**
```javascript
function connect() {
  const ws = new WebSocket('ws://localhost:3000/ws');
  
  ws.on('open', () => {
    console.log('Connected');
    // Start polling
  });
  
  ws.on('close', () => {
    console.log('Connection lost, reconnecting...');
    setTimeout(connect, 1000); // Reconnect after 1 second
  });
  
  // Handle other events...
}

connect();
```
