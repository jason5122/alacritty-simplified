use std::ffi::OsStr;
use std::io::{Error as IoError, ErrorKind, Result as IoResult, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::{env, fs};

use crate::cli::SocketMessage;

/// Environment variable name for the IPC socket path.
const ALACRITTY_SOCKET_ENV: &str = "ALACRITTY_SOCKET";

/// Send a message to the active Alacritty socket.
pub fn send_message(socket: Option<PathBuf>, message: SocketMessage) -> IoResult<()> {
    let mut socket = find_socket(socket)?;

    let message = serde_json::to_string(&message)?;
    socket.write_all(message[..].as_bytes())?;
    let _ = socket.flush();

    Ok(())
}

/// Directory for the IPC socket file.
#[cfg(not(target_os = "macos"))]
fn socket_dir() -> PathBuf {
    xdg::BaseDirectories::with_prefix("alacritty")
        .ok()
        .and_then(|xdg| xdg.get_runtime_directory().map(ToOwned::to_owned).ok())
        .and_then(|path| fs::create_dir_all(&path).map(|_| path).ok())
        .unwrap_or_else(env::temp_dir)
}

/// Directory for the IPC socket file.
#[cfg(target_os = "macos")]
fn socket_dir() -> PathBuf {
    env::temp_dir()
}

/// Find the IPC socket path.
fn find_socket(socket_path: Option<PathBuf>) -> IoResult<UnixStream> {
    // Handle --socket CLI override.
    if let Some(socket_path) = socket_path {
        // Ensure we inform the user about an invalid path.
        return UnixStream::connect(&socket_path).map_err(|err| {
            let message = format!("invalid socket path {:?}", socket_path);
            IoError::new(err.kind(), message)
        });
    }

    // Handle environment variable.
    if let Ok(path) = env::var(ALACRITTY_SOCKET_ENV) {
        let socket_path = PathBuf::from(path);
        if let Ok(socket) = UnixStream::connect(socket_path) {
            return Ok(socket);
        }
    }

    // Search for sockets files.
    for entry in fs::read_dir(socket_dir())?.filter_map(|entry| entry.ok()) {
        let path = entry.path();

        // Skip files that aren't Alacritty sockets.
        let socket_prefix = socket_prefix();
        if path
            .file_name()
            .and_then(OsStr::to_str)
            .filter(|file| file.starts_with(&socket_prefix) && file.ends_with(".sock"))
            .is_none()
        {
            continue;
        }

        // Attempt to connect to the socket.
        match UnixStream::connect(&path) {
            Ok(socket) => return Ok(socket),
            // Delete orphan sockets.
            Err(error) if error.kind() == ErrorKind::ConnectionRefused => {
                let _ = fs::remove_file(&path);
            },
            // Ignore other errors like permission issues.
            Err(_) => (),
        }
    }

    Err(IoError::new(ErrorKind::NotFound, "no socket found"))
}

/// File prefix matching all available sockets.
///
/// This prefix will include display server information to allow for environments with multiple
/// display servers running for the same user.
#[cfg(not(target_os = "macos"))]
fn socket_prefix() -> String {
    let display = env::var("WAYLAND_DISPLAY").or_else(|_| env::var("DISPLAY")).unwrap_or_default();
    format!("Alacritty-{}", display.replace('/', "-"))
}

/// File prefix matching all available sockets.
#[cfg(target_os = "macos")]
fn socket_prefix() -> String {
    String::from("Alacritty")
}
