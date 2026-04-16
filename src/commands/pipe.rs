use anyhow::Result;
use rusqlite::Connection;
use serde::Deserialize;
use std::io::{BufRead, Write};

use crate::commands::call::dispatch_tool;
use crate::mcp::server::GigaBrainServer;

/// Maximum JSONL line size accepted on stdin (5 MB).
const MAX_LINE_BYTES: usize = 5_242_880;

#[derive(Deserialize)]
struct PipeCommand {
    tool: String,
    input: serde_json::Value,
}

enum LineRead {
    Line(String),
    TooLong,
}

fn read_limited_line<R: BufRead>(
    reader: &mut R,
    max_bytes: usize,
) -> std::io::Result<Option<LineRead>> {
    let mut buf = Vec::new();
    let mut exceeded = false;
    enum ReturnKind {
        Line,
        TooLong,
    }

    loop {
        let mut consume_len = 0;
        let mut result = None;
        {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                if buf.is_empty() && !exceeded {
                    return Ok(None);
                }
                result = Some(if exceeded {
                    ReturnKind::TooLong
                } else {
                    ReturnKind::Line
                });
            } else if let Some(pos) = available.iter().position(|b| *b == b'\n') {
                let slice = &available[..pos];
                if !exceeded {
                    if buf.len() + slice.len() > max_bytes {
                        exceeded = true;
                        buf.clear();
                    } else {
                        buf.extend_from_slice(slice);
                    }
                }
                consume_len = pos + 1;
                result = Some(if exceeded {
                    ReturnKind::TooLong
                } else {
                    ReturnKind::Line
                });
            } else {
                if !exceeded {
                    if buf.len() + available.len() > max_bytes {
                        exceeded = true;
                        buf.clear();
                    } else {
                        buf.extend_from_slice(available);
                    }
                }
                consume_len = available.len();
            }
        }

        if consume_len > 0 {
            reader.consume(consume_len);
        }

        if let Some(result) = result {
            return Ok(Some(match result {
                ReturnKind::TooLong => LineRead::TooLong,
                ReturnKind::Line => {
                    let line = String::from_utf8(std::mem::take(&mut buf))
                        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
                    LineRead::Line(line)
                }
            }));
        }
    }
}

fn write_json_line<W: Write>(writer: &mut W, value: &serde_json::Value) -> std::io::Result<()> {
    let payload = serde_json::to_string(value)
        .unwrap_or_else(|_| "{\"error\":\"serialization failed\"}".to_string());
    writeln!(writer, "{payload}")
}

fn run_with_io<R: BufRead, W: Write>(
    server: &GigaBrainServer,
    mut reader: R,
    mut writer: W,
    max_line_bytes: usize,
) -> Result<()> {
    loop {
        let line = match read_limited_line(&mut reader, max_line_bytes) {
            Ok(Some(LineRead::Line(line))) => line,
            Ok(Some(LineRead::TooLong)) => {
                let err = serde_json::json!({
                    "error": format!(
                        "line exceeds maximum size of {max_line_bytes} bytes; rejected"
                    )
                });
                write_json_line(&mut writer, &err)?;
                continue;
            }
            Ok(None) => break,
            Err(e) => {
                let err = serde_json::json!({"error": format!("stdin read error: {e}")});
                write_json_line(&mut writer, &err)?;
                continue;
            }
        };

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let cmd: PipeCommand = match serde_json::from_str(trimmed) {
            Ok(c) => c,
            Err(e) => {
                let err = serde_json::json!({"error": format!("parse error: {e}")});
                write_json_line(&mut writer, &err)?;
                continue;
            }
        };

        match dispatch_tool(server, &cmd.tool, cmd.input) {
            Ok(result) => {
                // Output as single-line JSON (JSONL)
                write_json_line(&mut writer, &result)?;
            }
            Err(e) => {
                let err = serde_json::json!({"error": e});
                write_json_line(&mut writer, &err)?;
            }
        }
    }

    Ok(())
}

pub fn run(db: Connection) -> Result<()> {
    let server = GigaBrainServer::new(db);
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    run_with_io(&server, stdin.lock(), stdout.lock(), MAX_LINE_BYTES)
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use super::*;
    use crate::core::db;
    use serde_json::Value;
    use std::io::Cursor;

    #[test]
    fn pipe_rejects_oversized_line_and_continues() {
        let conn = db::open(":memory:").unwrap();
        let server = GigaBrainServer::new(conn);

        let oversized_payload = "x".repeat(80);
        let oversized_line =
            format!("{{\"tool\":\"brain_stats\",\"input\":{{\"pad\":\"{oversized_payload}\"}}}}");
        let valid_line = "{\"tool\":\"brain_stats\",\"input\":{}}";
        let input = format!("{oversized_line}\n{valid_line}\n");

        let reader = Cursor::new(input);
        let mut output = Vec::new();
        run_with_io(&server, reader, &mut output, 64).unwrap();

        let output = String::from_utf8(output).unwrap();
        let mut lines = output.lines();

        let first: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert!(first.get("error").is_some());

        let second: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert!(second.get("page_count").is_some());
    }
}
