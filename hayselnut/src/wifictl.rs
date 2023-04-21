//! TODO: move wifi code into this module

pub mod util {
    use anyhow::Result;

    /// black magic
    /// if this is not present, the call to UdpSocket::bind fails
    pub fn fix_networking() -> Result<()> {
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::esp_vfs_eventfd_register(&esp_idf_sys::esp_vfs_eventfd_config_t {
                max_fds: 5,
                ..Default::default()
            })
        })?;
        Ok(())
    }
}
