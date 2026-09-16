//! Process-level boot test for the shepherd-server binary.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn server_boots_from_unrelated_cwd_and_never_touches_home() {
    let fake_home = tempfile::tempdir().unwrap();
    let unrelated_cwd = tempfile::tempdir().unwrap();
    let ui_dist = tempfile::tempdir().unwrap();
    std::fs::write(
        ui_dist.path().join("index.html"),
        "<!doctype html><html><body><div id=\"root\"></div></body></html>",
    )
    .unwrap();

    let child = Command::new(env!("CARGO_BIN_EXE_shepherd-server"))
        .args(["--port", "0", "--ui-dir"])
        .arg(ui_dist.path())
        .env("HOME", fake_home.path())
        .current_dir(unrelated_cwd.path())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("failed to spawn shepherd-server");
    let mut child = KillOnDrop(child);

    let stdout = child.0.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    let listen_line = lines
        .next()
        .expect("server exited without printing a listen line")
        .unwrap();
    let addr = listen_line
        .split("http://")
        .nth(1)
        .expect("listen line must contain the bound address")
        .trim();

    let deadline = Instant::now() + Duration::from_secs(5);
    let mut stream = loop {
        match TcpStream::connect(addr) {
            Ok(stream) => break stream,
            Err(err) if Instant::now() < deadline => {
                let _ = err;
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(err) => panic!("could not connect to {addr}: {err}"),
        }
    };
    write!(
        stream,
        "GET /health HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();

    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected 200 from /health, got: {response}"
    );
    assert!(
        response.contains("\"status\":\"pass\""),
        "expected status pass, got: {response}"
    );

    assert!(
        !fake_home.path().join(".shepherd").exists(),
        "the scaffold must never create ~/.shepherd"
    );
    let leftovers: Vec<_> = std::fs::read_dir(fake_home.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "the scaffold must not write into HOME, found: {leftovers:?}"
    );
}
