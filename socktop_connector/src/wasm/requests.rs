//! WebSocket request handlers for WASM environments.

use crate::error::{ConnectorError, Result};
use crate::pb::Processes;
use crate::utils::{gunzip_to_string, gunzip_to_vec, is_gzip, log_debug};
use crate::{
    AgentRequest, AgentResponse, DiskInfo, JournalResponse, Metrics, ProcessInfo,
    ProcessMetricsResponse, ProcessesPayload,
};

use prost::Message as ProstMessage;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;
use web_sys::WebSocket;

/// Send a request and wait for response with binary data handling
pub async fn send_request_and_wait(
    websocket: &WebSocket,
    request: AgentRequest,
) -> Result<AgentResponse> {
    // Use the legacy string format that the agent expects
    let request_string = request.to_legacy_string();

    // Send request
    websocket
        .send_with_str(&request_string)
        .map_err(|e| ConnectorError::protocol_error(format!("Failed to send message: {e:?}")))?;

    // Wait for response using JavaScript Promise
    let (response, binary_data) = wait_for_response_with_binary(websocket).await?;

    // Parse the response based on the request type
    match request {
        AgentRequest::Metrics => {
            // Check if this is binary data (protobuf from agent)
            if response.starts_with("BINARY_DATA:") {
                // Extract the byte count
                let byte_count: usize = response
                    .strip_prefix("BINARY_DATA:")
                    .unwrap_or("0")
                    .parse()
                    .unwrap_or(0);

                // For now, return a placeholder metrics response indicating binary data received
                // TODO: Implement proper protobuf decoding for binary data
                let placeholder_metrics = Metrics {
                    sampled_at_ms: None,
                    cpu_total: 0.0,
                    cpu_per_core: vec![0.0],
                    mem_total: 0,
                    mem_used: 0,
                    swap_total: 0,
                    swap_used: 0,
                    hostname: format!("Binary protobuf data ({byte_count} bytes)"),
                    cpu_temp_c: None,
                    disks: vec![],
                    networks: vec![],
                    top_processes: vec![],
                    gpus: None,
                    process_count: None,
                };
                Ok(AgentResponse::Metrics(placeholder_metrics))
            } else {
                // Try to parse as JSON (fallback)
                let metrics: Metrics = serde_json::from_str(&response).map_err(|e| {
                    ConnectorError::serialization_error(format!("Failed to parse metrics: {e}"))
                })?;
                Ok(AgentResponse::Metrics(metrics))
            }
        }
        AgentRequest::Disks => {
            let disks: Vec<DiskInfo> = serde_json::from_str(&response).map_err(|e| {
                ConnectorError::serialization_error(format!("Failed to parse disks: {e}"))
            })?;
            Ok(AgentResponse::Disks(disks))
        }
        AgentRequest::Processes => {
            log_debug(&format!(
                "🔍 Processing process request - response: {}",
                if response.len() > 100 {
                    format!("{}...", &response[..100])
                } else {
                    response.clone()
                }
            ));
            log_debug(&format!(
                "🔍 Binary data available: {}",
                binary_data.is_some()
            ));
            if let Some(ref data) = binary_data {
                log_debug(&format!("🔍 Binary data size: {} bytes", data.len()));
                // Check if it's gzipped data and decompress it first
                if is_gzip(data) {
                    log_debug("🔍 Process data is gzipped, decompressing...");
                    match gunzip_to_vec(data) {
                        Ok(decompressed_bytes) => {
                            log_debug(&format!(
                                "🔍 Successfully decompressed {} bytes, now decoding protobuf...",
                                decompressed_bytes.len()
                            ));
                            // Now decode the decompressed bytes as protobuf
                            match <Processes as ProstMessage>::decode(decompressed_bytes.as_slice())
                            {
                                Ok(protobuf_processes) => {
                                    log_debug(&format!(
                                        "✅ Successfully decoded {} processes from gzipped protobuf",
                                        protobuf_processes.rows.len()
                                    ));

                                    // Convert protobuf processes to ProcessInfo structs
                                    let processes: Vec<ProcessInfo> = protobuf_processes
                                        .rows
                                        .into_iter()
                                        .map(|p| ProcessInfo {
                                            pid: p.pid,
                                            name: p.name,
                                            cpu_usage: p.cpu_usage,
                                            mem_bytes: p.mem_bytes,
                                        })
                                        .collect();

                                    let processes_payload = ProcessesPayload {
                                        top_processes: processes,
                                        process_count: protobuf_processes.process_count as usize,
                                    };
                                    return Ok(AgentResponse::Processes(processes_payload));
                                }
                                Err(e) => {
                                    log_debug(&format!(
                                        "❌ Failed to decode decompressed protobuf: {e}"
                                    ));
                                }
                            }
                        }
                        Err(e) => {
                            log_debug(&format!(
                                "❌ Failed to decompress gzipped process data: {e}"
                            ));
                        }
                    }
                }
            }

            // Check if this is binary data (protobuf from agent)
            if response.starts_with("BINARY_DATA:") {
                // Extract the binary data size and decode protobuf
                let byte_count_str = response.strip_prefix("BINARY_DATA:").unwrap_or("0");
                let _byte_count: usize = byte_count_str.parse().unwrap_or(0);

                // Check if we have the actual binary data
                if let Some(binary_bytes) = binary_data {
                    log_debug(&format!(
                        "🔧 Decoding {} bytes of protobuf process data",
                        binary_bytes.len()
                    ));

                    // Try to decode the protobuf data using the prost Message trait
                    match <Processes as ProstMessage>::decode(&binary_bytes[..]) {
                        Ok(protobuf_processes) => {
                            log_debug(&format!(
                                "✅ Successfully decoded {} processes from protobuf",
                                protobuf_processes.rows.len()
                            ));

                            // Convert protobuf processes to ProcessInfo structs
                            let processes: Vec<ProcessInfo> = protobuf_processes
                                .rows
                                .into_iter()
                                .map(|p| ProcessInfo {
                                    pid: p.pid,
                                    name: p.name,
                                    cpu_usage: p.cpu_usage,
                                    mem_bytes: p.mem_bytes,
                                })
                                .collect();

                            let processes_payload = ProcessesPayload {
                                top_processes: processes,
                                process_count: protobuf_processes.process_count as usize,
                            };
                            Ok(AgentResponse::Processes(processes_payload))
                        }
                        Err(e) => {
                            log_debug(&format!("❌ Failed to decode protobuf: {e}"));
                            // Fallback to empty processes
                            let processes = ProcessesPayload {
                                top_processes: vec![],
                                process_count: 0,
                            };
                            Ok(AgentResponse::Processes(processes))
                        }
                    }
                } else {
                    log_debug(
                        "❌ Binary data indicator received but no actual binary data preserved",
                    );
                    let processes = ProcessesPayload {
                        top_processes: vec![],
                        process_count: 0,
                    };
                    Ok(AgentResponse::Processes(processes))
                }
            } else {
                // Try to parse as JSON (fallback)
                let processes: ProcessesPayload = serde_json::from_str(&response).map_err(|e| {
                    ConnectorError::serialization_error(format!("Failed to parse processes: {e}"))
                })?;
                Ok(AgentResponse::Processes(processes))
            }
        }
        AgentRequest::ProcessMetrics { pid: _ } => {
            // Parse JSON response for process metrics
            let process_metrics: ProcessMetricsResponse =
                serde_json::from_str(&response).map_err(|e| {
                    ConnectorError::serialization_error(format!(
                        "Failed to parse process metrics: {e}"
                    ))
                })?;
            Ok(AgentResponse::ProcessMetrics(process_metrics))
        }
        AgentRequest::JournalEntries { pid: _ } => {
            // Parse JSON response for journal entries
            let journal_entries: JournalResponse =
                serde_json::from_str(&response).map_err(|e| {
                    ConnectorError::serialization_error(format!(
                        "Failed to parse journal entries: {e}"
                    ))
                })?;
            Ok(AgentResponse::JournalEntries(journal_entries))
        }
    }
}

async fn wait_for_response_with_binary(websocket: &WebSocket) -> Result<(String, Option<Vec<u8>>)> {
    let start_time = js_sys::Date::now();
    let timeout_ms = 10000.0; // 10 second timeout

    // Store the response in a shared location
    let response_cell = Rc::new(RefCell::new(None::<String>));
    let binary_data_cell = Rc::new(RefCell::new(None::<Vec<u8>>));
    let error_cell = Rc::new(RefCell::new(None::<String>));

    // Use a unique request ID to avoid message collision
    let _request_id = js_sys::Math::random();
    let response_received = Rc::new(RefCell::new(false));

    // Set up the message handler that only processes if we haven't gotten a response yet
    {
        let response_cell = response_cell.clone();
        let binary_data_cell = binary_data_cell.clone();
        let response_received = response_received.clone();
        let onmessage_callback = Closure::wrap(Box::new(move |e: web_sys::MessageEvent| {
            // Only process if we haven't already received a response for this request
            if !*response_received.borrow() {
                // Handle text messages (JSON responses for metrics/disks)
                if let Ok(data) = e.data().dyn_into::<js_sys::JsString>() {
                    let message = data.as_string().unwrap_or_default();
                    if !message.is_empty() {
                        // Debug: Log what we received (truncated)
                        let preview = if message.len() > 100 {
                            format!("{}...", &message[..100])
                        } else {
                            message.clone()
                        };
                        log_debug(&format!("🔍 Received text: {preview}"));

                        *response_cell.borrow_mut() = Some(message);
                        *response_received.borrow_mut() = true;
                    }
                }
                // Handle binary messages (could be JSON as text bytes or actual protobuf)
                else if let Ok(array_buffer) = e.data().dyn_into::<js_sys::ArrayBuffer>() {
                    let uint8_array = js_sys::Uint8Array::new(&array_buffer);
                    let length = uint8_array.length() as usize;
                    let mut bytes = vec![0u8; length];
                    uint8_array.copy_to(&mut bytes);

                    log_debug(&format!("🔍 Received binary data: {length} bytes"));

                    // Debug: Log the first few bytes to see what we're dealing with
                    let first_bytes = if bytes.len() >= 4 {
                        format!(
                            "0x{:02x} 0x{:02x} 0x{:02x} 0x{:02x}",
                            bytes[0], bytes[1], bytes[2], bytes[3]
                        )
                    } else {
                        format!("Only {} bytes available", bytes.len())
                    };
                    log_debug(&format!("🔍 First bytes: {first_bytes}"));

                    // Try to decode as UTF-8 text first (in case it's JSON sent as binary)
                    match String::from_utf8(bytes.clone()) {
                        Ok(text) => {
                            // If it decodes to valid UTF-8, check if it looks like JSON
                            let trimmed = text.trim();
                            if (trimmed.starts_with('{') && trimmed.ends_with('}'))
                                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
                            {
                                log_debug(&format!(
                                    "🔍 Binary data is actually JSON text: {}",
                                    if text.len() > 100 {
                                        format!("{}...", &text[..100])
                                    } else {
                                        text.clone()
                                    }
                                ));
                                *response_cell.borrow_mut() = Some(text);
                                *response_received.borrow_mut() = true;
                            } else {
                                log_debug(&format!(
                                    "🔍 Binary data is UTF-8 text but not JSON: {}",
                                    if text.len() > 100 {
                                        format!("{}...", &text[..100])
                                    } else {
                                        text.clone()
                                    }
                                ));
                                *response_cell.borrow_mut() = Some(text);
                                *response_received.borrow_mut() = true;
                            }
                        }
                        Err(_) => {
                            // If it's not valid UTF-8, check if it's gzipped data
                            if is_gzip(&bytes) {
                                log_debug(&format!(
                                    "🔍 Binary data appears to be gzipped ({length} bytes)"
                                ));
                                // Try to decompress using unified gzip decompression
                                match gunzip_to_string(&bytes) {
                                    Ok(decompressed_text) => {
                                        log_debug(&format!(
                                            "🔍 Gzipped data decompressed to text: {}",
                                            if decompressed_text.len() > 100 {
                                                format!("{}...", &decompressed_text[..100])
                                            } else {
                                                decompressed_text.clone()
                                            }
                                        ));
                                        *response_cell.borrow_mut() = Some(decompressed_text);
                                        *response_received.borrow_mut() = true;
                                    }
                                    Err(e) => {
                                        log_debug(&format!("🔍 Failed to decompress gzip: {e}"));
                                        // Fallback: treat as actual binary protobuf data
                                        *binary_data_cell.borrow_mut() = Some(bytes.clone());
                                        *response_cell.borrow_mut() =
                                            Some(format!("BINARY_DATA:{length}"));
                                        *response_received.borrow_mut() = true;
                                    }
                                }
                            } else {
                                // If it's not valid UTF-8 and not gzipped, it's likely actual binary protobuf data
                                log_debug(&format!(
                                    "🔍 Binary data is actual protobuf ({length} bytes)"
                                ));
                                *binary_data_cell.borrow_mut() = Some(bytes);
                                *response_cell.borrow_mut() = Some(format!("BINARY_DATA:{length}"));
                                *response_received.borrow_mut() = true;
                            }
                        }
                    }
                } else {
                    // Log what type of data we got
                    log_debug(&format!("🔍 Received unknown data type: {:?}", e.data()));
                }
            }
        }) as Box<dyn FnMut(_)>);
        websocket.set_onmessage(Some(onmessage_callback.as_ref().unchecked_ref()));
        onmessage_callback.forget();
    }

    // Set up the error handler
    {
        let error_cell = error_cell.clone();
        let response_received = response_received.clone();
        let onerror_callback = Closure::wrap(Box::new(move |_e: web_sys::ErrorEvent| {
            if !*response_received.borrow() {
                *error_cell.borrow_mut() = Some("WebSocket error occurred".to_string());
                *response_received.borrow_mut() = true;
            }
        }) as Box<dyn FnMut(_)>);
        websocket.set_onerror(Some(onerror_callback.as_ref().unchecked_ref()));
        onerror_callback.forget();
    }

    // Poll for response with proper async delays
    loop {
        // Check for response
        if *response_received.borrow() {
            if let Some(response) = response_cell.borrow().as_ref() {
                let binary_data = binary_data_cell.borrow().clone();
                return Ok((response.clone(), binary_data));
            }
            if let Some(error) = error_cell.borrow().as_ref() {
                return Err(ConnectorError::protocol_error(error));
            }
        }

        // Check timeout
        let now = js_sys::Date::now();
        if now - start_time > timeout_ms {
            *response_received.borrow_mut() = true; // Mark as done to prevent future processing
            return Err(ConnectorError::protocol_error("WebSocket response timeout"));
        }

        // Wait 50ms before checking again
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            let closure = Closure::once(move || resolve.call0(&JsValue::UNDEFINED));
            web_sys::window()
                .unwrap()
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    closure.as_ref().unchecked_ref(),
                    50,
                )
                .unwrap();
            closure.forget();
        });
        let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
    }
}
