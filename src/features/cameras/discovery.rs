use std::{
    collections::{HashMap, HashSet},
    io::ErrorKind,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use if_addrs::{IfAddr, get_if_addrs};

const V380_DISCOVERY_PORT: u16 = 10008;
const V380_RESPONSE_PORT: u16 = 10009;
const ONVIF_DISCOVERY_PORT: u16 = 3702;
const V380_PACKET_SIZE: usize = 140;
const V380_PROBE: &[u8] = b"NVDEVSEARCH^100";

const V380_ROUND_TIME: Duration = Duration::from_millis(350);
const V380_FINAL_LISTEN: Duration = Duration::from_millis(2000);
const ONVIF_LISTEN_TIME: Duration = Duration::from_millis(1800);
const TCP_TIMEOUT: Duration = Duration::from_millis(110);
const TCP_WORKERS: usize = 96;
const MAX_INTERFACES: usize = 16;
const MAX_SCAN_HOSTS: usize = 2048;
const TCP_PORTS: [u16; 4] = [8800, 554, 8554, 8899];

#[derive(Clone, Debug)]
pub struct DiscoveredCamera {
    pub address: Ipv4Addr,
    pub onvif_advertised: bool,
    pub v380_advertised: bool,
    pub open_ports: Vec<u16>,
}

impl DiscoveredCamera {
    fn new(address: Ipv4Addr) -> Self {
        Self {
            address,
            onvif_advertised: false,
            v380_advertised: false,
            open_ports: Vec::new(),
        }
    }

    pub fn rtsp_port(&self) -> Option<u16> {
        [554, 8554]
            .into_iter()
            .find(|port| self.open_ports.contains(port))
    }

    pub fn has_port(&self, port: u16) -> bool {
        self.open_ports.contains(&port)
    }

    pub fn service_summary(&self) -> String {
        let mut services = Vec::new();

        if self.onvif_advertised {
            services.push("ONVIF advertised".to_owned());
        } else if self.has_port(8899) {
            services.push("TCP 8899 open (unverified)".to_owned());
        }

        if self.v380_advertised {
            services.push("V380 discovery response".to_owned());
        } else if self.has_port(8800) {
            services.push("TCP 8800 open (V380 candidate)".to_owned());
        }

        for port in [554, 8554] {
            if self.has_port(port) {
                services.push(format!("TCP {port} open (RTSP candidate)"));
            }
        }

        services.join(" · ")
    }
}

#[derive(Clone, Copy)]
struct LocalNetwork {
    local: Ipv4Addr,
    network: u32,
    mask: u32,
    broadcast: Ipv4Addr,
}

impl LocalNetwork {
    fn contains(self, address: Ipv4Addr) -> bool {
        (u32::from(address) & self.mask) == self.network
    }

    fn hosts(self) -> impl Iterator<Item = Ipv4Addr> {
        let scan_mask = self.mask | 0xffff_ff00;
        let start = u32::from(self.local) & scan_mask;
        let end = start | !scan_mask;
        let local = u32::from(self.local);

        (start.saturating_add(1)..end)
            .filter(move |value| *value != local)
            .map(Ipv4Addr::from)
    }
}

pub fn scan(cancelled: Arc<AtomicBool>) -> Result<Vec<DiscoveredCamera>, String> {
    let networks = local_networks()?;

    if networks.is_empty() {
        return Err("No supported private IPv4 LAN interface was found.".into());
    }

    let results = Mutex::new(HashMap::<Ipv4Addr, DiscoveredCamera>::new());

    let hosts = scan_hosts(&networks);

    let v380_result = thread::scope(|scope| {
        let v380 = scope.spawn(|| discover_v380(&networks, &results, &cancelled));

        scope.spawn(|| {
            probe_tcp_hosts(&hosts, &results, &cancelled);
        });

        for &network in &networks {
            let results = &results;
            let cancelled = &cancelled;

            scope.spawn(move || {
                discover_onvif(network, results, cancelled);
            });
        }

        v380.join()
            .unwrap_or_else(|_| Err("V380 discovery worker stopped unexpectedly.".into()))
    });

    if cancelled.load(Ordering::Relaxed) {
        return Err("Scan cancelled.".into());
    }

    let mut devices = results
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .values()
        .cloned()
        .collect::<Vec<_>>();

    devices.sort_by_key(|device| u32::from(device.address));

    for device in &mut devices {
        device.open_ports.sort_unstable();
    }

    if devices.is_empty() {
        v380_result?;
    }

    Ok(devices)
}

fn scan_hosts(networks: &[LocalNetwork]) -> Vec<Ipv4Addr> {
    let mut seen = HashSet::new();
    let mut hosts = Vec::new();

    for network in networks {
        for address in network.hosts() {
            if seen.insert(address) {
                hosts.push(address);
            }

            if hosts.len() >= MAX_SCAN_HOSTS {
                return hosts;
            }
        }
    }

    hosts
}

fn preferred_local_ip() -> Option<Ipv4Addr> {
    let socket = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, 0)).ok()?;

    socket.connect((Ipv4Addr::new(192, 0, 2, 1), 9)).ok()?;

    match socket.local_addr().ok()? {
        SocketAddr::V4(address) => Some(*address.ip()),
        SocketAddr::V6(_) => None,
    }
}

fn local_networks() -> Result<Vec<LocalNetwork>, String> {
    let interfaces = get_if_addrs()
        .map_err(|error| format!("Could not enumerate network interfaces: {error}"))?;

    let mut networks = Vec::new();
    let mut seen = HashSet::new();

    for interface in interfaces {
        let IfAddr::V4(address) = interface.addr else {
            continue;
        };

        if !address.ip.is_private() || address.ip.is_loopback() {
            continue;
        }

        let mask = u32::from(address.netmask);

        if mask >= 0xffff_fffe {
            continue;
        }

        if !seen.insert((address.ip, mask)) {
            continue;
        }

        let network = u32::from(address.ip) & mask;

        networks.push(LocalNetwork {
            local: address.ip,
            network,
            mask,
            broadcast: Ipv4Addr::from(network | !mask),
        });
    }

    let preferred = preferred_local_ip();

    networks.sort_by_key(|network| (Some(network.local) != preferred, u32::from(network.local)));

    networks.truncate(MAX_INTERFACES);

    Ok(networks)
}

fn v380_packet() -> [u8; V380_PACKET_SIZE] {
    let mut packet = [0u8; V380_PACKET_SIZE];

    packet[..V380_PROBE.len()].copy_from_slice(V380_PROBE);

    packet
}

fn discover_v380(
    networks: &[LocalNetwork],
    results: &Mutex<HashMap<Ipv4Addr, DiscoveredCamera>>,
    cancelled: &AtomicBool,
) -> Result<(), String> {
    let receiver = UdpSocket::bind((Ipv4Addr::UNSPECIFIED, V380_RESPONSE_PORT))
        .map_err(|error| format!("Cannot listen for V380 on UDP 10009: {error}"))?;

    receiver
        .set_read_timeout(Some(Duration::from_millis(100)))
        .map_err(|error| format!("Cannot configure V380 receiver: {error}"))?;

    let mut senders = Vec::new();

    for &network in networks {
        let Ok(sender) = UdpSocket::bind((network.local, 0)) else {
            continue;
        };

        if sender.set_broadcast(true).is_ok() {
            senders.push((network, sender));
        }
    }

    if senders.is_empty() {
        return Err("No private LAN adapter could send a V380 broadcast.".into());
    }

    let packet = v380_packet();
    let mut sent_any = false;

    for _ in 0..3 {
        if cancelled.load(Ordering::Relaxed) {
            return Ok(());
        }

        for (network, sender) in &senders {
            for destination in [network.broadcast, Ipv4Addr::BROADCAST] {
                if sender
                    .send_to(&packet, (destination, V380_DISCOVERY_PORT))
                    .is_ok()
                {
                    sent_any = true;
                }
            }
        }

        collect_v380_responses(
            &receiver,
            networks,
            results,
            cancelled,
            Instant::now() + V380_ROUND_TIME,
        );
    }

    if !sent_any {
        return Err("V380 discovery broadcast could not be sent.".into());
    }

    collect_v380_responses(
        &receiver,
        networks,
        results,
        cancelled,
        Instant::now() + V380_FINAL_LISTEN,
    );

    Ok(())
}

fn collect_v380_responses(
    receiver: &UdpSocket,
    networks: &[LocalNetwork],
    results: &Mutex<HashMap<Ipv4Addr, DiscoveredCamera>>,
    cancelled: &AtomicBool,
    deadline: Instant,
) {
    let mut buffer = [0u8; 4096];

    while Instant::now() < deadline && !cancelled.load(Ordering::Relaxed) {
        let (length, sender) = match receiver.recv_from(&mut buffer) {
            Ok(response) => response,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                continue;
            }
            Err(_) => break,
        };

        let SocketAddr::V4(sender) = sender else {
            continue;
        };

        let sender_ip = *sender.ip();

        if !networks
            .iter()
            .any(|network| network.contains(sender_ip) && network.local != sender_ip)
        {
            continue;
        }

        let Some(reported_ip) = parse_v380_response(&buffer[..length], sender_ip) else {
            continue;
        };

        let address = if networks
            .iter()
            .any(|network| network.contains(reported_ip) && network.local != reported_ip)
        {
            reported_ip
        } else {
            sender_ip
        };

        update_result(results, address, |device| {
            device.v380_advertised = true;
        });
    }
}

fn parse_v380_response(bytes: &[u8], sender: Ipv4Addr) -> Option<Ipv4Addr> {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());

    let payload = std::str::from_utf8(&bytes[..end]).ok()?;
    let mut fields = payload.split('^');

    if fields.next()? != "NVDEVRESULT" || fields.next()? != "200" {
        return None;
    }

    let _mac = fields.next()?;

    Some(
        fields
            .next()
            .and_then(|field| field.parse().ok())
            .unwrap_or(sender),
    )
}

fn discover_onvif(
    network: LocalNetwork,
    results: &Mutex<HashMap<Ipv4Addr, DiscoveredCamera>>,
    cancelled: &AtomicBool,
) {
    let Ok(socket) = UdpSocket::bind((network.local, 0)) else {
        return;
    };

    if socket
        .set_read_timeout(Some(Duration::from_millis(100)))
        .is_err()
    {
        return;
    }

    let message_id = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    let probe = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<e:Envelope xmlns:e="http://www.w3.org/2003/05/soap-envelope"
 xmlns:w="http://schemas.xmlsoap.org/ws/2004/08/addressing"
 xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery">
 <e:Header>
  <w:MessageID>uuid:{message_id:032x}</w:MessageID>
  <w:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To>
  <w:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action>
 </e:Header>
 <e:Body><d:Probe/></e:Body>
</e:Envelope>"#
    );

    let _ = socket.send_to(
        probe.as_bytes(),
        (Ipv4Addr::new(239, 255, 255, 250), ONVIF_DISCOVERY_PORT),
    );

    let deadline = Instant::now() + ONVIF_LISTEN_TIME;
    let mut buffer = [0u8; 16 * 1024];

    while Instant::now() < deadline && !cancelled.load(Ordering::Relaxed) {
        let Ok((length, sender)) = socket.recv_from(&mut buffer) else {
            continue;
        };

        let SocketAddr::V4(sender) = sender else {
            continue;
        };

        let address = *sender.ip();

        if network.contains(address)
            && address != network.local
            && is_onvif_camera_response(&buffer[..length])
        {
            update_result(results, address, |device| {
                device.onvif_advertised = true;
            });
        }
    }
}

fn is_onvif_camera_response(bytes: &[u8]) -> bool {
    let Ok(xml) = std::str::from_utf8(bytes) else {
        return false;
    };

    let Ok(document) = roxmltree::Document::parse(xml) else {
        return false;
    };

    document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "ProbeMatch")
        .any(|match_node| {
            match_node
                .descendants()
                .filter(|node| node.is_element() && node.tag_name().name() == "Types")
                .filter_map(|node| node.text())
                .any(|types| {
                    types
                        .split_whitespace()
                        .any(|kind| kind.ends_with("NetworkVideoTransmitter"))
                })
        })
}

fn probe_tcp_hosts(
    hosts: &[Ipv4Addr],
    results: &Mutex<HashMap<Ipv4Addr, DiscoveredCamera>>,
    cancelled: &AtomicBool,
) {
    let next = AtomicUsize::new(0);

    thread::scope(|scope| {
        for _ in 0..hosts.len().min(TCP_WORKERS) {
            let next = &next;

            scope.spawn(move || {
                loop {
                    if cancelled.load(Ordering::Relaxed) {
                        break;
                    }

                    let index = next.fetch_add(1, Ordering::Relaxed);

                    let Some(&address) = hosts.get(index) else {
                        break;
                    };

                    let mut open_ports = Vec::new();

                    for port in TCP_PORTS {
                        if cancelled.load(Ordering::Relaxed) {
                            break;
                        }

                        let endpoint = SocketAddr::V4(SocketAddrV4::new(address, port));

                        if TcpStream::connect_timeout(&endpoint, TCP_TIMEOUT).is_ok() {
                            open_ports.push(port);
                        }
                    }

                    if !open_ports.is_empty() {
                        update_result(results, address, |device| {
                            for port in open_ports {
                                if !device.open_ports.contains(&port) {
                                    device.open_ports.push(port);
                                }
                            }
                        });
                    }
                }
            });
        }
    });
}

fn update_result(
    results: &Mutex<HashMap<Ipv4Addr, DiscoveredCamera>>,
    address: Ipv4Addr,
    update: impl FnOnce(&mut DiscoveredCamera),
) {
    let mut devices = results.lock().unwrap_or_else(|error| error.into_inner());

    let device = devices
        .entry(address)
        .or_insert_with(|| DiscoveredCamera::new(address));

    update(device);
}

pub fn configured_camera_ip(address: &str) -> Option<Ipv4Addr> {
    let rest = address.strip_prefix("rtsp://")?;
    let authority = rest.split(['/', '?', '#']).next()?;
    let host = authority.rsplit('@').next()?;

    host.split(':').next()?.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v380_packet_is_padded() {
        let packet = v380_packet();

        assert_eq!(packet.len(), V380_PACKET_SIZE);
        assert!(packet.starts_with(V380_PROBE));
        assert!(packet[V380_PROBE.len()..].iter().all(|byte| *byte == 0));
    }

    #[test]
    fn v380_response_is_parsed() {
        let sender = Ipv4Addr::new(192, 168, 1, 20);

        let response = b"NVDEVRESULT^200^00:11:22:33:44:55^192.168.1.21^255.255.255.0\0";

        assert_eq!(
            parse_v380_response(response, sender),
            Some(Ipv4Addr::new(192, 168, 1, 21))
        );

        assert_eq!(parse_v380_response(b"NVDEVSEARCH^100", sender), None);
    }

    #[test]
    fn onvif_only_matches_camera_types() {
        assert!(is_onvif_camera_response(
            br#"<Envelope><ProbeMatch><Types>dn:NetworkVideoTransmitter</Types></ProbeMatch></Envelope>"#
        ));

        assert!(!is_onvif_camera_response(
            br#"<Envelope><ProbeMatch><Types>dn:Printer</Types></ProbeMatch></Envelope>"#
        ));
    }

    #[test]
    fn camera_url_extracts_ipv4_host() {
        assert_eq!(
            configured_camera_ip("rtsp://192.168.1.20:8554/camera"),
            Some(Ipv4Addr::new(192, 168, 1, 20))
        );
    }

    #[test]
    fn fallback_includes_a_high_address() {
        let network = LocalNetwork {
            local: Ipv4Addr::new(192, 168, 1, 10),
            network: u32::from(Ipv4Addr::new(192, 168, 1, 0)),
            mask: u32::from(Ipv4Addr::new(255, 255, 255, 0)),
            broadcast: Ipv4Addr::new(192, 168, 1, 255),
        };

        let hosts = scan_hosts(&[network]);

        assert!(hosts.contains(&Ipv4Addr::new(192, 168, 1, 240)));

        assert!(!hosts.contains(&network.local));
    }
}
