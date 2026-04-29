use std::env;
use std::error::Error;
use std::fmt;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
#[cfg(unix)]
use std::os::unix::net::UnixStream;
#[cfg(unix)]
use std::path::PathBuf;
use std::process;
use std::sync::mpsc::Sender;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::{io, thread};

use cadis_protocol::{ClientId, ClientRequest, RequestEnvelope, RequestId, ServerFrame};
use cadis_store::load_config;

use crate::types::{Args, HudResult};

#[derive(Clone, Debug)]
pub(crate) enum Transport {
    #[cfg(unix)]
    Socket(PathBuf),
    Tcp(String),
}

impl fmt::Display for Transport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            #[cfg(unix)]
            Self::Socket(p) => write!(f, "{}", p.display()),
            Self::Tcp(addr) => write!(f, "tcp://{addr}"),
        }
    }
}

pub(crate) fn resolve_transport(_args: &Args) -> Transport {
    if let Ok(port) = env::var("CADIS_TCP_PORT") {
        return Transport::Tcp(format!("127.0.0.1:{port}"));
    }

    #[cfg(unix)]
    {
        let socket = _args
            .socket_path
            .clone()
            .or_else(|| env::var_os("CADIS_HUD_SOCKET").map(PathBuf::from))
            .or_else(|| load_config().ok().and_then(|c| c.effective_socket_path()));
        if let Some(path) = socket {
            return Transport::Socket(path);
        }
    }

    let config = load_config().unwrap_or_default();
    Transport::Tcp(config.effective_tcp_address())
}

pub(crate) fn send_request(
    transport: &Transport,
    request: ClientRequest,
) -> Result<Vec<ServerFrame>, Box<dyn Error>> {
    let envelope = RequestEnvelope::new(next_request_id(), ClientId::from("hud_main"), request);

    fn write_and_read(
        mut stream: impl Read + Write,
        envelope: &RequestEnvelope,
    ) -> Result<Vec<ServerFrame>, Box<dyn Error>> {
        serde_json::to_writer(&mut stream, envelope)?;
        stream.write_all(b"\n")?;
        stream.flush()?;
        let reader = BufReader::new(stream);
        let mut frames = Vec::new();
        for line in reader.lines() {
            let line = line?;
            if !line.trim().is_empty() {
                frames.push(serde_json::from_str::<ServerFrame>(&line)?);
            }
        }
        Ok(frames)
    }

    match transport {
        #[cfg(unix)]
        Transport::Socket(path) => {
            let stream = UnixStream::connect(path).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("could not connect to cadisd at {}: {e}", path.display()),
                )
            })?;
            write_and_read(stream, &envelope)
        }
        Transport::Tcp(addr) => {
            let stream = TcpStream::connect(addr).map_err(|e| {
                io::Error::new(
                    e.kind(),
                    format!("could not connect to cadisd at tcp://{addr}: {e}"),
                )
            })?;
            write_and_read(stream, &envelope)
        }
    }
}

/// Spawn a background thread that sends a request to the daemon.
/// If the connection fails, it retries every 2 seconds up to 3 times.
pub(crate) fn spawn_request(tx: Sender<HudResult>, transport: Transport, request: ClientRequest) {
    thread::spawn(move || {
        let mut attempts = 0;
        loop {
            match send_request(&transport, request.clone()) {
                Ok(frames) => {
                    let _ = tx.send(HudResult { result: Ok(frames) });
                    return;
                }
                Err(error) => {
                    attempts += 1;
                    if attempts >= 3 {
                        let _ = tx.send(HudResult {
                            result: Err(error.to_string()),
                        });
                        return;
                    }
                    thread::sleep(Duration::from_secs(2));
                }
            }
        }
    });
}

/// Spawn a background thread that opens a long-lived connection for event streaming.
/// Each frame is sent individually through the channel as it arrives.
pub(crate) fn spawn_subscription(
    tx: Sender<HudResult>,
    transport: Transport,
    request: ClientRequest,
) {
    thread::spawn(move || {
        let envelope =
            RequestEnvelope::new(next_request_id(), ClientId::from("hud_events"), request);

        fn stream_lines<S: Read + Write>(
            stream: &mut S,
            envelope: &RequestEnvelope,
            tx: &Sender<HudResult>,
        ) -> Result<(), Box<dyn std::error::Error>> {
            serde_json::to_writer(&mut *stream, envelope)?;
            (*stream).write_all(b"\n")?;
            (*stream).flush()?;
            let reader = BufReader::new(stream);
            for line in reader.lines() {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(frame) = serde_json::from_str::<ServerFrame>(&line) {
                    if tx
                        .send(HudResult {
                            result: Ok(vec![frame]),
                        })
                        .is_err()
                    {
                        return Ok(());
                    }
                }
            }
            Ok(())
        }

        let result: Result<(), Box<dyn std::error::Error>> = match &transport {
            #[cfg(unix)]
            Transport::Socket(path) => match UnixStream::connect(path) {
                Ok(mut stream) => {
                    let _ = stream.set_read_timeout(None);
                    stream_lines(&mut stream, &envelope, &tx)
                }
                Err(e) => Err(e.into()),
            },
            Transport::Tcp(addr) => match TcpStream::connect(addr) {
                Ok(mut stream) => {
                    let _ = stream.set_read_timeout(None);
                    stream_lines(&mut stream, &envelope, &tx)
                }
                Err(e) => Err(e.into()),
            },
        };

        if let Err(e) = result {
            let _ = tx.send(HudResult {
                result: Err(e.to_string()),
            });
        }
    });
}

fn next_request_id() -> RequestId {
    let millis = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or_default();
    RequestId::from(format!("req_hud_{}_{}", process::id(), millis))
}
