// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 The InvoiceKit Authors

//! `invoicekit-binding-rest-shim` binary.

use std::io::{Read as _, Write as _};
use std::net::{Shutdown, TcpStream, ToSocketAddrs};
use std::time::Duration;

const DEFAULT_BIND: &str = "127.0.0.1:8081";
const DEFAULT_HEALTH_URL: &str = "http://127.0.0.1:8081/healthz";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    if std::env::args().any(|arg| arg == "--healthcheck") {
        run_healthcheck()?;
        return Ok(());
    }

    let bind = std::env::var("INVOICEKIT_REST_BIND").unwrap_or_else(|_| DEFAULT_BIND.to_owned());
    invoicekit_binding_rest_shim::serve(&bind).await?;
    Ok(())
}

fn run_healthcheck() -> std::io::Result<()> {
    let url = std::env::var("INVOICEKIT_REST_HEALTH_URL")
        .unwrap_or_else(|_| DEFAULT_HEALTH_URL.to_owned());
    let endpoint = HealthEndpoint::parse(&url)?;
    let mut addrs = format!("{}:{}", endpoint.host, endpoint.port).to_socket_addrs()?;
    let addr = addrs
        .next()
        .ok_or_else(|| invalid_input("health URL host resolved to no socket addresses"))?;
    let mut stream = TcpStream::connect_timeout(&addr, Duration::from_secs(2))?;
    stream.set_read_timeout(Some(Duration::from_secs(2)))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: {}:{}\r\nConnection: close\r\n\r\n",
        endpoint.path, endpoint.host, endpoint.port
    )?;
    let mut response = String::new();
    stream.read_to_string(&mut response)?;
    let ok = response.starts_with("HTTP/1.1 200") || response.starts_with("HTTP/1.0 200");
    let _ = stream.shutdown(Shutdown::Both);
    if ok {
        Ok(())
    } else {
        Err(invalid_input("health endpoint did not return HTTP 200"))
    }
}

struct HealthEndpoint {
    host: String,
    port: u16,
    path: String,
}

impl HealthEndpoint {
    fn parse(url: &str) -> std::io::Result<Self> {
        let without_scheme = url
            .strip_prefix("http://")
            .ok_or_else(|| invalid_input("health URL must use http://"))?;
        let (authority, path_suffix) = without_scheme
            .split_once('/')
            .map_or((without_scheme, ""), |(authority, path)| (authority, path));
        let (host, port) = authority
            .rsplit_once(':')
            .ok_or_else(|| invalid_input("health URL must include host:port"))?;
        let port = port
            .parse()
            .map_err(|_| invalid_input("health URL port is not a valid u16"))?;
        let path = format!("/{path_suffix}");
        Ok(Self {
            host: host.to_owned(),
            port,
            path,
        })
    }
}

fn invalid_input(message: &str) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidInput, message)
}
