// src/packet_channel.rs
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use crate::constants::{PACKET_HEADER_SIZE, TIMEOUT_LATENCY_DELTA};
use crate::replica_options::ReplicaOptions;
use crate::ssl_mode::SslMode;

trait IoRW: Read + Write + Send {}
impl<T: Read + Write + Send> IoRW for T {}
type BoxIo = Box<dyn IoRW>;

enum Transport {
    Tcp(TcpStream),
    Tls(BoxIo),
}

impl Read for Transport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            Transport::Tcp(s) => s.read(buf),
            Transport::Tls(s) => s.read(buf),
        }
    }
}
impl Write for Transport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            Transport::Tcp(s) => s.write(buf),
            Transport::Tls(s) => s.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            Transport::Tcp(s) => s.flush(),
            Transport::Tls(s) => s.flush(),
        }
    }
}

pub struct PacketChannel {
    stream: Transport,
    ssl_mode: SslMode,
    hostname: String,
    #[allow(dead_code)]
    port: u16,
    #[allow(dead_code)]
    read_timeout: Option<Duration>,
}

impl PacketChannel {
    pub fn new(options: &ReplicaOptions) -> Result<Self, io::Error> {
        let address = format!("{}:{}", options.hostname, options.port);
        let tcp = TcpStream::connect(address)?;
        let read_timeout = Some(options.heartbeat_interval + TIMEOUT_LATENCY_DELTA);
        tcp.set_read_timeout(read_timeout)?;
        Ok(Self {
            stream: Transport::Tcp(tcp),
            ssl_mode: options.ssl_mode,
            hostname: options.hostname.clone(),
            port: options.port,
            read_timeout,
        })
    }

    pub fn read_packet(&mut self) -> Result<(Vec<u8>, u8), io::Error> {
        let mut header = [0; PACKET_HEADER_SIZE];
        self.stream.read_exact(&mut header)?;
        let packet_size = (&header[0..3]).read_u24::<LittleEndian>()?;
        let seq_num = header[3];

        let mut packet = vec![0u8; packet_size as usize];
        self.stream.read_exact(&mut packet)?;
        Ok((packet, seq_num))
    }

    pub fn write_packet(&mut self, packet: &[u8], seq_num: u8) -> Result<(), io::Error> {
        let packet_len = packet.len() as u32;
        self.stream.write_u24::<LittleEndian>(packet_len)?;
        self.stream.write_u8(seq_num)?;
        self.stream.write_all(packet)?;
        Ok(())
    }
    #[allow(dead_code)]
    fn reconnect_plain(&mut self) -> io::Result<()> {
        // Used only for IfAvailable fallback on native-tls connect failure.
        let addr = format!("{}:{}", self.hostname, self.port);
        let tcp = TcpStream::connect(addr)?;
        if let Some(rt) = self.read_timeout {
            tcp.set_read_timeout(Some(rt))?;
        }
        self.stream = Transport::Tcp(tcp);
        Ok(())
    }

    /// Native-TLS upgrade (compiled with `--features native-tls`)
    #[cfg(feature = "native-tls")]
    pub fn upgrade_to_ssl_native_tls(&mut self) -> Result<bool, io::Error> {
        use native_tls::TlsConnector;

        if matches!(self.ssl_mode, SslMode::Disabled) {
            return Ok(false);
        }

        let mut b = TlsConnector::builder();
        match self.ssl_mode {
            SslMode::IfAvailable | SslMode::Require => {
                b.danger_accept_invalid_certs(true);
                b.danger_accept_invalid_hostnames(true);
            }
            SslMode::RequireVerifyCa => {
                b.danger_accept_invalid_certs(false);
                b.danger_accept_invalid_hostnames(true);
            }
            SslMode::RequireVerifyFull => { /* defaults */ }
            SslMode::Disabled => unreachable!(),
        }
        let connector = b.build().map_err(tls_other)?;

        // Take the TcpStream
        let tcp = match std::mem::replace(&mut self.stream, Transport::Tls(Box::new(std::io::Cursor::new(Vec::<u8>::new())))) {
            Transport::Tcp(s) => s,
            other => {
                // Already TLS; put it back and say "true"
                self.stream = other;
                return Ok(true);
            }
        };

        match connector.connect(&self.hostname, tcp) {
            Ok(tls) => {
                self.stream = Transport::Tls(Box::new(tls));
                Ok(true)
            }
            Err(e) => {
                if matches!(self.ssl_mode, SslMode::IfAvailable) {
                    // Fallback to cleartext
                    self.reconnect_plain()?;
                    Ok(false)
                } else {
                    Err(tls_other(e))
                }
            }
        }
    }

    /// Rustls upgrade (compiled with `--features rustls-tls`)
    #[cfg(feature = "rustls-tls")]
    pub fn upgrade_to_ssl_rustls(&mut self) -> Result<bool, io::Error> {
        use rustls::{ClientConfig, ClientConnection, StreamOwned};
        use rustls_pki_types::ServerName;
        use std::sync::Arc;

        if matches!(self.ssl_mode, SslMode::Disabled) {
            return Ok(false);
        }

        let mut roots = rustls::RootCertStore::empty();

        // Root store: OS first, fallback to webpki-roots
        let native = rustls_native_certs::load_native_certs();
        if !native.errors.is_empty() {
            // log::warn!("native cert load had {} errors; proceeding", native.errors.len());
        }
        for cert in native.certs {
            let _ = roots.add(cert);
        }
        if roots.is_empty() {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
        }
       
        // Map SslMode -> config
        let cfg: ClientConfig = match self.ssl_mode {
            // "No verify" (dev only) – needs `dangerous-rustls`
            SslMode::IfAvailable | SslMode::Require => {
                #[cfg(feature = "dangerous-rustls")]
                {
                    use rustls::client::danger::{ServerCertVerified, ServerCertVerifier};
                    #[derive(Debug)]
                    struct NoVerify;
                    impl ServerCertVerifier for NoVerify {
                        fn verify_server_cert(
                            &self,
                            _end_entity: &rustls_pki_types::CertificateDer<'_>,
                            _intermediates: &[rustls_pki_types::CertificateDer<'_>],
                            _server_name: &ServerName<'_>,
                            _scts: &mut dyn Iterator<Item = &[u8]>,
                            _ocsp: &[u8],
                            _now: std::time::SystemTime,
                        ) -> Result<ServerCertVerified, rustls::Error> {
                            Ok(ServerCertVerified::assertion())
                        }
                    }
                    ClientConfig::builder()
                        .dangerous()
                        .with_custom_certificate_verifier(Arc::new(NoVerify))
                        .with_no_client_auth()
                }
                #[cfg(not(feature = "dangerous-rustls"))]
                {
                    ClientConfig::builder().with_root_certificates(roots).with_no_client_auth()
                }
            }
            // CA-only verify (skip hostname) would need a custom verifier; treat as full for now.
            SslMode::RequireVerifyCa => {
                ClientConfig::builder().with_root_certificates(roots).with_no_client_auth()
            }
            SslMode::RequireVerifyFull => {
                ClientConfig::builder().with_root_certificates(roots).with_no_client_auth()
            }
            SslMode::Disabled => unreachable!(),
        };

       let server_name = match ServerName::try_from(self.hostname.clone()) {
            Ok(s) => s, // DNS name, now owned => 'static
            Err(_) => {
                // Host is an IP literal; this variant is 'static by construction
                let ip: std::net::IpAddr = self.hostname
                    .parse()
                    .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
                ServerName::IpAddress(ip.into())
            }
        };

        // Take the TcpStream
        let tcp = match std::mem::replace(&mut self.stream, Transport::Tls(Box::new(io::empty()))) {
            Transport::Tcp(s) => s,
            other => {
                self.stream = other;
                return Ok(true); // already TLS
            }
        };

        let conn = ClientConnection::new(Arc::new(cfg), server_name)
            .map_err(|e| io::Error::new(io::ErrorKind::Other, format!("TLS error: {e}")))?;
        let tls = StreamOwned::new(conn, tcp);
        self.stream = Transport::Tls(Box::new(tls));
        Ok(true)
    }
}

#[cfg(feature = "native-tls")]
fn tls_other<E: std::fmt::Display>(e: E) -> io::Error {
    io::Error::new(io::ErrorKind::Other, format!("TLS error: {e}"))
}
