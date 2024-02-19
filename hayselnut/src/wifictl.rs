//! TODO: move wifi code into this module

use embedded_svc::wifi::AccessPointInfo;

use crate::conf;

pub mod util {
    use esp_idf_sys::EspError;

    /// black magic
    /// if this is not present, the call to UdpSocket::bind fails
    pub fn fix_networking() -> Result<(), EspError> {
        esp_idf_sys::esp!(unsafe {
            esp_idf_sys::esp_vfs_eventfd_register(&esp_idf_sys::esp_vfs_eventfd_config_t {
                max_fds: 5,
                ..Default::default()
            })
        })?;
        Ok(())
    }
}

/// find and return all known wifi networks, or ones that have no password,
/// in order of signal strength. known networks are prioritized
/// over ones with no password, and networks with no password can be removed entierly
/// with the `include_open_networks` option
///
/// Returns a list of networks and their passwords (None=needs no password)
pub fn filter_networks(
    networks: Vec<AccessPointInfo>,
    include_open_networks: bool,
) -> Vec<(AccessPointInfo, Option<&'static str>)> {
    // signal strength is measured in dBm (Decibls referenced to a miliwatt)
    // larger value = stronger signal
    // should be in the range of ??? (30dBm = 1W transmission power) to -100(min wifi net received)
    let mut found = networks
        .into_iter()
        .filter_map(|net| {
            conf::WIFI_CFG
                .iter()
                .find(|(ssid, _)| ssid == &net.ssid.as_str())
                .map(|(_, pass)| (net.clone(), Some(*pass)))
                .or(if net.auth_method.is_none() && include_open_networks {
                    Some((net, None))
                } else {
                    None
                })
        })
        .collect::<Vec<_>>();
    found.sort_by(|a, b| {
        use std::cmp::Ordering::{Equal, Greater, Less};
        match (a.1, b.1) {
            (Some(..), None) => Greater,
            (None, Some(..)) => Less,
            (..) => Equal,
        }
        .then(a.0.signal_strength.cmp(&b.0.signal_strength))
    });
    found
}

#[derive(Debug, Clone)]
pub enum WifiStatusUpdate {
    Disconnected,
}
