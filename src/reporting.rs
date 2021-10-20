//! Utilities for better error reporting and tracing/logging.

use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::Path,
    sync::{Arc, Mutex},
};

use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::{
    prelude::__tracing_subscriber_SubscriberExt, util::SubscriberInitExt, EnvFilter,
};

#[derive(Default, Debug)]
struct LogFileWriterInternal {
    buffer: Vec<u8>,
    file: Option<File>,
}

impl LogFileWriterInternal {
    fn write_buffer_to_file(buffer: &mut Vec<u8>, file: &mut File) -> std::io::Result<()> {
        std::io::copy(&mut buffer.as_slice(), file)?;

        // For some reason the Read implementation for &[u8] doesn't
        // consume the values during the copy, so we clear it here.
        buffer.clear();

        Ok(())
    }
}

impl Write for LogFileWriterInternal {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        std::io::stdout().write(buf)?;

        if let Some(file) = &mut self.file {
            file.write(buf)
        } else {
            self.buffer.write(buf)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        if let Some(file) = &mut self.file {
            Self::write_buffer_to_file(&mut self.buffer, file)?;
            file.flush()?;
        } else {
            self.buffer.flush()?;
        }

        std::io::stdout().flush()
    }
}

/// This struct manages the writing to `integration-test.log` in the
/// current out directory. If there is no out directory, then output
/// is buffered in memory until there is one.
#[derive(Clone, Debug)]
pub struct LogFileWriter {
    internal: Arc<Mutex<LogFileWriterInternal>>,
}

impl Default for LogFileWriter {
    fn default() -> Self {
        Self::new()
    }
}

impl LogFileWriter {
    pub fn new() -> Self {
        LogFileWriter {
            internal: Default::default(),
        }
    }

    /// Stop logging to file, buffer log output in memory until
    /// [Self::set_out_file()] is called again, where the output will
    /// then be unbuffered to.
    pub fn set_no_out_file(&self) {
        let mut internal = self.internal.lock().expect("error obtaining lock");
        internal.file = None;
    }

    /// Sets the file to use for writing the logs to, and writes out
    /// any buffered log data.
    pub fn set_out_file(&self, path: impl AsRef<Path>) -> eyre::Result<()> {
        let path = path.as_ref();

        let mut internal = self.internal.lock().expect("error obtaining lock");
        internal.flush()?;

        let mut file = OpenOptions::new().append(true).create(true).open(path)?;

        // If there was no file previously, buffer may still contain
        // values (because the flush will not clear the buffer if
        // there was no file to write to).
        if internal.file.is_none() {
            LogFileWriterInternal::write_buffer_to_file(&mut internal.buffer, &mut file)?;
        }

        internal.file = Some(file);

        Ok(())
    }
}

impl Write for LogFileWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut internal = self.internal.lock().expect("error obtaining lock");
        internal.write(buf)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let mut internal = self.internal.lock().expect("error obtaining lock");
        internal.flush()
    }
}

#[must_use]
pub struct ReportGuard {
    _guard: WorkerGuard,
}

/// Set up [tracing] and [color-eyre](color_eyre).
pub fn setup_reporting(log_writer: LogFileWriter) -> eyre::Result<ReportGuard> {
    color_eyre::install()?;

    let filter_layer = EnvFilter::try_from_default_env().or_else(|_| EnvFilter::try_new("info"))?;

    let (tracing_log_writer, guard) = tracing_appender::non_blocking(log_writer);
    let fmt_layer = tracing_subscriber::fmt::layer().with_writer(tracing_log_writer);

    let error_layer = tracing_error::ErrorLayer::default();

    tracing_subscriber::registry()
        .with(filter_layer)
        .with(fmt_layer)
        .with(error_layer)
        .init();

    Ok(ReportGuard { _guard: guard })
}

#[cfg(test)]
mod test {
    use std::io::Write;

    use super::LogFileWriter;

    #[test]
    fn test_log_file_writer() {
        let out_dir = tempfile::tempdir().unwrap();
        let out_file = out_dir.path().join("test.log");

        let mut log_writer = LogFileWriter::new();
        log_writer.write(b"a").unwrap();
        assert!(!out_file.exists());

        let mut log_writer_1 = log_writer.clone();
        let join1 = std::thread::spawn(move || {
            for _ in 0..100 {
                log_writer_1.write(b"b").unwrap();
            }
        });

        let out_file_2 = out_file.clone();
        let join2 = std::thread::spawn(move || {
            log_writer.set_out_file(&out_file_2).unwrap();
            assert!(out_file_2.exists());
            let log_string = std::fs::read_to_string(&out_file_2).unwrap();
            assert!(!log_string.is_empty())
        });

        join1.join().unwrap();
        join2.join().unwrap();

        let log_string = std::fs::read_to_string(&out_file).unwrap();
        assert_eq!(101, log_string.len());

        println!();
    }
}
