//! Own and reap every subprocess. A separate monitor can interrupt blocked pipe I/O.
use anyhow::{bail, Context, Result};
use std::{
    io::Read,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub struct Process {
    child: Arc<Mutex<Child>>,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    monitor: Option<JoinHandle<()>>,
    logger: Option<JoinHandle<()>>,
    log: Arc<Mutex<Vec<u8>>>,
}

impl Process {
    pub fn spawn(command: &mut Command, cancel: &Arc<AtomicBool>) -> Result<Self> {
        super::check_cancel(cancel)?;
        command
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000); // CREATE_NO_WINDOW
        }
        let mut child = command
            .spawn()
            .context("Cannot start FFmpeg/ffprobe. Install both and add them to PATH")?;
        let mut stderr = child.stderr.take().unwrap();
        let log = Arc::new(Mutex::new(Vec::new()));
        let log_copy = log.clone();
        let logger = thread::spawn(move || {
            let mut buffer = [0; 4096];
            while let Ok(n) = stderr.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                let mut log = log_copy.lock().unwrap();
                log.extend_from_slice(&buffer[..n]);
                let excess = log.len().saturating_sub(16384);
                log.drain(..excess);
            }
        });
        let child = Arc::new(Mutex::new(child));
        let stop = Arc::new(AtomicBool::new(false));
        let (watch_child, watch_stop, watch_cancel) = (child.clone(), stop.clone(), cancel.clone());
        let monitor = thread::spawn(move || {
            while !watch_stop.load(Ordering::Relaxed) {
                if watch_cancel.load(Ordering::Relaxed) {
                    let _ = watch_child.lock().unwrap().kill();
                    break;
                }
                thread::sleep(Duration::from_millis(25));
            }
        });
        Ok(Self {
            child,
            stop,
            cancel: cancel.clone(),
            monitor: Some(monitor),
            logger: Some(logger),
            log,
        })
    }
    pub fn stdin(&mut self) -> ChildStdin {
        self.child.lock().unwrap().stdin.take().unwrap()
    }
    pub fn stdout(&mut self) -> ChildStdout {
        self.child.lock().unwrap().stdout.take().unwrap()
    }
    pub fn wait(&mut self) -> Result<()> {
        let status = loop {
            super::check_cancel(&self.cancel)?;
            if let Some(status) = self.child.lock().unwrap().try_wait()? {
                break status;
            }
            thread::sleep(Duration::from_millis(25));
        };
        if let Some(logger) = self.logger.take() {
            let _ = logger.join();
        }
        if !status.success() {
            bail!(
                "FFmpeg failed: {}",
                String::from_utf8_lossy(&self.log.lock().unwrap())
            );
        }
        Ok(())
    }
}
impl Drop for Process {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        {
            let mut child = self.child.lock().unwrap();
            if !matches!(child.try_wait(), Ok(Some(_))) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
        if let Some(monitor) = self.monitor.take() {
            let _ = monitor.join();
        }
        if let Some(logger) = self.logger.take() {
            let _ = logger.join();
        }
    }
}
