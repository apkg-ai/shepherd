use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct KillOnDrop(Child);

impl Drop for KillOnDrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

struct Daemon {
    child: KillOnDrop,
    addr: String,
    // Kept open so later daemon prints cannot EPIPE, and so tests can drain
    // everything printed after the listen line.
    stdout: std::io::Lines<BufReader<std::process::ChildStdout>>,
}

impl Daemon {
    /// Kills the daemon and returns everything it printed after the listen
    /// line, plus all of stderr.
    fn kill_and_collect_output(mut self) -> String {
        let _ = self.child.0.kill();
        let _ = self.child.0.wait();
        let mut collected = String::new();
        for line in self.stdout.by_ref() {
            let Ok(line) = line else { break };
            collected.push_str(&line);
            collected.push('\n');
        }
        if let Some(mut err) = self.child.0.stderr.take() {
            let _ = err.read_to_string(&mut collected);
        }
        collected
    }
}

fn spawn_daemon(data_dir: &Path, ui_dir: &Path, home: &Path, extra: &[&str]) -> Daemon {
    let mut args = vec!["--port", "0", "--ui-dir"];
    let ui = ui_dir.to_str().unwrap();
    let data = data_dir.to_str().unwrap();
    args.push(ui);
    args.push("--data-dir");
    args.push(data);
    args.extend_from_slice(extra);
    let child = Command::new(env!("CARGO_BIN_EXE_shepherd-server"))
        .args(&args)
        .env("HOME", home)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("failed to spawn shepherd-server");
    let mut child = KillOnDrop(child);

    let stdout = child.0.stdout.take().unwrap();
    let mut lines = BufReader::new(stdout).lines();
    // Startup may print e.g. the reissue notice before the listen line.
    let listen_line = lines
        .by_ref()
        .map(|line| line.unwrap())
        .find(|line| line.contains("listening on"))
        .expect("server exited without printing a listen line");
    let addr = listen_line
        .split("http://")
        .nth(1)
        .expect("listen line must contain the bound address")
        .trim()
        .to_string();
    Daemon {
        child,
        addr,
        stdout: lines,
    }
}

fn http(addr: &str, request_head: &str, body: &str) -> String {
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
        "{request_head}Host: {addr}\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{body}",
        body.len()
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn ui_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("index.html"),
        "<!doctype html><html><body><div id=\"root\"></div></body></html>",
    )
    .unwrap();
    dir
}

#[test]
fn server_boots_from_explicit_data_dir_and_never_touches_home() {
    let fake_home = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let ui_dist = ui_dir();

    let daemon = spawn_daemon(data_dir.path(), ui_dist.path(), fake_home.path(), &[]);
    let response = http(&daemon.addr, "GET /health HTTP/1.1\r\n", "");
    assert!(
        response.starts_with("HTTP/1.1 200"),
        "expected 200 from /health, got: {response}"
    );
    assert!(
        response.contains("\"status\":\"pass\""),
        "expected status pass, got: {response}"
    );

    // Owner bootstrap provisioned the identity files with restrictive modes.
    use std::os::unix::fs::PermissionsExt;
    for file in ["owner-token", "replay-key"] {
        let path = data_dir.path().join(file);
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "{file} must be 0600");
    }
    assert!(data_dir.path().join("shepherd.db").exists());
    assert!(data_dir.path().join("daemon.lock").exists());

    let leftovers: Vec<_> = std::fs::read_dir(fake_home.path())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(
        leftovers.is_empty(),
        "an explicit --data-dir must keep HOME untouched, found: {leftovers:?}"
    );
    drop(daemon);
}

#[test]
fn second_daemon_is_refused_by_the_advisory_lock() {
    let fake_home = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let ui_dist = ui_dir();

    let first = spawn_daemon(data_dir.path(), ui_dist.path(), fake_home.path(), &[]);
    let second = Command::new(env!("CARGO_BIN_EXE_shepherd-server"))
        .args([
            "--port",
            "0",
            "--ui-dir",
            ui_dist.path().to_str().unwrap(),
            "--data-dir",
            data_dir.path().to_str().unwrap(),
        ])
        .env("HOME", fake_home.path())
        .output()
        .expect("failed to run second daemon");
    assert!(
        !second.status.success(),
        "the second daemon must exit nonzero"
    );
    let stderr = String::from_utf8_lossy(&second.stderr);
    assert!(
        stderr.contains("another shepherd-server"),
        "expected a lock diagnostic, got: {stderr}"
    );
    drop(first);
}

#[test]
fn no_secrets_in_server_output() {
    let fake_home = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let ui_dist = ui_dir();

    let daemon = spawn_daemon(data_dir.path(), ui_dist.path(), fake_home.path(), &[]);
    let owner_token = std::fs::read_to_string(data_dir.path().join("owner-token")).unwrap();

    // A real login flows the owner token and mints a session cookie.
    let login = http(
        &daemon.addr,
        "POST /api/v1/session HTTP/1.1\r\nContent-Type: application/json\r\n",
        &format!("{{\"owner_token\":\"{owner_token}\"}}"),
    );
    assert!(login.starts_with("HTTP/1.1 201"), "login failed: {login}");
    let cookie_value = login
        .lines()
        .find_map(|line| line.strip_prefix("set-cookie: shepherd_session="))
        .map(|rest| rest.split(';').next().unwrap().to_string())
        .expect("login must set the session cookie");

    let combined = daemon.kill_and_collect_output();
    // The startup lines print resolved paths, so the output is nonempty even
    // when no secret leaked (guards against scanning an accidentally empty
    // stream, which would pass vacuously).
    assert!(
        combined.contains("data dir:"),
        "expected startup output, got: {combined:?}"
    );
    let owner_token = owner_token.trim();
    assert!(
        !combined.contains(owner_token),
        "owner token leaked to output"
    );
    assert!(
        !combined.contains(&cookie_value),
        "session cookie leaked to output"
    );
}

#[test]
fn missing_owner_token_fails_loud_and_reissue_recovers() {
    let fake_home = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let ui_dist = ui_dir();

    let daemon = spawn_daemon(data_dir.path(), ui_dist.path(), fake_home.path(), &[]);
    drop(daemon);
    std::fs::remove_file(data_dir.path().join("owner-token")).unwrap();

    let failed = Command::new(env!("CARGO_BIN_EXE_shepherd-server"))
        .args([
            "--port",
            "0",
            "--ui-dir",
            ui_dist.path().to_str().unwrap(),
            "--data-dir",
            data_dir.path().to_str().unwrap(),
        ])
        .env("HOME", fake_home.path())
        .output()
        .expect("failed to run daemon");
    assert!(!failed.status.success(), "missing owner token must abort");
    let stderr = String::from_utf8_lossy(&failed.stderr);
    assert!(
        stderr.contains("--reissue-owner-token"),
        "diagnostic must name the recovery flag, got: {stderr}"
    );

    let recovered = spawn_daemon(
        data_dir.path(),
        ui_dist.path(),
        fake_home.path(),
        &["--reissue-owner-token"],
    );
    let response = http(&recovered.addr, "GET /health HTTP/1.1\r\n", "");
    assert!(response.contains("\"status\":\"pass\""), "{response}");
    assert!(data_dir.path().join("owner-token").exists());
}

#[test]
fn missing_replay_key_enters_diagnostic_only_mode() {
    let fake_home = tempfile::tempdir().unwrap();
    let data_dir = tempfile::tempdir().unwrap();
    let ui_dist = ui_dir();

    let daemon = spawn_daemon(data_dir.path(), ui_dist.path(), fake_home.path(), &[]);
    drop(daemon);
    std::fs::remove_file(data_dir.path().join("replay-key")).unwrap();

    let daemon = spawn_daemon(data_dir.path(), ui_dist.path(), fake_home.path(), &[]);
    let health = http(&daemon.addr, "GET /health HTTP/1.1\r\n", "");
    assert!(
        health.contains("\"status\":\"warn\""),
        "health must warn in diagnostic mode: {health}"
    );
    let api = http(&daemon.addr, "GET /api/v1/principal HTTP/1.1\r\n", "");
    assert!(
        api.starts_with("HTTP/1.1 503"),
        "API must be 503 in diagnostic mode: {api}"
    );
    assert!(api.contains("integrity_failure"), "{api}");
    // Never silently regenerate the replay key (plan/12).
    assert!(!data_dir.path().join("replay-key").exists());
}
