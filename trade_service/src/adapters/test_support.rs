pub(crate) mod read_rpc_server;

use std::{
    io::{BufRead, BufReader},
    process::{Child, Command, Stdio},
    sync::mpsc::{self, Receiver},
    thread::JoinHandle,
    time::{Duration, Instant},
};

pub(crate) struct IsolatedTestProcess {
    child: Child,
    lines: Receiver<String>,
    output: Option<JoinHandle<()>>,
}

impl IsolatedTestProcess {
    pub(crate) fn spawn(exact_test: &str, flag: &str, value: &str) -> Self {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", exact_test, "--nocapture", "--test-threads=1"])
            .env(flag, value)
            .current_dir(std::env::temp_dir())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        // Only this child is modified; a loopback fixture cannot inherit an outbound proxy.
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(key);
        }
        command.env("NO_PROXY", "*").env("no_proxy", "*");
        let mut child = command.spawn().unwrap();
        let stdout = child.stdout.take().unwrap();
        let (tx, lines) = mpsc::channel();
        let output = std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { return };
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        Self {
            child,
            lines,
            output: Some(output),
        }
    }

    pub(crate) fn await_line(&self, marker: &str) {
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            let line = self
                .lines
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .unwrap_or_else(|error| panic!("test child did not report {marker}: {error}"));
            if line.contains(marker) {
                return;
            }
        }
    }

    #[cfg(unix)]
    pub(crate) fn signal(&mut self, signal: &str) {
        assert!(matches!(signal, "-TERM" | "-INT"));
        assert!(self.child.try_wait().unwrap().is_none());
        assert!(Command::new("/bin/kill")
            .args([signal, &self.child.id().to_string()])
            .status()
            .unwrap()
            .success());
    }

    pub(crate) fn finish(mut self) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success(), "test child exited with {status}");
                break;
            }
            assert!(Instant::now() < deadline, "test child did not exit");
            std::thread::sleep(Duration::from_millis(10));
        }
        self.output.take().unwrap().join().unwrap();
    }
}

impl Drop for IsolatedTestProcess {
    fn drop(&mut self) {
        if let Ok(None) = self.child.try_wait() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
        if let Some(output) = self.output.take() {
            let _ = output.join();
        }
    }
}
