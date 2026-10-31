//! A request gets its own connection and reply; concurrent commands cannot
//! overwrite one another. Only the current user can access the control socket.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::net::UnixStream,
    path::Path,
    time::Duration,
};

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Request {
    Ping,
    Pause,
    Resume,
    Refresh,
    Restore,
    Stop,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Reply {
    pub success: bool,
    pub message: String,
}

pub fn send(path: &Path, request: Request) -> Result<String> {
    let mut stream = UnixStream::connect(path).context(
        "could not connect to ProcScape; restart procscape.service to load the updated daemon",
    )?;
    stream.set_read_timeout(Some(Duration::from_secs(30)))?;
    stream.set_write_timeout(Some(Duration::from_secs(5)))?;
    write_message(&mut stream, &request)?;
    let reply: Reply = read_message(&mut stream)
        .context("waiting for the daemon to finish the command (30 second timeout)")?;
    if !reply.success {
        anyhow::bail!("{}", reply.message);
    }
    Ok(reply.message)
}

pub fn read_message<T: serde::de::DeserializeOwned>(stream: &mut UnixStream) -> Result<T> {
    // Bound both the wait and the message size so an incomplete request cannot
    // stall the service forever or consume unbounded memory.
    let mut line = String::new();
    BufReader::new(stream.take(16 * 1024)).read_line(&mut line)?;
    anyhow::ensure!(
        line.ends_with('\n'),
        "incomplete or oversized control message"
    );
    Ok(serde_json::from_str(&line)?)
}

pub fn write_message(stream: &mut UnixStream, message: &impl Serialize) -> Result<()> {
    serde_json::to_writer(&mut *stream, message)?;
    stream.write_all(b"\n")?;
    stream.flush()?;
    Ok(())
}
