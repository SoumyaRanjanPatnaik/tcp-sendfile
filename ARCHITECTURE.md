# System Architecture

## Overview

Sendfile is a high-performance, cross-platform file transfer utility designed to securely transfer large files (up to 16GB+) over TCP. It prioritizes data integrity, reliability, and efficient resource utilization through parallelization and a custom binary protocol.

## 1. Transport Protocol & Wire Format

The application implements a custom application-layer protocol over TCP. It uses a lightweight framing strategy combined with `postcard`, a `#![no_std]` compatible binary serialization format, to ensure minimal overhead.

### Wire Format

Every message sent over the wire follows a strict framing format to handle stream segmentation and reassembly:

```text
Ver: 1\r\n           <-- Protocol Version Header
Len: 1024\r\n        <-- Payload Length Header
\r\n                 <-- Delimiter
[Binary Payload]     <-- Postcard-serialized data
```

- **Headers**: ASCII-based for easy debugging and version negotiation.
- **Payload**: Serialized using `postcard` + `serde`.
- **Zero-Copy Deserialization**: The implementation leverages `serde`'s borrowing capabilities. Data structures often borrow directly from the network buffer rather than allocating new memory (e.g., `&[u8]` fields in `DataV1`), significantly reducing memory churn.

### Connection Architecture

The system uses a dual-port strategy to separate control flow from data transfer:

1.  **Handshake (Port 7878)**: Used for initial metadata exchange (filename, size, BLAKE3 hash, concurrency settings).
2.  **Data Transfer (Port 7879)**: Used for high-throughput parallel data transmission.

---

## 2. Design Considerations

### Parallelization & Concurrency

To maximize bandwidth utilization, the file is virtually split into "ranges" based on the concurrency level (defaulting to available CPU cores, capped at 16).

- **Receiver**: Spawns a thread pool where each thread is responsible for a specific range of sequence numbers (blocks).
- **Sender**: Listens on the transfer port and spawns a worker thread for each incoming connection, serving block requests statelessly.
- **State Management**: Shared state (e.g., bitmap of received blocks, file handles) is managed using `Arc` (Atomic Reference Counting) and `AtomicBool`/`AtomicU64` primitives, avoiding expensive mutex locks for progress tracking.

### Chunking & Flow Control

- **Block-Based Transfer**: Files are broken into fixed-size blocks (default 1MB, max 4MB). This allows the system to transfer files larger than available RAM.
- **Request-Response Model**: The receiver actively requests specific blocks (`RequestV1`). The sender responds with the data (`DataV1`). This acts as a natural backpressure mechanism—the sender cannot overwhelm the receiver since it only sends data when requested.

### Reliability & Error Handling

- **Integrity**:
  - **File Level**: BLAKE3 hash computed (in parallel) before transfer and verified after completion.
  - **Block Level**: CRC32 checksums attached to every data packet to detect transmission errors immediately.
- **Exponential Backoff**: If a block request fails (network error or checksum mismatch), the receiver enters a retry loop with exponential backoff (e.g., 500ms -> 1s -> 2s) up to a maximum limit (5 retries) before failing the connection.
- **Structured Errors**: The `thiserror` crate is used to define typed, context-rich error variants (`SendFileError`, `TransportError`, etc), allowing precise handling of I/O, serialization, and protocol errors.
- **Failed Chunks Handling**: If a chunk verification fails or a timeout occurs, the receiver explicitly re-requests the same chunk sequence number.

---

## 3. Resource Consumption & Performance

### Memory Management

- **Stream-Based I/O**: The system never loads the entire file into memory. It reads/writes exactly one block size (plus overhead) per connection.
- **Bounded Buffers**: Transport buffers are statically sized (`MAX_MESSAGE_SIZE`), preventing Out-Of-Memory (OOM) attacks or crashes with large payloads.
- **Allocation Efficiency**: Heavy data vectors are allocated once and reused. `Arc` is used to share read-only configuration and file paths across threads, ensuring almost zero cloning of heavy data.

### CPU Utilization

- **Parallel Hashing**: BLAKE3 hashing is parallelized using Rayon-like logic (manual threading in this case) to prevent hashing from becoming a bottleneck on multi-gigabyte files.
- **Smart Compression**: The sender probes the first block with Gzip. If compression does not yield space savings (e.g., random data or already compressed files), it disables compression for the remainder of the session to save CPU cycles.

---

## 4. C4 Architecture Diagrams
![C4 Diagram](C4-diag.svg)
