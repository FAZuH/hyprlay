//! Control-socket transport adapter: the byte stream that carries the
//! one-line command/protocol to the daemon. A unix socket on Linux/macOS, a
//! named pipe on Windows. This is the concrete [`ControlEndpoint`] (client
//! connect) and [`ControlListener`] (daemon bind/accept) the fronts inject
//! into `hyprlay-core::ctl`.

use std::io::Read;
use std::io::Write;
use std::path::Path;

use hyprlay_core::ctl::ControlEndpoint;
use hyprlay_core::ctl::ControlListener;
use hyprlay_core::ctl::ControlStream;

/// The platform control transport (unit struct — the client side is
/// stateless; a daemon listener is owned by [`Control::listen`]).
pub struct Control;

impl Control {
    /// Bind the daemon-side control listener at `path`, returning a
    /// [`ControlListener`] the daemon accept loop serves on its own thread.
    pub fn listen(path: &Path) -> std::io::Result<Box<dyn ControlListener>> {
        imp::listen(path)
    }
}

#[cfg(unix)]
mod imp {
    use std::os::unix::net::UnixListener;
    use std::os::unix::net::UnixStream;

    use super::*;

    /// Newtype so we can implement the foreign [`ControlStream`] trait
    /// without an orphan-rule clash, and gain the half-close the protocol
    /// needs.
    struct Stream(UnixStream);

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    impl ControlStream for Stream {
        fn shutdown_write(&mut self) -> std::io::Result<()> {
            self.0.shutdown(std::net::Shutdown::Write)
        }
    }

    impl ControlEndpoint for Control {
        fn connect(&self, path: &Path) -> std::io::Result<Box<dyn ControlStream>> {
            Ok(Box::new(Stream(UnixStream::connect(path)?)))
        }
    }

    /// Daemon-side listener backed by a unix domain socket.
    struct Listener(UnixListener);

    impl ControlListener for Listener {
        fn accept(&self) -> std::io::Result<Box<dyn ControlStream>> {
            let (stream, _) = self.0.accept()?;
            Ok(Box::new(Stream(stream)))
        }
    }

    pub(super) fn listen(path: &Path) -> std::io::Result<Box<dyn ControlListener>> {
        Ok(Box::new(Listener(UnixListener::bind(path)?)))
    }
}

#[cfg(windows)]
mod imp {
    use std::io;

    use interprocess::local_socket::GenericNamespaced;
    use interprocess::local_socket::ListenerOptions;
    use interprocess::local_socket::Name;
    use interprocess::local_socket::prelude::*;

    use super::*;

    struct Stream(LocalSocketStream);

    impl Read for Stream {
        fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
            self.0.read(buf)
        }
    }
    impl Write for Stream {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.write(buf)
        }
        fn flush(&mut self) -> std::io::Result<()> {
            self.0.flush()
        }
    }
    impl ControlStream for Stream {
        fn shutdown_write(&mut self) -> std::io::Result<()> {
            // A named pipe has no distinct half-close; the one-line protocol
            // reads up to the newline rather than to a write-side EOF, so a
            // no-op is correct here.
            Ok(())
        }
    }

    /// The namespaced pipe name for the socket path: the runtime-dir prefix
    /// is meaningless on Windows, so only the file name is used. A
    /// `GenericNamespaced` name prepends `\\.\pipe\` for us, so callers never
    /// write that prefix by hand.
    fn pipe_name(path: &Path) -> io::Result<Name<'static>> {
        let name = path.file_name().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "control path has no file name")
        })?;
        Ok(name.to_ns_name::<GenericNamespaced>()?.into_owned())
    }

    impl ControlEndpoint for Control {
        fn connect(&self, path: &Path) -> std::io::Result<Box<dyn ControlStream>> {
            let stream = LocalSocketStream::connect(pipe_name(path)?)?;
            Ok(Box::new(Stream(stream)))
        }
    }

    /// Daemon-side listener backed by a named pipe. `ListenerOptions` uses
    /// first-instance ownership, so a second daemon binding the same name
    /// fails with `AddrInUse` instead of displacing the first.
    struct Listener(LocalSocketListener);

    impl ControlListener for Listener {
        fn accept(&self) -> std::io::Result<Box<dyn ControlStream>> {
            let stream = self.0.accept()?;
            Ok(Box::new(Stream(stream)))
        }
    }

    pub(super) fn listen(path: &Path) -> std::io::Result<Box<dyn ControlListener>> {
        let listener = ListenerOptions::new()
            .name(pipe_name(path)?)
            .create_sync()?;
        Ok(Box::new(Listener(listener)))
    }
}
