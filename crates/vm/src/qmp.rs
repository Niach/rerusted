//! Client for QEMU's machine monitor (QMP): one JSON object per line.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Value, json};

use crate::error::{Error, Result};

pub struct Qmp {
    reader: BufReader<UnixStream>,
    writer: UnixStream,
}

impl Qmp {
    /// Connects to the monitor socket and completes the handshake.
    pub fn connect(socket: &Path) -> Result<Self> {
        let stream = UnixStream::connect(socket)
            .map_err(|e| Error::new(format!("monitor {}: {e}", socket.display())))?;
        Self::from_stream(stream)
    }

    pub fn from_stream(stream: UnixStream) -> Result<Self> {
        stream.set_read_timeout(Some(Duration::from_secs(30)))?;
        let writer = stream.try_clone()?;
        let mut qmp = Self {
            reader: BufReader::new(stream),
            writer,
        };
        let greeting = qmp.read()?;
        if greeting.get("QMP").is_none() {
            return Err(Error::new(format!("monitor sent no greeting: {greeting}")));
        }
        qmp.execute("qmp_capabilities", Value::Null)?;
        Ok(qmp)
    }

    /// Runs one command and returns its `return` value. Events that arrive
    /// in between are skipped.
    pub fn execute(&mut self, command: &str, arguments: Value) -> Result<Value> {
        let mut request = json!({ "execute": command });
        if !arguments.is_null() {
            request["arguments"] = arguments;
        }
        writeln!(self.writer, "{request}")?;
        loop {
            let mut reply = self.read()?;
            if let Some(value) = reply.get_mut("return") {
                return Ok(value.take());
            }
            if let Some(error) = reply.get("error") {
                let text = error["desc"].as_str().unwrap_or("unknown error");
                return Err(Error::new(format!("monitor refused {command}: {text}")));
            }
        }
    }

    fn read(&mut self) -> Result<Value> {
        let mut line = String::new();
        if self.reader.read_line(&mut line)? == 0 {
            return Err(Error::new("monitor closed the connection"));
        }
        Ok(serde_json::from_str(&line)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    /// Plays QEMU's side: greeting, then one scripted reply per request.
    fn fake(replies: Vec<&'static str>) -> (UnixStream, thread::JoinHandle<Vec<Value>>) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        let handle = thread::spawn(move || {
            let mut writer = theirs.try_clone().unwrap();
            let mut reader = BufReader::new(theirs);
            writeln!(
                writer,
                r#"{{"QMP": {{"version": {{}}, "capabilities": []}}}}"#
            )
            .unwrap();
            let mut seen = Vec::new();
            for reply in replies {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                seen.push(serde_json::from_str(&line).unwrap());
                writeln!(writer, "{reply}").unwrap();
            }
            seen
        });
        (ours, handle)
    }

    #[test]
    fn handshake_then_command_with_arguments() {
        let (stream, qemu) = fake(vec![
            r#"{"return": {}}"#,
            r#"{"return": {"running": true}}"#,
        ]);
        let mut qmp = Qmp::from_stream(stream).unwrap();
        let status = qmp.execute("query-status", json!({"x": 1})).unwrap();
        assert_eq!(status["running"], true);
        let seen = qemu.join().unwrap();
        assert_eq!(seen[0], json!({"execute": "qmp_capabilities"}));
        assert_eq!(
            seen[1],
            json!({"execute": "query-status", "arguments": {"x": 1}})
        );
    }

    #[test]
    fn events_before_the_reply_are_skipped() {
        let (stream, qemu) = fake(vec![
            r#"{"return": {}}"#,
            "{\"event\": \"RESUME\", \"timestamp\": {}}\n{\"return\": 7}",
        ]);
        let mut qmp = Qmp::from_stream(stream).unwrap();
        assert_eq!(qmp.execute("anything", Value::Null).unwrap(), 7);
        qemu.join().unwrap();
    }

    #[test]
    fn an_error_reply_names_the_command() {
        let (stream, qemu) = fake(vec![
            r#"{"return": {}}"#,
            r#"{"error": {"class": "GenericError", "desc": "no such head"}}"#,
        ]);
        let mut qmp = Qmp::from_stream(stream).unwrap();
        let error = qmp.execute("screendump", Value::Null).unwrap_err();
        assert_eq!(
            error.to_string(),
            "monitor refused screendump: no such head"
        );
        qemu.join().unwrap();
    }
}
