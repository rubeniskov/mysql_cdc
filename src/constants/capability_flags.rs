/// Server and client capability flags
/// <a href="https://mariadb.com/kb/en/library/connection/#capabilities">See more</a>
#[allow(dead_code)]
pub const LONG_PASSWORD: u64 = 1 << 0;
#[allow(dead_code)]
pub const FOUND_ROWS: u64 = 1 << 1;
pub const LONG_FLAG: u64 = 1 << 2;
pub const CONNECT_WITH_DB: u64 = 1 << 3;
#[allow(dead_code)]
pub const NO_SCHEMA: u64 = 1 << 4;
#[allow(dead_code)]
pub const COMPRESS: u64 = 1 << 5;
#[allow(dead_code)]
pub const ODBC: u64 = 1 << 6;
#[allow(dead_code)]
pub const LOCAL_FILES: u64 = 1 << 7;
#[allow(dead_code)]
pub const IGNORE_SPACE: u64 = 1 << 8;
pub const PROTOCOL_41: u64 = 1 << 9;
#[allow(dead_code)]
pub const INTERACTIVE: u64 = 1 << 10;
pub const SSL: u64 = 1 << 11;
#[allow(dead_code)]
pub const IGNORE_SIGPIPE: u64 = 1 << 12;
#[allow(dead_code)]
pub const TRANSACTIONS: u64 = 1 << 13;
#[allow(dead_code)]
pub const RESERVED: u64 = 1 << 14;
pub const SECURE_CONNECTION: u64 = 1 << 15;
#[allow(dead_code)]
pub const MULTI_STATEMENTS: u64 = 1 << 16;
#[allow(dead_code)]
pub const MULTI_RESULTS: u64 = 1 << 17;
#[allow(dead_code)]
pub const PS_MULTI_RESULTS: u64 = 1 << 18;
pub const PLUGIN_AUTH: u64 = 1 << 19;
