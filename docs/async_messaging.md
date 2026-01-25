*Asynchronous Messaging in Mujina*

- [1. Tokio Channel Types Comparison](#1-tokio-channel-types-comparison)
- [2. Backplane Command Queue](#2-backplane-command-queue)
  - [2.1. Purpose](#21-purpose)
  - [2.2. Command Queue Pattern](#22-command-queue-pattern)
  - [2.3. Architecture](#23-architecture)
  - [2.4. Command Types](#24-command-types)
  - [2.5. Request-Response Flow](#25-request-response-flow)
  - [2.6. Implementation Details](#26-implementation-details)
  - [2.7. Error Handling](#27-error-handling)
  - [2.8. Extensibility](#28-extensibility)


## 1. Tokio Channel Types Comparison

The codebase uses three Tokio channel types, each suited to different communication patterns:

| Aspect | `mpsc` | `oneshot` | `watch` |
|--------|--------|-----------|---------|
| **Producers** | Multiple | Single | Single |
| **Consumers** | Single | Single | Multiple |
| **Messages** | Many, queued | Exactly one | Latest value only |
| **Consumption** | Each message consumed once | One-shot delivery | All receivers see same value |
| **Backpressure** | Bounded queue can block sender | N/A (single message) | Never blocks (overwrites) |
| **Use Case** | Command streams, event queues | Request-response correlation | State broadcast, shutdown signals |

**In this codebase:**

| Channel | Usage | Example |
|---------|-------|---------|
| `mpsc` | API → Backplane command queue | `BackplaneCommand::ReinitializeBoard` |
| `oneshot` | Embedded in command for response | `response_tx: oneshot::Sender<ReinitializeResult>` |
| `watch` | Board → HashThread shutdown signal | `ThreadRemovalSignal::Shutdown` |

## 2. Backplane Command Queue

The backplane command queue provides a decoupled interface for external systems to
control board lifecycle operations without direct access to internal data structures.

### 2.1. Purpose

The command queue solves several architectural challenges:

1. **Decoupling**: External interfaces (REST API, MQTT, CLI) should not have direct
   access to the backplane's internal state or board instances
2. **Thread Safety**: Board operations involve async I/O, mutex locks, and hardware
   communication that must be serialized
3. **Request-Response Pattern**: Callers need to know whether operations succeeded
   and receive relevant results (e.g., new voltage after reinitialization)
4. **Extensibility**: New board operations can be added without modifying the API layer


### 2.2. Command Queue Pattern

The backplane command queue combines `mpsc` + `oneshot` to create a request-response pattern:

**Raw mpsc example** (fire-and-forget):
```rust
// Sender side
tx.send("reinitialize:ABC123".to_string()).await?;
// No way to get a response!

// Receiver side
while let Some(msg) = rx.recv().await {
    // Must parse string, no type safety
    if msg.starts_with("reinitialize:") { ... }
}
```

**Command queue pattern** (request-response):
```rust
// Sender side
let (response_tx, response_rx) = oneshot::channel();
tx.send(BackplaneCommand::ReinitializeBoard {
    serial: "ABC123".to_string(),
    response_tx,  // Embed reply channel in command
}).await?;
let result = response_rx.await?;  // Wait for typed response

// Receiver side
while let Some(cmd) = rx.recv().await {
    match cmd {
        BackplaneCommand::ReinitializeBoard { serial, response_tx } => {
            let result = self.reinitialize(&serial).await;
            let _ = response_tx.send(result);  // Send typed response
        }
    }
}
```

**Why not just use `mpsc` for responses too?**

Using a second `mpsc` channel for responses would require correlation IDs to match
responses to requests, adding complexity:

```rust
// Anti-pattern: mpsc for both directions
struct Request { id: u64, command: String }
struct Response { id: u64, result: String }

// Sender must track pending requests by ID
// Receiver must include ID in response
// Race conditions if responses arrive out of order
```

The `oneshot` channel eliminates this complexity: each request creates its own
dedicated response channel, guaranteeing 1:1 request-response correlation with
no ID management needed.

### 2.3. Architecture

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   REST API      │     │   Backplane     │     │   Board         │
│   (v1.rs)       │     │   (backplane.rs)│     │   (bitaxe.rs)   │
├─────────────────┤     ├─────────────────┤     ├─────────────────┤
│                 │     │                 │     │                 │
│ reinitialize()  │────▶│ cmd_rx.recv()   │────▶│ shutdown()      │
│                 │     │                 │     │ initialize()    │
│ ◀───────────────│     │                 │     │                 │
│ oneshot response│◀────│ response_tx     │◀────│ Result          │
│                 │     │                 │     │                 │
└─────────────────┘     └─────────────────┘     └─────────────────┘
        │                       │
        │  mpsc (commands)      │  oneshot (response)
        └───────────────────────┘
```

### 2.4. Command Types

Commands are defined in `backplane_cmd.rs`:

```rust
pub enum BackplaneCommand {
    /// Request to reinitialize a specific board by serial number.
    ReinitializeBoard {
        serial: String,
        response_tx: oneshot::Sender<ReinitializeResult>,
    },
}
```

Each command variant includes:
- **Parameters**: Data needed to execute the command (e.g., board serial number)
- **Response Channel**: A `oneshot::Sender` for returning results to the caller

### 2.5. Request-Response Flow

```
1. API receives HTTP request
2. API creates oneshot channel (tx, rx)
3. API sends BackplaneCommand with tx via mpsc channel
4. API awaits rx with timeout
5. Backplane receives command from cmd_rx
6. Backplane executes operation (may involve hardware I/O)
7. Backplane sends ReinitializeResult via response_tx
8. API receives result and returns HTTP response
```

### 2.6. Implementation Details

**Channel Setup** (in `daemon.rs`):
```rust
let (backplane_cmd_tx, backplane_cmd_rx) = mpsc::channel(16);

// Pass sender to API state
api_state.backplane_cmd_tx = Some(backplane_cmd_tx);

// Pass receiver to backplane
let mut backplane = Backplane::new(event_rx, backplane_cmd_rx, scheduler_tx, api_state);
```

**Command Processing** (in `backplane.rs`):
```rust
pub async fn run(&mut self) -> Result<()> {
    loop {
        tokio::select! {
            Some(event) = self.event_rx.recv() => {
                self.handle_usb_event(event).await?;
            }
            Some(cmd) = self.cmd_rx.recv() => {
                self.handle_command(cmd).await;
            }
            else => break,
        }
    }
    Ok(())
}

async fn handle_command(&mut self, cmd: BackplaneCommand) {
    match cmd {
        BackplaneCommand::ReinitializeBoard { serial, response_tx } => {
            let result = self.reinitialize_board(&serial).await;
            let _ = response_tx.send(result);  // Ignore if receiver dropped
        }
    }
}
```

**Result Structure**:
```rust
pub struct ReinitializeResult {
    pub success: bool,
    pub message: String,
    pub error: Option<String>,
    pub current_voltage: Option<f32>,
}
```

### 2.7. Error Handling

The command queue handles several error scenarios:

| Scenario | Handling |
|----------|----------|
| Channel full | mpsc channel has capacity 16; send blocks if full |
| Receiver dropped | `response_tx.send()` returns Err, which is ignored |
| Command timeout | API applies its own timeout on the oneshot receiver |
| Board not found | Returns `ReinitializeResult::failure()` |
| Hardware error | Returns failure result with error details |

**Timeout Chain**:
```
API Timeout = board_init_timeout + 5 seconds buffer

Example with default 10s init timeout:
- Board initialization: up to 10 seconds
- API response timeout: 15 seconds total
```

### 2.8. Extensibility

To add a new board operation:

1. **Add command variant** to `BackplaneCommand` enum:
   ```rust
   pub enum BackplaneCommand {
       ReinitializeBoard { ... },
       SetBoardVoltage {
           serial: String,
           voltage: f32,
           response_tx: oneshot::Sender<SetVoltageResult>,
       },
   }
   ```

2. **Define result type** with appropriate fields:
   ```rust
   pub struct SetVoltageResult {
       pub success: bool,
       pub actual_voltage: Option<f32>,
       pub error: Option<String>,
   }
   ```

3. **Handle in backplane**:
   ```rust
   async fn handle_command(&mut self, cmd: BackplaneCommand) {
       match cmd {
           BackplaneCommand::ReinitializeBoard { ... } => { ... }
           BackplaneCommand::SetBoardVoltage { serial, voltage, response_tx } => {
               let result = self.set_board_voltage(&serial, voltage).await;
               let _ = response_tx.send(result);
           }
       }
   }
   ```

4. **Add API endpoint** that sends the command and awaits response.

This pattern ensures all board operations flow through the backplane's event loop,
maintaining consistent state management and serialized hardware access.
