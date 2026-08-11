use std::fs;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct TempHome(PathBuf);

impl TempHome {
    fn new() -> Self {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path =
            std::env::temp_dir().join(format!("bo-snap-output-{}-{suffix}", std::process::id()));
        fs::create_dir_all(path.join(".bo/notes")).unwrap();
        fs::write(
            path.join(".bo/notes/state.json"),
            "{\n  \"raw\": [],\n  \"summaries\": []\n}\n",
        )
        .unwrap();
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

fn run(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_bo"))
        .args(args)
        .env("HOME", home)
        .env_remove("USERPROFILE")
        .output()
        .unwrap()
}

fn request(stream: &mut TcpStream) -> String {
    let mut bytes = Vec::new();
    let mut byte = [0; 1];
    while !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte).unwrap();
        bytes.push(byte[0]);
    }
    String::from_utf8(bytes).unwrap()
}

fn response(stream: &mut TcpStream, status: &str, headers: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).unwrap();
}

#[test]
fn snap_separates_success_output_and_diagnostics() {
    let home = TempHome::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert!(request(&mut stream).contains("GET /ok "));
        response(
            &mut stream,
            "200 OK",
            "Content-Type: text/html\r\n",
            "<html><head><title>Page</title></head><body><article>content</article></body></html>",
        );

        let (mut stream, _) = listener.accept().unwrap();
        assert!(request(&mut stream).contains("GET /missing "));
        response(
            &mut stream,
            "404 Not Found",
            "Content-Type: text/plain\r\nX-Request-Id: request-123\r\n",
            "private response body",
        );
    });

    let success_url = format!("http://{address}/ok");
    let failure_url = format!("http://{address}/missing");
    let output = run(home.path(), &["snap", "notes", &success_url, &failure_url]);
    server.join().unwrap();

    assert!(!output.status.success());
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert_eq!(stdout, format!("snapped: {success_url} -> page.md\n"));
    assert_eq!(
        stderr,
        format!(
            "failed: {failure_url} (http: HTTP 404 (request_id: request-123))\n1 succeeded / 1 failed\n"
        )
    );
    assert!(!stderr.contains("private response body"));
    assert!(home.path().join(".bo/notes/page.md").is_file());
}

#[test]
fn snap_reports_state_write_failure_with_source_context() {
    let home = TempHome::new();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let first_url = format!("http://{address}/first");
    let second_url = format!("http://{address}/second");
    let unattempted_url = format!("http://{address}/unattempted");
    let target = home.path().join(".bo/notes");
    let first_url_for_server = first_url.clone();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert!(request(&mut stream).contains("GET /first "));
        response(
            &mut stream,
            "200 OK",
            "Content-Type: text/html\r\n",
            "<html><head><title>First Page</title></head><body><article>content</article></body></html>",
        );

        while !fs::read_to_string(target.join("state.json"))
            .unwrap()
            .contains(&first_url_for_server)
        {
            thread::sleep(Duration::from_millis(1));
        }
        fs::create_dir(target.join(".state.json.tmp")).unwrap();

        let (mut stream, _) = listener.accept().unwrap();
        assert!(request(&mut stream).contains("GET /second "));
        response(
            &mut stream,
            "200 OK",
            "Content-Type: text/html\r\n",
            "<html><head><title>Second Page</title></head><body><article>content</article></body></html>",
        );
    });

    let output = run(
        home.path(),
        &["snap", "notes", &first_url, &second_url, &unattempted_url],
    );
    server.join().unwrap();

    assert!(!output.status.success());
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        format!("snapped: {first_url} -> first-page.md\n")
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains(&format!(
        "failed: {second_url} (filesystem: updating state failed:"
    )));
    assert!(!stderr.contains(&unattempted_url));
    assert!(stderr.contains("1 succeeded / 1 failed; batch aborted"));
    assert!(stderr.contains("state.json"));
    assert!(stderr.contains("snapshot written then deleted"));
    let state = fs::read_to_string(home.path().join(".bo/notes/state.json")).unwrap();
    assert!(state.contains(&first_url));
    assert!(!state.contains(&second_url));
    assert!(!home.path().join(".bo/notes/second-page.md").exists());
}
