//! TODO: move wifi code into this module

// use std::{net::Ipv4Addr, time::Duration};

// use embedded_svc::wifi::{self, AccessPointInfo, AuthMethod, Wifi as _};
use embedded_svc::wifi::{AccessPointInfo, AuthMethod};
// use esp_idf_svc::{
//     eventloop::EspSystemEventLoop,
//     netif::{EspNetif, EspNetifWait},
//     wifi::{EspWifi, WifiWait},
// };
// use esp_idf_sys::EspError;

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

// pub struct Wifi<'a> {
//     inner: EspWifi<'a>,
//     sysloop: EspSystemEventLoop,
// }
//
// impl<'a> Wifi<'a> {
//     pub fn new(wifi: EspWifi<'a>, sysloop: EspSystemEventLoop) -> Self {
//         Self {
//             inner: wifi,
//             sysloop,
//         }
//     }
//
//     pub fn start(&mut self) -> Result<(), WifiError> {
//         self.inner.set_configuration(&wifi::Configuration::Client(
//             wifi::ClientConfiguration::default(),
//         ))?;
//         self.inner.start()?;
//         if !WifiWait::new(&self.sysloop)?
//             .wait_with_timeout(Duration::from_secs(20), || self.inner.is_started().unwrap())
//         {
//             // writeln!(display, "Wifi failed to start")?;
//             Err(WifiError::TimedOut)?
//         }
//         Ok(())
//     }
//
//     pub fn scan(&mut self) -> Result<Option<(AccessPointInfo, Option<&'static str>)>, EspError> {
//         let access_points = self.inner.scan()?;
//         let mut useable = filter_networks(access_points, conf::INCLUDE_OPEN_NETWORKS);
//         Ok((!useable.is_empty()).then(|| useable.remove(0)))
//     }
//
//     pub fn connect(
//         &mut self,
//         to: (AccessPointInfo, Option<&'static str>),
//     ) -> Result<(), WifiError> {
//         self.inner
//             .set_configuration(&wifi::Configuration::Client(wifi::ClientConfiguration {
//                 ssid: to.0.ssid.clone(),
//                 password: to.1.unwrap_or_default().into(),
//                 channel: Some(to.0.channel),
//                 ..Default::default()
//             }))?;
//
//         self.inner.connect()?;
//         if !EspNetifWait::new::<EspNetif>(self.inner.sta_netif(), &self.sysloop)?.wait_with_timeout(
//             Duration::from_secs(20),
//             || {
//                 self.inner.is_connected().unwrap()
//                     && self.inner.sta_netif().get_ip_info().unwrap().ip != Ipv4Addr::new(0, 0, 0, 0)
//             },
//         ) {
//             error!("Wifi did not connect or receive a DHCP lease");
//             Err(WifiError::TimedOut)?
//         }
//         Ok(())
//     }
//
//     pub fn inner(&mut self) -> &mut EspWifi<'a> {
//         &mut self.inner
//     }
// }
//
// #[derive(Debug, thiserror::Error)]
// pub enum WifiError {
//     #[error("ESP-IDF error: {0}")]
//     Esp(#[from] EspError),
//     #[error("Failed to start within the given time limit")]
//     TimedOut,
// }

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
                .or(
                    if net.auth_method == AuthMethod::None && include_open_networks {
                        Some((net, None))
                    } else {
                        None
                    },
                )
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
