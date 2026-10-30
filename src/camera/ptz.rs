use std::{
    collections::HashSet,
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream, UdpSocket},
    sync::{
        Arc,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{SecondsFormat, Utc};
use if_addrs::{IfAddr, get_if_addrs};
use md5::{Digest as _, Md5};
use roxmltree::Document;
use sha1::Sha1;
use sha2::Sha256;
use zeroize::Zeroize;

use super::CameraId;

const IO_TIMEOUT: Duration = Duration::from_millis(1200);
const DISCOVERY_TIME: Duration = Duration::from_millis(1200);
const MAX_REPLY: usize = 128 * 1024;
const MAX_DISCOVERY_REPLY: usize = 16 * 1024;
const MAX_DISCOVERY_INTERFACES: usize = 8;
const MOVE_REFRESH: Duration = Duration::from_millis(250);
const KEEPALIVE_DEADLINE: Duration = Duration::from_millis(450);
const CAMERA_MOVE_TIMEOUT: &str = "PT0.8S";
const ONVIF_MULTICAST: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

impl Direction {
    fn velocity(self) -> (&'static str, &'static str) {
        match self {
            Self::Up => ("0", "0.35"),
            Self::Down => ("0", "-0.35"),
            Self::Left => ("-0.35", "0"),
            Self::Right => ("0.35", "0"),
        }
    }
}

pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    pub fn new(username: String, password: String) -> Result<Self, String> {
        if username.is_empty()
            || username.len() > 128
            || password.is_empty()
            || password.len() > 512
        {
            return Err("Enter an ONVIF username and password".into());
        }
        if username.chars().any(char::is_control) || password.chars().any(char::is_control) {
            return Err("ONVIF credentials contain unsupported control characters".into());
        }
        Ok(Self { username, password })
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("Credentials([redacted])")
    }
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
    }
}

#[derive(Clone, Debug)]
pub struct Target {
    endpoint: Endpoint,
    profile: String,
    credentials: Option<Arc<Credentials>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Endpoint {
    ip: Ipv4Addr,
    port: u16,
    path: String,
}

#[derive(Debug)]
pub enum Event {
    Discovered(CameraId, String, Result<Target, String>),
    CommandFailed(CameraId, String),
}

pub enum Command {
    Discover(CameraId, String, Option<Arc<Credentials>>),
    Start(CameraId, Target, Direction),
    KeepAlive(CameraId),
    Stop(CameraId),
    Shutdown,
}

struct Motion {
    id: CameraId,
    target: Target,
    direction: Direction,
    refreshed: Instant,
    heartbeat: Instant,
}

pub fn spawn_worker() -> (Sender<Command>, Receiver<Event>) {
    let (commands_tx, commands_rx) = mpsc::channel();
    let (events_tx, events_rx) = mpsc::channel();
    thread::spawn(move || worker(commands_rx, events_tx));
    (commands_tx, events_rx)
}

fn worker(rx: Receiver<Command>, events: Sender<Event>) {
    let mut active: Option<Motion> = None;
    loop {
        match rx.recv_timeout(Duration::from_millis(70)) {
            Ok(Command::Discover(id, url, credentials)) => {
                stop_motion(&mut active);
                let result = discover(&url, credentials);
                let _ = events.send(Event::Discovered(id, url, result));
            }
            Ok(Command::Start(id, target, direction)) => {
                stop_motion(&mut active);
                match move_camera(&target, direction) {
                    Ok(()) => {
                        let now = Instant::now();
                        active = Some(Motion {
                            id,
                            target,
                            direction,
                            refreshed: now,
                            heartbeat: now,
                        });
                    }
                    Err(error) => {
                        let _ = events.send(Event::CommandFailed(id, error));
                    }
                }
            }
            Ok(Command::KeepAlive(id)) => {
                if let Some(motion) = active.as_mut().filter(|motion| motion.id == id) {
                    motion.heartbeat = Instant::now();
                }
            }
            Ok(Command::Stop(id)) => {
                if active.as_ref().is_some_and(|motion| motion.id == id) {
                    stop_motion(&mut active);
                }
            }
            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                stop_motion(&mut active);
                break;
            }
            Err(RecvTimeoutError::Timeout) => {}
        }
        if let Some(motion) = active.as_mut() {
            let now = Instant::now();
            if now.duration_since(motion.heartbeat) >= KEEPALIVE_DEADLINE {
                stop_motion(&mut active);
            } else if now.duration_since(motion.refreshed) >= MOVE_REFRESH {
                let result = move_camera(&motion.target, motion.direction);
                motion.refreshed = Instant::now();
                if let Err(error) = result {
                    let id = motion.id;
                    stop_motion(&mut active);
                    let _ = events.send(Event::CommandFailed(id, error));
                }
            }
        }
    }
}

fn stop_motion(active: &mut Option<Motion>) {
    if let Some(motion) = active.take() {
        let _ = stop(&motion.target);
    }
}

pub fn discover(rtsp_url: &str, credentials: Option<Arc<Credentials>>) -> Result<Target, String> {
    let ip = camera_ip(rtsp_url)?;
    let mut candidates = discover_endpoints(ip);
    for port in [8899, 80, 8000, 8080] {
        let fallback = Endpoint {
            ip,
            port,
            path: "/onvif/device_service".into(),
        };
        if !candidates.contains(&fallback) {
            candidates.push(fallback);
        }
    }

    let mut last_error = "No ONVIF device service responded on this camera".to_owned();
    let mut authentication_error: Option<String> = None;
    for endpoint in &candidates {
        match probe_device(endpoint, credentials.as_ref()) {
            Ok(mut target) => {
                target.credentials = credentials;
                return Ok(target);
            }
            Err(error) => {
                if error.contains("authentication") || error.contains("credentials") {
                    authentication_error = Some(error);
                } else {
                    last_error = error;
                }
            }
        }
    }
    Err(authentication_error.unwrap_or(last_error))
}

fn camera_ip(rtsp_url: &str) -> Result<Ipv4Addr, String> {
    let host = rtsp_url
        .strip_prefix("rtsp://")
        .ok_or("Not an RTSP camera")?
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();
    let host = host.split(':').next().unwrap_or_default();
    let ip: Ipv4Addr = host
        .parse()
        .map_err(|_| "PTZ discovery requires an IPv4 RTSP address".to_owned())?;
    if !(ip.is_private() || ip.is_link_local())
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_loopback()
    {
        return Err("PTZ discovery is restricted to private local camera addresses".into());
    }
    Ok(ip)
}

fn discover_endpoints(camera: Ipv4Addr) -> Vec<Endpoint> {
    let Ok(interfaces) = get_if_addrs() else {
        return Vec::new();
    };
    let mut local = HashSet::new();
    for interface in interfaces {
        let IfAddr::V4(address) = interface.addr else {
            continue;
        };
        if address.ip != camera
            && (u32::from(address.ip) & u32::from(address.netmask))
                == (u32::from(camera) & u32::from(address.netmask))
        {
            local.insert(address.ip);
        }
    }

    let mut results = Vec::new();
    let mut seen = HashSet::new();
    for interface in local.into_iter().take(MAX_DISCOVERY_INTERFACES) {
        let Ok(socket) = UdpSocket::bind((interface, 0)) else {
            continue;
        };
        if socket
            .set_read_timeout(Some(Duration::from_millis(120)))
            .is_err()
            || socket.set_multicast_ttl_v4(1).is_err()
        {
            continue;
        }

        let mut nonce = [0u8; 16];
        rand::fill(&mut nonce);
        let message_id = nonce
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let probe = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <e:Envelope xmlns:e=\"http://www.w3.org/2003/05/soap-envelope\" \
             xmlns:w=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\" \
             xmlns:d=\"http://schemas.xmlsoap.org/ws/2005/04/discovery\" \
             xmlns:dn=\"http://www.onvif.org/ver10/network/wsdl\">\
             <e:Header><w:MessageID>uuid:{message_id}</w:MessageID>\
             <w:To>urn:schemas-xmlsoap-org:ws:2005:04:discovery</w:To>\
             <w:Action>http://schemas.xmlsoap.org/ws/2005/04/discovery/Probe</w:Action>\
             </e:Header><e:Body><d:Probe><d:Types>dn:NetworkVideoTransmitter</d:Types>\
             </d:Probe></e:Body></e:Envelope>"
        );
        if socket
            .send_to(probe.as_bytes(), (ONVIF_MULTICAST, 3702))
            .is_err()
        {
            continue;
        }
        let deadline = Instant::now() + DISCOVERY_TIME;
        let mut buffer = [0u8; MAX_DISCOVERY_REPLY];
        while Instant::now() < deadline {
            let Ok((len, sender)) = socket.recv_from(&mut buffer) else {
                continue;
            };
            let SocketAddr::V4(sender) = sender else {
                continue;
            };
            if *sender.ip() != camera {
                continue;
            }
            for endpoint in parse_probe_matches(camera, &buffer[..len]) {
                if seen.insert(endpoint.clone()) {
                    results.push(endpoint);
                }
            }
        }
    }
    results
}

fn parse_probe_matches(ip: Ipv4Addr, bytes: &[u8]) -> Vec<Endpoint> {
    let Ok(xml) = std::str::from_utf8(bytes) else {
        return Vec::new();
    };
    let Ok(document) = Document::parse(xml) else {
        return Vec::new();
    };
    let mut endpoints = Vec::new();
    for response in document
        .descendants()
        .filter(|node| node.is_element() && node.tag_name().name() == "ProbeMatch")
    {
        let is_camera = response
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "Types")
            .filter_map(|node| node.text())
            .any(|types| {
                types
                    .split_whitespace()
                    .any(|item| item.ends_with("NetworkVideoTransmitter"))
            });
        if !is_camera {
            continue;
        }
        for value in response
            .descendants()
            .filter(|node| node.is_element() && node.tag_name().name() == "XAddrs")
            .filter_map(|node| node.text())
            .flat_map(str::split_whitespace)
        {
            if let Ok(endpoint) = Endpoint::same_camera_url(ip, value) {
                endpoints.push(endpoint);
            }
        }
    }
    endpoints
}

fn probe_device(
    device: &Endpoint,
    credentials: Option<&Arc<Credentials>>,
) -> Result<Target, String> {
    let reply = request(
        device,
        "http://www.onvif.org/ver10/device/wsdl/GetCapabilities",
        "<tds:GetCapabilities><tds:Category>All</tds:Category></tds:GetCapabilities>",
        credentials,
    )?;
    let doc = parse_xml(&reply)?;
    let services = doc
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Capabilities")
        .ok_or("ONVIF response has no capabilities")?;
    let ptz_url = services
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "PTZ")
        .and_then(|node| child_text(node, "XAddr"))
        .ok_or("This ONVIF device does not advertise PTZ")?;
    let media_url = services
        .children()
        .find(|node| node.is_element() && node.tag_name().name() == "Media")
        .and_then(|node| child_text(node, "XAddr"))
        .ok_or("This ONVIF device does not advertise Media")?;
    let ptz_endpoint = Endpoint::same_camera_url(device.ip, &ptz_url)?;
    let media_endpoint = Endpoint::same_camera_url(device.ip, &media_url)?;

    let reply = request(
        &media_endpoint,
        "http://www.onvif.org/ver10/media/wsdl/GetProfiles",
        "<trt:GetProfiles/>",
        credentials,
    )?;
    let doc = parse_xml(&reply)?;
    let profile = doc
        .descendants()
        .find(|node| {
            node.is_element()
                && node.tag_name().name() == "Profiles"
                && node.children().any(|child| {
                    child.is_element() && child.tag_name().name() == "PTZConfiguration"
                })
        })
        .ok_or("No ONVIF media profile has PTZ configuration")?;
    let token = escape_token(
        profile
            .attribute("token")
            .ok_or("ONVIF PTZ profile has no token")?,
    )?;

    let body =
        format!("<tptz:GetStatus><tptz:ProfileToken>{token}</tptz:ProfileToken></tptz:GetStatus>");
    let reply = request(
        &ptz_endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/GetStatus",
        &body,
        credentials,
    )?;
    let doc = parse_xml(&reply)?;
    if !doc
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "GetStatusResponse")
    {
        return Err("ONVIF PTZ service did not return GetStatusResponse".into());
    }
    Ok(Target {
        endpoint: ptz_endpoint,
        profile: token,
        credentials: None,
    })
}

fn move_camera(target: &Target, direction: Direction) -> Result<(), String> {
    let (x, y) = direction.velocity();
    let body = format!(
        "<tptz:ContinuousMove><tptz:ProfileToken>{}</tptz:ProfileToken>\
         <tptz:Velocity><tt:PanTilt x=\"{x}\" y=\"{y}\"/></tptz:Velocity>\
         <tptz:Timeout>{CAMERA_MOVE_TIMEOUT}</tptz:Timeout></tptz:ContinuousMove>",
        target.profile
    );
    request(
        &target.endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/ContinuousMove",
        &body,
        target.credentials.as_ref(),
    )
    .map(|_| ())
}

fn stop(target: &Target) -> Result<(), String> {
    let body = format!(
        "<tptz:Stop><tptz:ProfileToken>{}</tptz:ProfileToken>\
         <tptz:PanTilt>true</tptz:PanTilt><tptz:Zoom>true</tptz:Zoom></tptz:Stop>",
        target.profile
    );
    request(
        &target.endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/Stop",
        &body,
        target.credentials.as_ref(),
    )
    .map(|_| ())
}

impl Endpoint {
    fn same_camera_url(expected_ip: Ipv4Addr, url: &str) -> Result<Self, String> {
        let value = url
            .strip_prefix("http://")
            .ok_or("ONVIF advertised an unsupported non-HTTP service URL")?;
        let (authority, path) = value.split_once('/').ok_or("Invalid ONVIF XAddr")?;
        if authority.contains('@') {
            return Err("ONVIF XAddr contains credentials".into());
        }
        let (ip, port) = if let Some((host, port)) = authority.rsplit_once(':') {
            (
                host.parse::<Ipv4Addr>()
                    .map_err(|_| "Invalid ONVIF XAddr host")?,
                port.parse::<u16>()
                    .map_err(|_| "Invalid ONVIF XAddr port")?,
            )
        } else {
            (
                authority
                    .parse::<Ipv4Addr>()
                    .map_err(|_| "Invalid ONVIF XAddr host")?,
                80,
            )
        };
        if ip != expected_ip
            || port == 0
            || path.is_empty()
            || path.len() > 1024
            || !path
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'#' && byte != b'?')
        {
            return Err("ONVIF XAddr must use the configured camera IP".into());
        }
        Ok(Self {
            ip,
            port,
            path: format!("/{path}"),
        })
    }
}

fn escape_token(value: &str) -> Result<String, String> {
    if value.is_empty()
        || value.len() > 128
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | ':' | '@'))
    {
        return Err("Invalid ONVIF profile token".into());
    }
    Ok(value.to_owned())
}

fn child_text(node: roxmltree::Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|child| child.is_element() && child.tag_name().name() == name)
        .and_then(|child| child.text())
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn parse_xml(xml: &str) -> Result<Document<'_>, String> {
    let doc = Document::parse(xml).map_err(|_| "Invalid ONVIF XML response")?;
    if let Some(fault) = doc
        .descendants()
        .find(|node| node.is_element() && node.tag_name().name() == "Fault")
    {
        let is_auth = fault
            .descendants()
            .filter_map(|node| node.text())
            .any(|text| {
                let normalized = text.to_ascii_lowercase();
                normalized.contains("notauthorized")
                    || normalized.contains("unauthorized")
                    || normalized.contains("invalidsecurity")
                    || normalized.contains("failedauthentication")
            });
        return Err(if is_auth {
            "ONVIF authentication required or credentials rejected"
        } else {
            "Camera returned an ONVIF SOAP fault"
        }
        .into());
    }
    Ok(doc)
}

fn ws_security_header(credentials: Option<&Arc<Credentials>>) -> String {
    let Some(credentials) = credentials else {
        return String::new();
    };
    let mut nonce = [0u8; 24];
    rand::fill(&mut nonce);
    let created = Utc::now().to_rfc3339_opts(SecondsFormat::Secs, true);
    let mut digest = Sha1::new();
    digest.update(nonce);
    digest.update(created.as_bytes());
    digest.update(credentials.password.as_bytes());
    let token = STANDARD.encode(digest.finalize());
    let nonce_encoded = STANDARD.encode(nonce);
    let username = xml_escape(&credentials.username);
    format!(
        "<s:Header><wsse:Security s:mustUnderstand=\"1\" \
         xmlns:wsse=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd\" \
         xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\">\
         <wsse:UsernameToken><wsse:Username>{username}</wsse:Username>\
         <wsse:Password Type=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest\">{token}</wsse:Password>\
         <wsse:Nonce EncodingType=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary\">{nonce_encoded}</wsse:Nonce>\
         <wsu:Created>{created}</wsu:Created></wsse:UsernameToken></wsse:Security></s:Header>"
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn request(
    endpoint: &Endpoint,
    action: &str,
    body: &str,
    credentials: Option<&Arc<Credentials>>,
) -> Result<String, String> {
    let envelope = format!(
        "<?xml version=\"1.0\" encoding=\"utf-8\"?>\
         <s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" \
         xmlns:tds=\"http://www.onvif.org/ver10/device/wsdl\" \
         xmlns:trt=\"http://www.onvif.org/ver10/media/wsdl\" \
         xmlns:tptz=\"http://www.onvif.org/ver20/ptz/wsdl\" \
         xmlns:tt=\"http://www.onvif.org/ver10/schema\">\
         {}<s:Body>{body}</s:Body></s:Envelope>",
        ws_security_header(credentials)
    );
    let first = http_post(endpoint, action, &envelope, None)?;
    let reply = if first.code == 401 {
        let credentials = credentials.ok_or("ONVIF authentication required — use ONVIF sign-in")?;
        let challenge = first
            .headers
            .iter()
            .find(|(key, value)| {
                key.eq_ignore_ascii_case("www-authenticate")
                    && value.to_ascii_lowercase().starts_with("digest ")
            })
            .map(|(_, value)| value.as_str())
            .ok_or("ONVIF rejected credentials or requires unsupported HTTP authentication")?;
        let authorization = digest_authorization(challenge, endpoint, credentials)?;
        http_post(endpoint, action, &envelope, Some(&authorization))?
    } else {
        first
    };
    if reply.code == 401 || reply.code == 403 {
        return Err("ONVIF authentication required or credentials rejected".into());
    }
    if reply.code != 200 {
        return Err(format!("ONVIF service returned HTTP {}", reply.code));
    }
    let xml = String::from_utf8(reply.body).map_err(|_| "Invalid ONVIF XML encoding")?;
    parse_xml(&xml)?;
    Ok(xml)
}

struct HttpReply {
    code: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

fn http_post(
    endpoint: &Endpoint,
    action: &str,
    xml: &str,
    auth: Option<&str>,
) -> Result<HttpReply, String> {
    let address = SocketAddrV4::new(endpoint.ip, endpoint.port).into();
    let mut stream = TcpStream::connect_timeout(&address, IO_TIMEOUT)
        .map_err(|_| "ONVIF service is not reachable")?;
    stream
        .set_read_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "ONVIF socket error")?;
    stream
        .set_write_timeout(Some(IO_TIMEOUT))
        .map_err(|_| "ONVIF socket error")?;
    let authorization = auth.map_or(String::new(), |header| {
        format!("Authorization: {header}\r\n")
    });
    let headers = format!(
        "POST {} HTTP/1.1\r\nHost: {}:{}\r\n\
         Content-Type: application/soap+xml; charset=utf-8; action=\"{action}\"\r\n\
         Content-Length: {}\r\n{}Connection: close\r\n\r\n",
        endpoint.path,
        endpoint.ip,
        endpoint.port,
        xml.len(),
        authorization
    );
    stream
        .write_all(headers.as_bytes())
        .and_then(|()| stream.write_all(xml.as_bytes()))
        .map_err(|_| "Could not send ONVIF request")?;
    let mut raw = Vec::with_capacity(4096);
    let mut block = [0u8; 4096];
    loop {
        match stream.read(&mut block) {
            Ok(0) => break,
            Ok(length) => {
                if raw.len() + length > MAX_REPLY {
                    return Err("ONVIF response exceeds size limit".into());
                }
                raw.extend_from_slice(&block[..length]);
                if let Some(cut) = raw.windows(4).position(|bytes| bytes == b"\r\n\r\n")
                    && let Ok(headers) = std::str::from_utf8(&raw[..cut])
                    && let Some(length) = headers.lines().find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then_some(value.trim())
                            .and_then(|value| value.parse::<usize>().ok())
                    })
                {
                    if length > MAX_REPLY {
                        return Err("ONVIF response exceeds size limit".into());
                    }
                    if raw.len() >= cut + 4 + length {
                        break;
                    }
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                if raw.windows(4).any(|part| part == b"\r\n\r\n") {
                    break;
                }
                return Err("ONVIF service timed out".into());
            }
            Err(_) => return Err("Could not read ONVIF response".into()),
        }
    }
    parse_http_reply(&raw)
}

fn parse_http_reply(raw: &[u8]) -> Result<HttpReply, String> {
    let cut = raw
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .ok_or("Invalid ONVIF HTTP response")?;
    let header_text = std::str::from_utf8(&raw[..cut]).map_err(|_| "Invalid ONVIF HTTP headers")?;
    let code = header_text
        .lines()
        .next()
        .filter(|line| line.starts_with("HTTP/"))
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or("Invalid ONVIF HTTP status")?;
    let headers: Vec<(String, String)> = header_text
        .lines()
        .skip(1)
        .filter_map(|line| {
            line.split_once(':')
                .map(|(name, value)| (name.to_owned(), value.trim().to_owned()))
        })
        .collect();
    let chunked = headers.iter().any(|(key, value)| {
        key.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
    });
    let body = if chunked {
        decode_chunked(&raw[cut + 4..])?
    } else {
        let length = headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse::<usize>().ok());
        let bytes = &raw[cut + 4..];
        if let Some(length) = length {
            if bytes.len() < length {
                return Err("Truncated ONVIF HTTP response".into());
            }
            bytes[..length].to_vec()
        } else {
            bytes.to_vec()
        }
    };
    Ok(HttpReply {
        code,
        headers,
        body,
    })
}

fn decode_chunked(input: &[u8]) -> Result<Vec<u8>, String> {
    let mut position = 0usize;
    let mut output = Vec::new();
    loop {
        let end = input[position..]
            .windows(2)
            .position(|bytes| bytes == b"\r\n")
            .map(|offset| position + offset)
            .ok_or("Incomplete ONVIF chunked response")?;
        let line =
            std::str::from_utf8(&input[position..end]).map_err(|_| "Invalid HTTP chunk length")?;
        let count = usize::from_str_radix(line.split(';').next().unwrap_or(""), 16)
            .map_err(|_| "Invalid HTTP chunk length")?;
        position = end + 2;
        if count == 0 {
            break;
        }
        if count > MAX_REPLY.saturating_sub(output.len())
            || position + count + 2 > input.len()
            || &input[position + count..position + count + 2] != b"\r\n"
        {
            return Err("Incomplete or oversized ONVIF HTTP chunk".into());
        }
        output.extend_from_slice(&input[position..position + count]);
        position += count + 2;
    }
    Ok(output)
}

fn digest_authorization(
    challenge: &str,
    endpoint: &Endpoint,
    credentials: &Credentials,
) -> Result<String, String> {
    let fields = parse_auth_params(
        challenge
            .strip_prefix("Digest ")
            .or_else(|| challenge.strip_prefix("digest "))
            .ok_or("Unsupported HTTP authentication challenge")?,
    );
    let realm = auth_field(&fields, "realm").ok_or("Missing HTTP Digest realm")?;
    let nonce = auth_field(&fields, "nonce").ok_or("Missing HTTP Digest nonce")?;
    let algorithm = auth_field(&fields, "algorithm").unwrap_or("MD5");
    if !matches!(algorithm.to_ascii_uppercase().as_str(), "MD5" | "SHA-256") {
        return Err("Unsupported ONVIF HTTP Digest algorithm".into());
    }
    let qop = auth_field(&fields, "qop").unwrap_or("");
    if !qop.is_empty()
        && !qop
            .split(',')
            .any(|item| item.trim().eq_ignore_ascii_case("auth"))
    {
        return Err("Unsupported ONVIF HTTP Digest quality of protection".into());
    }
    let mut random = [0u8; 12];
    rand::fill(&mut random);
    let cnonce = random
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let ha1 = digest_hash(
        algorithm,
        &format!("{}:{realm}:{}", credentials.username, credentials.password),
    );
    let ha2 = digest_hash(algorithm, &format!("POST:{}", endpoint.path));
    let response = if qop.is_empty() {
        digest_hash(algorithm, &format!("{ha1}:{nonce}:{ha2}"))
    } else {
        digest_hash(
            algorithm,
            &format!("{ha1}:{nonce}:00000001:{cnonce}:auth:{ha2}"),
        )
    };
    let mut authorization = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{response}\", algorithm={algorithm}",
        quote_header(&credentials.username),
        quote_header(realm),
        quote_header(nonce),
        quote_header(&endpoint.path)
    );
    if !qop.is_empty() {
        authorization.push_str(&format!(", qop=auth, nc=00000001, cnonce=\"{cnonce}\""));
    }
    if let Some(opaque) = auth_field(&fields, "opaque") {
        authorization.push_str(&format!(", opaque=\"{}\"", quote_header(opaque)));
    }
    Ok(authorization)
}

fn digest_hash(algorithm: &str, input: &str) -> String {
    let bytes = if algorithm.eq_ignore_ascii_case("SHA-256") {
        Sha256::digest(input.as_bytes()).to_vec()
    } else {
        Md5::digest(input.as_bytes()).to_vec()
    };
    bytes
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>()
}

fn quote_header(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

fn auth_field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

fn parse_auth_params(text: &str) -> Vec<(String, String)> {
    let mut parts = Vec::new();
    let mut position = 0;
    let bytes = text.as_bytes();
    while position < bytes.len() {
        while position < bytes.len() && matches!(bytes[position], b' ' | b',' | b'\t') {
            position += 1;
        }
        let start = position;
        while position < bytes.len() && bytes[position] != b'=' && bytes[position] != b',' {
            position += 1;
        }
        if position >= bytes.len() || bytes[position] != b'=' {
            break;
        }
        let key = text[start..position].trim().to_string();
        position += 1;
        let value = if position < bytes.len() && bytes[position] == b'"' {
            position += 1;
            let mut value = String::new();
            while position < bytes.len() {
                let ch = bytes[position];
                position += 1;
                if ch == b'"' {
                    break;
                }
                if ch == b'\\' && position < bytes.len() {
                    value.push(bytes[position] as char);
                    position += 1;
                } else {
                    value.push(ch as char);
                }
            }
            value
        } else {
            let start = position;
            while position < bytes.len() && bytes[position] != b',' {
                position += 1;
            }
            text[start..position].trim().to_string()
        };
        parts.push((key, value));
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_private_rtsp_hosts() {
        assert!(camera_ip("rtsp://8.8.8.8/live").is_err());
        assert!(camera_ip("rtsp://example.com/live").is_err());
        assert!(camera_ip("rtsp://192.168.1.20/live").is_ok());
    }

    #[test]
    fn xaddr_cannot_retarget_to_another_camera() {
        let ip = Ipv4Addr::new(192, 168, 1, 20);
        assert!(Endpoint::same_camera_url(ip, "http://192.168.1.21/onvif/device_service").is_err());
        assert!(Endpoint::same_camera_url(ip, "http://192.168.1.20:9000/onvif/special").is_ok());
        assert!(
            Endpoint::same_camera_url(ip, "https://192.168.1.20/onvif/device_service").is_err()
        );
    }

    #[test]
    fn discover_accepts_actual_xaddr() {
        let ip = Ipv4Addr::new(192, 168, 1, 20);
        let response = br#"<s:Envelope xmlns:s="http://www.w3.org/2003/05/soap-envelope"><s:Body><d:ProbeMatches xmlns:d="http://schemas.xmlsoap.org/ws/2005/04/discovery"><d:ProbeMatch><d:Types>dn:NetworkVideoTransmitter</d:Types><d:XAddrs>http://192.168.1.20:9090/custom/onvif http://192.168.1.21/nope</d:XAddrs></d:ProbeMatch></d:ProbeMatches></s:Body></s:Envelope>"#;
        assert_eq!(
            parse_probe_matches(ip, response),
            vec![Endpoint {
                ip,
                port: 9090,
                path: "/custom/onvif".into()
            }]
        );
    }

    #[test]
    fn digest_header_uses_server_challenge() {
        let challenge = "Digest realm=\"cam\", nonce=\"xyz\", qop=\"auth,auth-int\", algorithm=MD5";
        let credentials = Credentials::new("operator".into(), "secret".into()).unwrap();
        let endpoint = Endpoint {
            ip: Ipv4Addr::LOCALHOST,
            port: 80,
            path: "/onvif".into(),
        };
        let header = digest_authorization(challenge, &endpoint, &credentials).unwrap();
        assert!(header.contains("qop=auth"));
        assert!(header.contains("username=\"operator\""));
        assert!(!header.contains("secret"));
    }

    #[test]
    fn http_parser_supports_chunked_responses() {
        let response =
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\ntest\r\n0\r\n\r\n";
        assert_eq!(parse_http_reply(response).unwrap().body, b"test");
    }
}
