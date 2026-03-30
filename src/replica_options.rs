use crate::binlog_options::BinlogOptions;
use crate::ssl_mode::SslMode;
use std::borrow::Cow;
use std::str::FromStr;
use std::time::Duration;
use std::num::ParseIntError;


#[derive(Debug, thiserror::Error)]
pub enum ParseReplicaOptionsError {
    #[error("invalid URL: {0}")]
    Url(#[from] url::ParseError),

    #[error("invalid boolean for '{0}': '{1}'")]
    Bool(&'static str, String),

    #[error("invalid integer for '{0}': {1}")]
    Int(&'static str, ParseIntError),

    #[error("invalid duration for '{0}': '{1}' (try 30, 30s, 500ms)")]
    Duration(&'static str, String),

    #[error("invalid SSL mode: '{0}'")]
    SslMode(String),

    #[error("invalid binlog option: '{0}'")]
    Binlog(String),

    #[error("binlog_pos requires binlog_file")]
    BinlogFileMissing,

    #[error("missing host in URL")]
    MissingHost,
}

/// Settings used to connect to MySQL/MariaDB.
pub struct ReplicaOptions {
    /// Port number to connect. Defaults to 3306.
    pub port: u16,

    /// Hostname to connect. Defaults to "localhost".
    pub hostname: String,

    /// Defines whether SSL/TLS must be used. Defaults to SslMode.DISABLED.
    pub ssl_mode: SslMode,

    /// A database user which is used to register as a database slave.
    /// The user needs to have <c>REPLICATION SLAVE</c>, <c>REPLICATION CLIENT</c> privileges.
    pub username: String,

    /// The password of the user which is used to connect.
    pub password: Option<String>,

    /// Default database name specified in Handshake connection.
    /// Has nothing to do with filtering events by database name.
    pub database: Option<String>,

    /// Specifies the slave server id and used only in blocking mode. Defaults to 65535.
    /// <a href="https://dev.mysql.com/doc/refman/8.0/en/mysqlbinlog-server-id.html">See more</a>
    pub server_id: u32,

    /// Specifies whether to stream events or read until last event and then return.
    /// Defaults to true (stream events and wait for new ones).
    pub blocking: bool,

    /// Defines interval of keep alive messages that the master sends to the slave.
    /// Defaults to 30 seconds.
    pub heartbeat_interval: Duration,

    /// Defines the binlog coordinates that replication should start from.
    /// Defaults to BinlogOptions.FromEnd()
    pub binlog: BinlogOptions,
}

impl Default for ReplicaOptions {
    fn default() -> ReplicaOptions {
        ReplicaOptions {
            port: 3306,
            hostname: String::from("localhost"),
            ssl_mode: SslMode::Disabled,
            username: String::new(),
            password: Some(String::new()),
            database: None,
            server_id: 65535,
            blocking: true,
            heartbeat_interval: Duration::from_secs(30),
            binlog: BinlogOptions::from_end(),
        }
    }
}

impl FromStr for ReplicaOptions {
    type Err = ParseReplicaOptionsError;

    /// Examples:
    /// - mysql://user:pass@127.0.0.1:3306/mydb
    /// - mysqls://user:pass@db/mydb?ssl_mode=verify_identity&server_id=123&blocking=false&heartbeat=30s
    /// - mysql://user:pass@db?binlog=file:mysql-bin.000123:456
    /// - mysql://u:p@db/mydb?binlog_file=mysql-bin.000123&binlog_pos=456
    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let url = url::Url::parse(input)?;

        let mut opts = ReplicaOptions::default();

        // Scheme can imply SSL
        match url.scheme() {
            "mysqls" => {
                // If user also sets ssl_mode, that will override later.
                opts.ssl_mode = SslMode::IfAvailable;
            }
            "mysql" => { /* keep default */ }
            _ => {
                // Accept other aliases if you like, but we'll keep it strict:
                return Err(ParseReplicaOptionsError::Url(url::ParseError::IdnaError));
            }
        }

        // Host
        let host = url.host_str().map(|h| h.to_string());
        opts.hostname = host.ok_or(ParseReplicaOptionsError::MissingHost)?;

        // Port
        if let Some(port) = url.port() {
            opts.port = port;
        }

        // Username & password
        let user = url.username();
        if !user.is_empty() {
            opts.username = percent_decode(user);
        }
        if let Some(pw) = url.password() {
            opts.password = Some(percent_decode(pw));
        }

        // Database path (strip leading '/'; empty path = None)
        let path = url.path().trim_start_matches('/');
        if !path.is_empty() {
            opts.database = Some(path.to_string());
        }

        // Query params
        let mut q_binlog_file: Option<String> = None;
        let mut q_binlog_pos: Option<u32> = None;

        for (k, v) in url.query_pairs() {
            let key = k.to_ascii_lowercase();
            let val: Cow<'_, str> = v;

            match key.as_str() {
                "ssl_mode" | "sslmode" => {
                    opts.ssl_mode = parse_ssl_mode(&val)?;
                }
                "server_id" | "serverid" => {
                    opts.server_id = val
                        .parse()
                        .map_err(|e: ParseIntError| ParseReplicaOptionsError::Int("server_id", e))?;
                }
                "blocking" => {
                    opts.blocking = parse_bool("blocking", &val)?;
                }
                "heartbeat" | "heartbeat_interval" => {
                    opts.heartbeat_interval = parse_duration_s_or_ms("heartbeat", &val)?;
                }
                "binlog" => {
                    opts.binlog = parse_binlog_from_query(&val)?;
                }
                "binlog_file" => q_binlog_file = Some(val.to_string()),
                "binlog_pos" => {
                    q_binlog_pos = Some(
                        val.parse()
                            .map_err(|e| ParseReplicaOptionsError::Int("binlog_pos", e))?,
                    );
                }
                // ignore unknown keys for forward-compat
                _ => {}
            }
        }

        // If file/pos pair was provided, use it (overrides plain `binlog` if both given)
        match (q_binlog_file, q_binlog_pos) {
            (Some(f), Some(p)) => {
                opts.binlog = BinlogOptions::from_position(f, p);
            }
            (None, Some(_)) => return Err(ParseReplicaOptionsError::BinlogFileMissing),
            _ => {}
        }

        Ok(opts)
    }
}


fn parse_ssl_mode(s: &str) -> Result<SslMode, ParseReplicaOptionsError> {
    match s.to_ascii_lowercase().as_str() {
        "disabled" | "disable" | "off" => Ok(SslMode::Disabled),
        "preferred" | "prefer" => Ok(SslMode::IfAvailable),
        "required" | "require" | "on" => Ok(SslMode::Require),
        "verify_ca" | "verify-ca" | "verifyca" => Ok(SslMode::RequireVerifyCa),
        "verify_identity" | "verify-identity" | "verifyidentity" => Ok(SslMode::RequireVerifyFull),
        other => Err(ParseReplicaOptionsError::SslMode(other.to_string())),
    }
}

fn parse_bool(name: &'static str, s: &str) -> Result<bool, ParseReplicaOptionsError> {
    match s.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "y" | "on" => Ok(true),
        "0" | "false" | "no" | "n" | "off" => Ok(false),
        _ => Err(ParseReplicaOptionsError::Bool(name, s.to_string())),
    }
}

fn parse_duration_s_or_ms(name: &'static str, s: &str) -> Result<Duration, ParseReplicaOptionsError> {
    let s_trim = s.trim().to_ascii_lowercase();
    if let Some(ns) = s_trim.strip_suffix("ms") {
        let ms: u64 = ns.trim().parse().map_err(|e| ParseReplicaOptionsError::Int(name, e))?;
        return Ok(Duration::from_millis(ms));
    }
    if let Some(ns) = s_trim.strip_suffix('s') {
        let secs: u64 = ns.trim().parse().map_err(|e| ParseReplicaOptionsError::Int(name, e))?;
        return Ok(Duration::from_secs(secs));
    }
    // plain integer = seconds
    let secs: u64 = s_trim.parse().map_err(|e| ParseReplicaOptionsError::Int(name, e))?;
    Ok(Duration::from_secs(secs))
}

fn parse_binlog_from_query(
    value: &str,
) -> Result<BinlogOptions, ParseReplicaOptionsError> {
    let v = value.trim();
    if v.eq_ignore_ascii_case("start") {
        return Ok(BinlogOptions::from_start());
    }
    if v.eq_ignore_ascii_case("end") {
        return Ok(BinlogOptions::from_end());
    }
    // file:<name>:<pos>
    if let Some(rest) = v.strip_prefix("file:") {
        let mut parts = rest.split(':');
        let name = parts
            .next()
            .ok_or_else(|| ParseReplicaOptionsError::Binlog(value.to_string()))?;
        let pos_str = parts
            .next()
            .ok_or_else(|| ParseReplicaOptionsError::Binlog(value.to_string()))?;
        let pos: u32 = pos_str
            .parse()
            .map_err(|e| ParseReplicaOptionsError::Int("binlog_pos", e))?;
        return Ok(BinlogOptions::from_position(name.to_string(), pos));
    }
    Err(ParseReplicaOptionsError::Binlog(value.to_string()))
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().to_string()
}