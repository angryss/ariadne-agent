#[cfg(unix)]
mod unix {
    use std::net::{TcpListener, TcpStream};
    use std::process::Command as ProcessCommand;
    use std::thread;
    use std::time::{Duration, Instant};

    #[test]
    fn serve_shuts_down_cleanly_on_sigterm() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);

        let mut command = ProcessCommand::new(assert_cmd::cargo::cargo_bin!("ariadne"));
        let mut child = command
            .args(["serve", "--bind", &address.to_string()])
            .spawn()
            .unwrap();

        let ready_deadline = Instant::now() + Duration::from_secs(5);
        while TcpStream::connect(address).is_err() {
            if Instant::now() >= ready_deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Ariadne server did not become ready");
            }
            thread::sleep(Duration::from_millis(20));
        }

        let signal = ProcessCommand::new("kill")
            .args(["-TERM", &child.id().to_string()])
            .status()
            .unwrap();
        assert!(signal.success());

        let exit_deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                assert!(status.success(), "server exited unsuccessfully: {status}");
                break;
            }
            if Instant::now() >= exit_deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("Ariadne server did not stop after SIGTERM");
            }
            thread::sleep(Duration::from_millis(20));
        }
    }
}
