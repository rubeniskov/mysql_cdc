#[derive(Debug)]
pub enum DatabaseProvider {
    MariaDB,
    MySQL,
}

impl DatabaseProvider {
    pub fn from(server: &str) -> Self {
        match server.contains("MariaDB") {
            true => DatabaseProvider::MariaDB,
            _ => DatabaseProvider::MySQL,
        }
    }
}
