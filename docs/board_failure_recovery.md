*Board Problems*

- [1. I2C Communication and Timeout Handling](#1-i2c-communication-and-timeout-handling)
  - [1.1. Problem](#11-problem)
  - [1.2. Solution](#12-solution)
  - [1.3. Diagnostics](#13-diagnostics)
- [2. Board Failure \& Recovery](#2-board-failure--recovery)
  - [2.1. Configuration](#21-configuration)
  - [2.2. Failed Board Tracking](#22-failed-board-tracking)
  - [2.3. Timeout Handling](#23-timeout-handling)
  - [2.4. Control Channel Timeouts](#24-control-channel-timeouts)
  - [2.5. Affected Operations](#25-affected-operations)
- [3. Automatic Recovery (Future)](#3-automatic-recovery-future)
  - [3.1. Background](#31-background)
  - [3.2. Success Criteria](#32-success-criteria)
  - [3.3. Configuration (Environment Variables)](#33-configuration-environment-variables)


## 1. I2C Communication and Timeout Handling

### 1.1. Problem

The TPS546D24A voltage regulator is accessed via I2C through the bitaxe-raw
protocol over USB serial. In rare cases, I2C communication can hang
indefinitely due to:

- USB connection issues
- Firmware bugs in bitaxe-raw
- Hardware faults on the I2C bus
- Board power issues

When the board monitoring thread holds a lock on the voltage controller and
encounters a hung I2C operation, it blocks indefinitely. This prevents the REST
API from accessing the same controller to read board status, causing API
requests (like `GET /api/v1/boards`) to hang and never return.

### 1.2. Solution

All I2C operations that acquire locks on shared resources (voltage controllers,
fan controllers) now use 500ms timeouts:

```rust
match tokio::time::timeout(
    Duration::from_millis(500),
    async { controller.lock().await.get_vout().await }
).await {
    Ok(Ok(value)) => { /* success */ }
    Ok(Err(e)) => { /* I2C error */ }
    Err(_) => { /* timeout - I2C hung */ }
}
```

This ensures:
- Locks are released promptly even if I2C hangs
- The REST API remains responsive (returns within ~500ms)
- Warning messages are logged when timeouts occur
- Board monitoring continues despite communication failures

### 1.3. Diagnostics

When I2C communication issues occur, you'll see warnings in the logs:

```
WARN Timeout reading VOUT (I2C may be hung)
WARN Timeout reading voltage for board serial=ABC12345 (I2C may be hung)
```

If you see these warnings persistently, check:
1. USB cable connection
2. Board power supply
3. bitaxe-raw firmware version
4. USB host controller stability

## 2. Board Failure & Recovery

### 2.1. Configuration

```bash
# Board initialization timeout in seconds (also used for reinitialize API timeout + 5s buffer)
# Read once at startup and stored in AppState.board_init_timeout
MUJINA_BOARD_INIT_TIMEOUT_SECS=10  # Default: 10
```

### 2.2. Failed Board Tracking

When board initialization fails (error, panic, or timeout), the system:
1. Stores `UsbDeviceInfo` in `Backplane.failed_board_devices` HashMap
2. Registers the failure in `AppState.failed_boards` for API visibility
3. Aborts any stuck initialization tasks to release serial port resources

This allows failed boards to be reinitialized later via the API without requiring
a physical USB reconnect.

### 2.3. Timeout Handling

Board initialization uses `tokio::select!` to race the init task against a timeout:
- On timeout: the spawned task is explicitly aborted via `task.abort()`
- After abort: waits for task completion and adds 100ms delay for OS to release serial ports
- Serial port open uses `spawn_blocking` to prevent blocking the async runtime

**Timeout Chain:**
```
Board Init Timeout (MUJINA_BOARD_INIT_TIMEOUT_SECS, default 10s)
    └── API Reinitialize Timeout = Board Init Timeout + 5s buffer
```

### 2.4. Control Channel Timeouts

All I2C and GPIO operations through the control channel have timeouts:
- Lock acquisition: 2 seconds (prevents deadlocks)
- Write operation: 1 second
- Read operation: 1 second

```rust
// channel.rs timeout structure
Lock timeout (2s) → Write timeout (1s) → Read timeout (1s)
```

### 2.5. Affected Operations

Timeout protection is applied to:

**Board monitoring thread** (runs every 30 seconds):
- Voltage regulator reads: VIN, VOUT, IOUT, power, temperature
- Status checks and fault clearing
- Fan controller operations

**REST API endpoints**:
- `GET /api/v1/boards` - board voltage reads
- `POST /api/v1/board/{serial}/voltage` - voltage set operations

## 3. Automatic Recovery (Future)
This section documents thoughts regarding possible automatic recovery for boards experiencing I2C communication failures, with configurable retry parameters via environment variables.

### 3.1. Background
Currently, when a board experiences persistent I2C failures (e.g., due to USB issues, firmware bugs, or hardware faults), it remains in a degraded state with error messages but requires manual intervention to recover. This feature adds automatic recovery capability with manual override.

### 3.2. Success Criteria
- Environment variables correctly configure retry behavior
- Failure counter increments on I2C errors, resets on success
- `needs_reinit` flag appears in API response when threshold reached
- Manual reinit endpoint works and returns appropriate response
- Auto-recovery triggers at configured interval when enabled
- Auto-recovery stops after max_retries attempts
- All logging events emit at WARN level with structured fields
- Board fully reprobes (voltage controller + fan controller + monitoring thread)
- Old monitoring thread cleanly terminates before new one starts
- Swagger UI documents the new endpoint and schema fields

### 3.3. Configuration (Environment Variables)
```bash
# Number of consecutive failures before marking board as needing recovery
MUJINA_BOARD_FAILURE_THRESHOLD=3  # Default: 3

# Number of automatic retry attempts before giving up
MUJINA_BOARD_MAX_AUTO_RETRIES=3  # Default: 3

# Duration between automatic retry attempts (seconds)
MUJINA_BOARD_RETRY_INTERVAL=30  # Default: 30

# Enable/disable automatic recovery (if false, manual only)
MUJINA_BOARD_AUTO_RECOVERY=false  # Default: false
```