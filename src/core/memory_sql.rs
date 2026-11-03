//! Notchy Memory in SQL Server.
//!
//! When a connection string is set in the Memory tab, every memory entry is also written to
//! the `NotchyMemory` table (created on first connect), and the newest rows are loaded back so
//! search covers everything in the database, not just this PC's local file. The local file
//! keeps working as a buffer while the server is unreachable; entries written then are sent
//! once it is back.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use parking_lot::Mutex;
use tiberius::{Client, Config};
use tokio::net::TcpStream;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

use crate::core::memory::{Entry, Kind};

const CREATE: &str = "IF OBJECT_ID(N'dbo.NotchyMemory', N'U') IS NULL
BEGIN
    CREATE TABLE dbo.NotchyMemory (
        Id BIGINT IDENTITY(1,1) PRIMARY KEY,
        At BIGINT NOT NULL,
        Kind NVARCHAR(20) NOT NULL,
        Title NVARCHAR(200) NOT NULL,
        Detail NVARCHAR(600) NOT NULL,
        App NVARCHAR(60) NOT NULL,
        Device NVARCHAR(60) NOT NULL
    );
    CREATE INDEX IX_NotchyMemory_At ON dbo.NotchyMemory (At DESC);
END";

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Status {
    Off,
    Connecting,
    Connected { rows: u64 },
    Failed(String),
}

struct Shared {
    status: Status,
    sender: Option<UnboundedSender<Command>>,
}

enum Command {
    Insert(Entry),
    Reconnect,
}

static SHARED: Mutex<Shared> = Mutex::new(Shared {
    status: Status::Off,
    sender: None,
});
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn status() -> Status {
    SHARED.lock().status.clone()
}

fn set_status(status: Status) {
    SHARED.lock().status = status;
    crate::platform::wake();
}

/// Queues an entry for the database (no-op without a connection string).
pub fn push(entry: &Entry) {
    ensure_started();
    if let Some(sender) = SHARED.lock().sender.as_ref() {
        let _ = sender.send(Command::Insert(entry.clone()));
    }
}

/// The connection string changed: drop the connection and start again.
pub fn reconnect() {
    ensure_started();
    if let Some(sender) = SHARED.lock().sender.as_ref() {
        let _ = sender.send(Command::Reconnect);
    }
}

fn ensure_started() {
    if SHARED.lock().sender.is_some() {
        return;
    }
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        return;
    };
    let (sender, receiver) = unbounded_channel();
    SHARED.lock().sender = Some(sender);
    handle.spawn(worker(receiver));
}

type Sql = Client<Compat<TcpStream>>;

async fn connect(connection: &str) -> Result<Sql, String> {
    let config = Config::from_ado_string(connection).map_err(|error| format!("Bad connection string: {error}"))?;
    let tcp = {
        use tiberius::SqlBrowser;
        TcpStream::connect_named(&config)
            .await
            .map_err(|error| format!("Cannot reach the server: {error}"))?
    };
    let _ = tcp.set_nodelay(true);
    let mut client = Client::connect(config, tcp.compat_write())
        .await
        .map_err(|error| format!("Sign-in failed: {error}"))?;
    client
        .simple_query(CREATE)
        .await
        .map_err(|error| format!("Cannot create the table: {error}"))?
        .into_results()
        .await
        .map_err(|error| error.to_string())?;
    Ok(client)
}

fn device() -> String {
    std::env::var("COMPUTERNAME").unwrap_or_else(|_| "PC".to_string())
}

async fn insert(client: &mut Sql, entry: &Entry) -> Result<(), String> {
    let kind = serde_json::to_value(entry.kind)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_default();
    client
        .execute(
            "INSERT INTO dbo.NotchyMemory (At, Kind, Title, Detail, App, Device) VALUES (@P1, @P2, @P3, @P4, @P5, @P6)",
            &[&(entry.at as i64), &kind, &entry.title, &entry.detail, &entry.app, &device()],
        )
        .await
        .map(|_| ())
        .map_err(|error| error.to_string())
}

/// The newest rows, oldest first.
async fn load(client: &mut Sql) -> Result<Vec<Entry>, String> {
    let rows = client
        .query(
            "SELECT TOP 4000 At, Kind, Title, Detail, App FROM dbo.NotchyMemory ORDER BY At DESC",
            &[],
        )
        .await
        .map_err(|error| error.to_string())?
        .into_first_result()
        .await
        .map_err(|error| error.to_string())?;
    let mut entries: Vec<Entry> = rows
        .iter()
        .filter_map(|row| {
            let kind: Kind = serde_json::from_value(serde_json::Value::String(
                row.get::<&str, _>(1)?.to_string(),
            ))
            .ok()?;
            Some(Entry {
                at: row.get::<i64, _>(0)? as u64,
                kind,
                title: row.get::<&str, _>(2).unwrap_or_default().to_string(),
                detail: row.get::<&str, _>(3).unwrap_or_default().to_string(),
                app: row.get::<&str, _>(4).unwrap_or_default().to_string(),
            })
        })
        .collect();
    entries.reverse();
    Ok(entries)
}

async fn count(client: &mut Sql) -> u64 {
    let Ok(stream) = client.query("SELECT COUNT_BIG(*) FROM dbo.NotchyMemory", &[]).await else {
        return 0;
    };
    stream
        .into_row()
        .await
        .ok()
        .flatten()
        .and_then(|row| row.get::<i64, _>(0))
        .map_or(0, |value| value as u64)
}

async fn worker(mut receiver: UnboundedReceiver<Command>) {
    let mut client: Option<Sql> = None;
    let mut pending: Vec<Entry> = Vec::new();
    let mut rows = 0u64;
    loop {
        let connection = crate::core::brain::settings().sql_connection;
        if connection.is_empty() {
            client = None;
            set_status(Status::Off);
        } else if client.is_none() {
            set_status(Status::Connecting);
            match connect(&connection).await {
                Ok(mut connected) => {
                    if let Ok(entries) = load(&mut connected).await {
                        crate::core::memory::merge(entries);
                    }
                    rows = count(&mut connected).await;
                    client = Some(connected);
                    set_status(Status::Connected { rows });
                }
                Err(error) => {
                    log::warn!("Notchy Memory SQL: {error}");
                    set_status(Status::Failed(error));
                }
            }
        }
        // Send anything written while the server was away.
        if let Some(connected) = client.as_mut() {
            while let Some(entry) = pending.first() {
                match insert(connected, entry).await {
                    Ok(()) => {
                        pending.remove(0);
                        rows += 1;
                    }
                    Err(error) => {
                        set_status(Status::Failed(error));
                        client = None;
                        break;
                    }
                }
            }
            if client.is_some() {
                set_status(Status::Connected { rows });
            }
        }
        let wait = if client.is_none() && !connection.is_empty() {
            Duration::from_secs(30)
        } else {
            Duration::from_secs(3600)
        };
        match tokio::time::timeout(wait, receiver.recv()).await {
            Ok(Some(Command::Insert(entry))) => {
                if !connection.is_empty() {
                    pending.push(entry);
                    if pending.len() > 5000 {
                        pending.remove(0);
                    }
                }
            }
            Ok(Some(Command::Reconnect)) => {
                GENERATION.fetch_add(1, Ordering::SeqCst);
                client = None;
            }
            Ok(None) => return,
            Err(_) => {}
        }
    }
}
