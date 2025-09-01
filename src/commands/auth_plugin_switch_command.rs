use crate::{constants::auth_plugin_names::AuthPlugin, extensions::encrypt_password};
use std::io::{self, Cursor, Write};

pub struct AuthPluginSwitchCommand {
    pub password: String,
    pub scramble: String,
    #[allow(dead_code)]
    pub auth_plugin_name: String,
    pub auth_plugin: AuthPlugin,
}

impl AuthPluginSwitchCommand {
    pub fn new(
        password: &str,
        scramble: &str,
        auth_plugin_name: &str,
        auth_plugin: AuthPlugin,
    ) -> Self {
        Self {
            password: password.to_owned(),
            scramble: scramble.to_owned(),
            auth_plugin_name: auth_plugin_name.to_owned(),
            auth_plugin,
        }
    }

    pub fn serialize(&self) -> Result<Vec<u8>, io::Error> {
        let mut vec = Vec::new();
        let mut cursor = Cursor::new(&mut vec);

        let encrypted_password =
            encrypt_password(&self.password, &self.scramble, &self.auth_plugin);
        cursor.write_all(&encrypted_password)?;

        Ok(vec)
    }
}
