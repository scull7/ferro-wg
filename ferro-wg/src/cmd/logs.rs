use chrono::{DateTime, Local};
use ferro_wg_core::client::{DaemonClientError, stream_logs};
use ferro_wg_core::ipc::{LogEntry, LogLevel};
use ferro_wg_core::logs::{ConnectionFilter, entry_passes_filter, line_matches_search};
use tokio::sync::mpsc;

/// Run the logs command.
pub async fn cmd_logs(
    level: LogLevel,
    connection: Option<String>,
    search: String,
    lines: Option<usize>,
    watch: bool,
) -> Result<(), DaemonClientError> {
    let mut receiver = stream_logs().await?;
    let search_lower = search.to_ascii_lowercase();
    let connection_filter = connection
        .as_ref()
        .map_or(ConnectionFilter::All, |_| ConnectionFilter::Active);
    let active_connection = connection.as_deref();

    if watch {
        loop {
            tokio::select! {
                entry = receiver.recv() => match entry {
                    Some(e) if entry_passes_filter(&e, level, connection_filter, active_connection)
                        && line_matches_search(&e.message, &search_lower) => {
                        println!("{}", format_entry(&e));
                    }
                    Some(_) => {}
                    None => break,
                },
                _ = tokio::signal::ctrl_c() => break,
            }
        }
    } else {
        let filtered = collect_filtered_logs(receiver, level, connection, search, lines).await;
        for e in &filtered {
            println!("{}", format_entry(e));
        }
    }
    Ok(())
}

async fn collect_filtered_logs(
    mut receiver: mpsc::Receiver<LogEntry>,
    level: LogLevel,
    connection: Option<String>,
    search: String,
    lines: Option<usize>,
) -> Vec<LogEntry> {
    let search_lower = search.to_ascii_lowercase();
    let connection_filter = connection
        .as_ref()
        .map_or(ConnectionFilter::All, |_| ConnectionFilter::Active);
    let active_connection = connection.as_deref();

    let mut buffer = Vec::new();
    let _ = tokio::time::timeout(tokio::time::Duration::from_millis(200), async {
        while let Some(entry) = receiver.recv().await {
            buffer.push(entry);
        }
    })
    .await;
    let filtered: Vec<_> = buffer
        .into_iter()
        .filter(|e| {
            entry_passes_filter(e, level, connection_filter, active_connection)
                && line_matches_search(&e.message, &search_lower)
        })
        .collect();
    if let Some(n) = lines {
        filtered.into_iter().rev().take(n).rev().collect()
    } else {
        filtered
    }
}

/// Format a log entry for plain-text output.
fn format_entry(entry: &LogEntry) -> String {
    let dt = DateTime::from_timestamp_millis(entry.timestamp_ms).unwrap_or(DateTime::UNIX_EPOCH);
    let local = dt.with_timezone(&Local);
    let time_str = local.format("%H:%M:%S").to_string();
    let level_str = format!("[{}]", entry.level.badge());
    let conn_str = entry.connection_name.as_deref().unwrap_or("(global)");
    format!(
        "[{}] {} ({}) {}",
        time_str, level_str, conn_str, entry.message
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ferro_wg_core::ipc::LogEntry;
    use tokio;

    #[test]
    fn test_format_entry() {
        let entry = LogEntry {
            timestamp_ms: 0,
            level: LogLevel::Info,
            connection_name: None,
            message: "test message".to_string(),
        };
        let formatted = format_entry(&entry);
        assert!(formatted.contains("[INFO]"));
        assert!(formatted.contains("(global)"));
        assert!(formatted.contains("test message"));
    }

    #[tokio::test]
    async fn test_collect_filtered_logs() {
        let (tx, rx) = mpsc::channel(10);
        let entries = vec![
            LogEntry {
                timestamp_ms: 0,
                level: LogLevel::Info,
                connection_name: None,
                message: "info global".to_string(),
            },
            LogEntry {
                timestamp_ms: 1,
                level: LogLevel::Warn,
                connection_name: Some("conn1".to_string()),
                message: "warn conn1".to_string(),
            },
            LogEntry {
                timestamp_ms: 2,
                level: LogLevel::Error,
                connection_name: None,
                message: "error global".to_string(),
            },
        ];
        for e in entries {
            tx.send(e).await.unwrap();
        }
        drop(tx);
        let result = collect_filtered_logs(rx, LogLevel::Warn, None, String::new(), None).await;
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].message, "warn conn1");
        assert_eq!(result[1].message, "error global");
    }
}
