use std::{
    collections::{HashMap, HashSet},
    io::Read,
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, UdpSocket},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, RecvTimeoutError, Sender},
    },
    thread,
    time::{Duration, Instant},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use chrono::{SecondsFormat, Utc};
use if_addrs::{IfAddr, get_if_addrs};
use md5::{Digest as _, Md5};
use roxmltree::{Document, Node};
use sha1::Sha1;
use sha2::Sha256;
use zeroize::Zeroize;

use super::CameraId;

const DISCOVERY_TIME: Duration = Duration::from_millis(1200);
const DISCOVERY_IO: Duration = Duration::from_millis(120);
const CONTROL_IO: Duration = Duration::from_millis(400);
const PROBE_IO: Duration = Duration::from_millis(1500);
const MAX_REPLY: usize = 128 * 1024;
const MAX_DISCOVERY_REPLY: usize = 16 * 1024;
const MAX_DISCOVERY_INTERFACES: usize = 8;
const MOVE_REFRESH: Duration = Duration::from_millis(250);
const KEEPALIVE_DEADLINE: Duration = Duration::from_millis(450);
const DESIRED_CAMERA_TIMEOUT: Duration = Duration::from_millis(800);
const MAX_SAFE_CAMERA_TIMEOUT: Duration = Duration::from_secs(1);
const ONVIF_MULTICAST: Ipv4Addr = Ipv4Addr::new(239, 255, 255, 250);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Direction {
    Up,
    Down,
    Left,
    Right,
}

pub struct Credentials {
    username: String,
    password: String,
}

impl Credentials {
    pub fn new(mut username: String, mut password: String) -> Result<Self, String> {
        let valid = !username.is_empty()
            && username.len() <= 128
            && !password.is_empty()
            && password.len() <= 512
            && !username.chars().any(char::is_control)
            && !password.chars().any(char::is_control);

        if !valid {
            username.zeroize();
            password.zeroize();
            return Err("Enter valid ONVIF credentials (username up to 128 bytes)".into());
        }

        Ok(Self { username, password })
    }
}

impl std::fmt::Debug for Credentials {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Credentials([redacted])")
    }
}

impl Drop for Credentials {
    fn drop(&mut self) {
        self.username.zeroize();
        self.password.zeroize();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Scheme {
    Http,
    Https,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct Endpoint {
    ip: Ipv4Addr,
    host: String,
    port: u16,
    path: String,
    scheme: Scheme,
}

impl Endpoint {
    fn same_camera_url(ip: Ipv4Addr, input: &str) -> Result<Self, String> {
        let url = reqwest::Url::parse(input).map_err(|_| "Invalid ONVIF service URL")?;

        let scheme = match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err("ONVIF service must use HTTP or HTTPS".into()),
        };

        if !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(
                "ONVIF service URL cannot contain credentials, queries, or fragments".into(),
            );
        }

        let host = url
            .host_str()
            .ok_or("ONVIF service URL is missing its host")?;

        if host.contains(':')
            || host.len() > 253
            || !host.is_ascii()
            || !host.split('.').all(|label| {
                !label.is_empty()
                    && label.len() <= 63
                    && !label.starts_with('-')
                    && !label.ends_with('-')
                    && label
                        .bytes()
                        .all(|c| c.is_ascii_alphanumeric() || c == b'-')
            })
        {
            return Err("Unsupported ONVIF service hostname".into());
        }

        if let Ok(other_ip) = host.parse::<Ipv4Addr>()
            && other_ip != ip
        {
            return Err("ONVIF XAddr points to a different camera".into());
        }

        let port = url.port_or_known_default().ok_or("Invalid ONVIF port")?;

        if port == 0 {
            return Err("Invalid ONVIF port".into());
        }

        let path = url.path().to_owned();

        if path.len() > 1024
            || !path
                .bytes()
                .all(|c| c.is_ascii_graphic() && c != b'#' && c != b'?')
        {
            return Err("Unsupported ONVIF service path".into());
        }

        Ok(Self {
            ip,
            host: host.to_owned(),
            port,
            path,
            scheme,
        })
    }

    fn url(&self) -> String {
        let scheme = if self.scheme == Scheme::Https {
            "https"
        } else {
            "http"
        };

        format!("{scheme}://{}:{}{}", self.host, self.port, self.path)
    }

    fn client(&self, timeout: Duration) -> Result<reqwest::blocking::Client, String> {
        let forced_ip: SocketAddr = SocketAddrV4::new(self.ip, self.port).into();

        reqwest::blocking::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(timeout)
            .timeout(timeout)
            .resolve(&self.host, forced_ip)
            .tls_backend_rustls()
            .build()
            .map_err(|_| "Could not initialize ONVIF HTTP(S) transport".into())
    }
}

#[derive(Clone, Debug)]
struct Axis {
    min: f32,
    max: f32,
}

impl Axis {
    fn velocity(&self, positive: bool) -> f32 {
        if positive {
            (self.max * 0.35).min(self.max)
        } else {
            (self.min * 0.35).max(self.min)
        }
    }

    fn can_move_both_ways(&self) -> bool {
        self.min.is_finite() && self.max.is_finite() && self.min < 0.0 && self.max > 0.0
    }
}

#[derive(Clone, Debug)]
struct MoveConfig {
    space_uri: String,
    pan: Axis,
    tilt: Axis,
    timeout: Duration,
}

#[derive(Clone, Debug)]
pub struct Target {
    endpoint: Endpoint,
    profile: String,
    movement: MoveConfig,
    credentials: Option<Arc<Credentials>>,
    digest: Arc<Mutex<HashMap<Endpoint, DigestSession>>>,
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
    let (command_tx, command_rx) = mpsc::channel();
    let (event_tx, event_rx) = mpsc::channel();

    thread::spawn(move || worker(command_rx, event_tx));

    (command_tx, event_rx)
}

fn worker(rx: Receiver<Command>, events: Sender<Event>) {
    let mut active: Option<Motion> = None;
    let mut shutting_down = false;

    loop {
        match rx.recv_timeout(Duration::from_millis(35)) {
            Ok(Command::Discover(id, url, credentials)) => {
                let sender = events.clone();

                thread::spawn(move || {
                    let result = discover(&url, credentials);
                    let _ = sender.send(Event::Discovered(id, url, result));
                });
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
                        let _ = stop(&target);
                        let _ = events.send(Event::CommandFailed(id, error));
                    }
                }
            }

            Ok(Command::KeepAlive(id)) => {
                if let Some(motion) = active.as_mut().filter(|m| m.id == id) {
                    motion.heartbeat = Instant::now();
                }
            }

            Ok(Command::Stop(id)) => {
                if active.as_ref().is_some_and(|m| m.id == id) {
                    stop_motion(&mut active);
                }
            }

            Ok(Command::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                stop_motion(&mut active);
                shutting_down = true;
            }

            Err(RecvTimeoutError::Timeout) => {}
        }

        if shutting_down {
            break;
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

pub fn discover(rtsp: &str, credentials: Option<Arc<Credentials>>) -> Result<Target, String> {
    let ip = camera_ip(rtsp)?;
    let mut candidates = discover_endpoints(ip);

    candidates.sort_by_key(|endpoint| endpoint.scheme != Scheme::Https);

    let has_https = candidates.iter().any(|e| e.scheme == Scheme::Https);

    if has_https {
        candidates.retain(|e| e.scheme == Scheme::Https);
    }

    if !has_https {
        for port in [8899, 80, 8000, 8080] {
            let url = format!("http://{ip}:{port}/onvif/device_service");
            let endpoint = Endpoint::same_camera_url(ip, &url)?;

            if !candidates.contains(&endpoint) {
                candidates.push(endpoint);
            }
        }
    }

    let digest = Arc::new(Mutex::new(HashMap::new()));
    let mut last_error = "No accessible ONVIF PTZ service on this camera".to_owned();
    let mut auth_error: Option<String> = None;

    for endpoint in &candidates {
        match probe_device(endpoint, credentials.as_ref(), &digest) {
            Ok(mut target) => {
                target.credentials = credentials;
                return Ok(target);
            }

            Err(error) => {
                if error.to_ascii_lowercase().contains("authenticat")
                    || error.to_ascii_lowercase().contains("credential")
                {
                    auth_error = Some(error);
                } else {
                    last_error = error;
                }
            }
        }
    }

    Err(auth_error.unwrap_or(last_error))
}

fn camera_ip(rtsp: &str) -> Result<Ipv4Addr, String> {
    let host = rtsp
        .strip_prefix("rtsp://")
        .ok_or("Not an RTSP camera")?
        .split(['/', '?', '#'])
        .next()
        .unwrap_or_default();

    let host = host.split(':').next().unwrap_or_default();

    let ip: Ipv4Addr = host
        .parse()
        .map_err(|_| "PTZ discovery requires a camera IPv4 address")?;

    if !(ip.is_private() || ip.is_link_local())
        || ip.is_broadcast()
        || ip.is_unspecified()
        || ip.is_loopback()
    {
        return Err("PTZ discovery is restricted to private LAN camera addresses".into());
    }

    Ok(ip)
}

fn discover_endpoints(camera: Ipv4Addr) -> Vec<Endpoint> {
    let Ok(interfaces) = get_if_addrs() else {
        return Vec::new();
    };

    let mut local = HashSet::new();

    for interface in interfaces {
        let IfAddr::V4(addr) = interface.addr else {
            continue;
        };

        if addr.ip != camera
            && (u32::from(addr.ip) & u32::from(addr.netmask))
                == (u32::from(camera) & u32::from(addr.netmask))
        {
            local.insert(addr.ip);
        }
    }

    let mut results = Vec::new();
    let mut seen = HashSet::new();

    for interface in local.into_iter().take(MAX_DISCOVERY_INTERFACES) {
        let Ok(socket) = UdpSocket::bind((interface, 0)) else {
            continue;
        };

        if socket.set_read_timeout(Some(DISCOVERY_IO)).is_err()
            || socket.set_multicast_ttl_v4(1).is_err()
        {
            continue;
        }

        let mut nonce = [0u8; 16];
        rand::fill(&mut nonce);

        let message_id: String = nonce.iter().map(|byte| format!("{byte:02x}")).collect();

        let probe = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
              <e:Envelope xmlns:e=\"http://www.w3.org/2003/05/soap-envelope\" \
              xmlns:w=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\" \
              xmlns:d=\"http://schemas.xmlsoap.org/ws/2005/04/discovery\" \
              xmlns:dn=\"http://www.onvif.org/ver10/network/wsdl\">\
              <e:Header><w:MessageID>urn:uuid:{message_id}</w:MessageID>\
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

    let Ok(doc) = Document::parse(xml) else {
        return Vec::new();
    };

    let mut endpoints = Vec::new();

    for match_node in doc.descendants().filter(|n| is_tag(*n, "ProbeMatch")) {
        let is_camera = match_node
            .descendants()
            .filter(|n| is_tag(*n, "Types"))
            .filter_map(|n| n.text())
            .any(|types| {
                types
                    .split_whitespace()
                    .any(|item| item.ends_with("NetworkVideoTransmitter"))
            });

        if !is_camera {
            continue;
        }

        for xaddr in match_node
            .descendants()
            .filter(|n| is_tag(*n, "XAddrs"))
            .filter_map(|n| n.text())
            .flat_map(str::split_whitespace)
        {
            if let Ok(endpoint) = Endpoint::same_camera_url(ip, xaddr) {
                endpoints.push(endpoint);
            }
        }
    }

    endpoints
}

fn is_tag(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn child_text(node: Node<'_, '_>, name: &str) -> Option<String> {
    node.children()
        .find(|n| is_tag(*n, name))
        .and_then(|n| n.text())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

fn escape_token(text: &str) -> Result<String, String> {
    if text.is_empty()
        || text.len() > 128
        || !text
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':' | '@'))
    {
        return Err("Invalid ONVIF token".into());
    }

    Ok(text.to_owned())
}

fn probe_device(
    device: &Endpoint,
    credentials: Option<&Arc<Credentials>>,
    digest: &Arc<Mutex<HashMap<Endpoint, DigestSession>>>,
) -> Result<Target, String> {
    let reply = request(
        device,
        "http://www.onvif.org/ver10/device/wsdl/GetCapabilities",
        "<tds:GetCapabilities><tds:Category>All</tds:Category></tds:GetCapabilities>",
        credentials,
        digest,
        PROBE_IO,
    )?;

    let doc = parse_xml(&reply)?;

    let services = doc
        .descendants()
        .find(|n| is_tag(*n, "Capabilities"))
        .ok_or("ONVIF response has no capabilities")?;

    let ptz_url = services
        .children()
        .find(|n| is_tag(*n, "PTZ"))
        .and_then(|n| child_text(n, "XAddr"))
        .ok_or("ONVIF camera does not advertise PTZ")?;

    let media_url = services
        .children()
        .find(|n| is_tag(*n, "Media"))
        .and_then(|n| child_text(n, "XAddr"))
        .ok_or("ONVIF camera does not advertise Media")?;

    let ptz_endpoint = Endpoint::same_camera_url(device.ip, &ptz_url)?;
    let media_endpoint = Endpoint::same_camera_url(device.ip, &media_url)?;

    if device.scheme == Scheme::Https
        && (ptz_endpoint.scheme != Scheme::Https || media_endpoint.scheme != Scheme::Https)
    {
        return Err("Camera advertises insecure Media/PTZ URLs after HTTPS discovery".into());
    }

    let reply = request(
        &media_endpoint,
        "http://www.onvif.org/ver10/media/wsdl/GetProfiles",
        "<trt:GetProfiles/>",
        credentials,
        digest,
        PROBE_IO,
    )?;

    let doc = parse_xml(&reply)?;

    let profile = doc
        .descendants()
        .find(|n| {
            is_tag(*n, "Profiles") && n.children().any(|child| is_tag(child, "PTZConfiguration"))
        })
        .ok_or("No media profile contains a PTZ configuration")?;

    let profile_token = escape_token(
        profile
            .attribute("token")
            .ok_or("Missing PTZ profile token")?,
    )?;

    let config = profile
        .children()
        .find(|n| is_tag(*n, "PTZConfiguration"))
        .ok_or("Missing PTZ configuration")?;

    let config_token = escape_token(
        config
            .attribute("token")
            .ok_or("Missing PTZ configuration token")?,
    )?;

    let body = format!(
        "<tptz:GetConfigurationOptions>\
          <tptz:ConfigurationToken>{config_token}</tptz:ConfigurationToken>\
          </tptz:GetConfigurationOptions>"
    );

    let reply = request(
        &ptz_endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/GetConfigurationOptions",
        &body,
        credentials,
        digest,
        PROBE_IO,
    )?;

    let doc = parse_xml(&reply)?;
    let movement = parse_movement_options(&doc)?;

    let body = format!(
        "<tptz:GetStatus>\
          <tptz:ProfileToken>{profile_token}</tptz:ProfileToken>\
          </tptz:GetStatus>"
    );

    let reply = request(
        &ptz_endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/GetStatus",
        &body,
        credentials,
        digest,
        PROBE_IO,
    )?;

    let doc = parse_xml(&reply)?;

    if !doc.descendants().any(|n| is_tag(n, "GetStatusResponse")) {
        return Err("ONVIF PTZ service did not return GetStatusResponse".into());
    }

    Ok(Target {
        endpoint: ptz_endpoint,
        profile: profile_token,
        movement,
        credentials: None,
        digest: Arc::clone(digest),
    })
}

fn parse_movement_options(doc: &Document<'_>) -> Result<MoveConfig, String> {
    let options = doc
        .descendants()
        .find(|n| is_tag(*n, "GetConfigurationOptionsResponse"))
        .ok_or("Missing ONVIF PTZ configuration options")?;

    let mut compatible_space = None;

    for space in options
        .descendants()
        .filter(|n| is_tag(*n, "ContinuousPanTiltVelocitySpace"))
    {
        let Some(uri) = child_text(space, "URI") else {
            continue;
        };

        let Ok(uri) = escape_token_uri(&uri) else {
            continue;
        };

        let Some(x) = space.children().find(|n| is_tag(*n, "XRange")) else {
            continue;
        };

        let Some(y) = space.children().find(|n| is_tag(*n, "YRange")) else {
            continue;
        };

        let (Ok(pan), Ok(tilt)) = (axis_range(x), axis_range(y)) else {
            continue;
        };

        if pan.can_move_both_ways() && tilt.can_move_both_ways() {
            compatible_space = Some((uri, pan, tilt));
            break;
        }
    }

    let (uri, pan, tilt) = compatible_space
        .ok_or("No ONVIF velocity space supports movement in all four directions")?;

    let timeout = if let Some(range) = options.descendants().find(|n| is_tag(*n, "PTZTimeout")) {
        let min = child_text(range, "Min")
            .and_then(|v| parse_onvif_duration(&v))
            .ok_or("Invalid ONVIF minimum PTZ timeout")?;

        let max = child_text(range, "Max")
            .and_then(|v| parse_onvif_duration(&v))
            .ok_or("Invalid ONVIF maximum PTZ timeout")?;

        if min > max || min > MAX_SAFE_CAMERA_TIMEOUT || max.is_zero() {
            return Err("Camera requires an unsafe continuous movement timeout".into());
        }

        DESIRED_CAMERA_TIMEOUT.max(min).min(max)
    } else {
        DESIRED_CAMERA_TIMEOUT
    };

    Ok(MoveConfig {
        space_uri: uri,
        pan,
        tilt,
        timeout,
    })
}

fn axis_range(node: Node<'_, '_>) -> Result<Axis, String> {
    let min: f32 = child_text(node, "Min")
        .ok_or("Missing PTZ range minimum")?
        .parse()
        .map_err(|_| "Invalid PTZ range minimum")?;

    let max: f32 = child_text(node, "Max")
        .ok_or("Missing PTZ range maximum")?
        .parse()
        .map_err(|_| "Invalid PTZ range maximum")?;

    if !min.is_finite() || !max.is_finite() || min >= max {
        return Err("Invalid ONVIF PTZ velocity range".into());
    }

    Ok(Axis { min, max })
}

fn escape_token_uri(input: &str) -> Result<String, String> {
    if input.is_empty()
        || input.len() > 512
        || input
            .chars()
            .any(|c| c.is_control() || matches!(c, '"' | '<' | '>' | '&'))
    {
        return Err("Invalid ONVIF PTZ velocity URI".into());
    }

    Ok(input.to_owned())
}

fn parse_onvif_duration(text: &str) -> Option<Duration> {
    let value = text.strip_prefix("PT")?;

    let (minutes, seconds) = if let Some((min, sec)) = value.split_once('M') {
        let minutes: f64 = min.parse().ok()?;

        let seconds: f64 = if sec.is_empty() {
            0.0
        } else {
            sec.strip_suffix('S')?.parse().ok()?
        };

        (minutes, seconds)
    } else {
        (0.0, value.strip_suffix('S')?.parse().ok()?)
    };

    let total = minutes * 60.0 + seconds;

    if !total.is_finite() || total < 0.0 || total > 86_400.0 {
        return None;
    }

    Some(Duration::from_secs_f64(total))
}

fn move_camera(target: &Target, direction: Direction) -> Result<(), String> {
    let (x, y) = match direction {
        Direction::Up => (0.0, target.movement.tilt.velocity(true)),
        Direction::Down => (0.0, target.movement.tilt.velocity(false)),
        Direction::Left => (target.movement.pan.velocity(false), 0.0),
        Direction::Right => (target.movement.pan.velocity(true), 0.0),
    };

    let timeout = target.movement.timeout.as_secs_f64();

    let body = format!(
        "<tptz:ContinuousMove>\
          <tptz:ProfileToken>{}</tptz:ProfileToken>\
          <tptz:Velocity>\
          <tt:PanTilt space=\"{}\" x=\"{x}\" y=\"{y}\"/>\
          </tptz:Velocity>\
          <tptz:Timeout>PT{timeout:.3}S</tptz:Timeout>\
          </tptz:ContinuousMove>",
        target.profile, target.movement.space_uri,
    );

    request(
        &target.endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/ContinuousMove",
        &body,
        target.credentials.as_ref(),
        &target.digest,
        CONTROL_IO,
    )
    .map(|_| ())
}

fn stop(target: &Target) -> Result<(), String> {
    let body = format!(
        "<tptz:Stop>\
          <tptz:ProfileToken>{}</tptz:ProfileToken>\
          <tptz:PanTilt>true</tptz:PanTilt>\
          <tptz:Zoom>true</tptz:Zoom>\
          </tptz:Stop>",
        target.profile
    );

    request(
        &target.endpoint,
        "http://www.onvif.org/ver20/ptz/wsdl/Stop",
        &body,
        target.credentials.as_ref(),
        &target.digest,
        CONTROL_IO,
    )
    .map(|_| ())
}

fn parse_xml(xml: &str) -> Result<Document<'_>, String> {
    let doc = Document::parse(xml).map_err(|_| "Invalid ONVIF XML response")?;

    if let Some(fault) = doc.descendants().find(|n| is_tag(*n, "Fault")) {
        let is_auth = fault.descendants().filter_map(|n| n.text()).any(|s| {
            let s = s.to_ascii_lowercase();

            [
                "notauthorized",
                "unauthorized",
                "invalidsecurity",
                "failedauthentication",
            ]
            .iter()
            .any(|needle| s.contains(needle))
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

    let password_digest = STANDARD.encode(digest.finalize());
    let nonce64 = STANDARD.encode(nonce);
    nonce.zeroize();

    let username = xml_escape(&credentials.username);

    format!(
        "<s:Header>\
          <wsse:Security s:mustUnderstand=\"1\" \
          xmlns:wsse=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-secext-1.0.xsd\" \
          xmlns:wsu=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-wssecurity-utility-1.0.xsd\">\
          <wsse:UsernameToken>\
          <wsse:Username>{username}</wsse:Username>\
          <wsse:Password Type=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-username-token-profile-1.0#PasswordDigest\">{password_digest}</wsse:Password>\
          <wsse:Nonce EncodingType=\"http://docs.oasis-open.org/wss/2004/01/oasis-200401-wss-soap-message-security-1.0#Base64Binary\">{nonce64}</wsse:Nonce>\
          <wsu:Created>{created}</wsu:Created>\
          </wsse:UsernameToken>\
          </wsse:Security>\
          </s:Header>"
    )
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[derive(Clone, Debug)]
struct DigestSession {
    challenge: String,
    nonce_count: u32,
}

struct HttpReply {
    code: u16,
    challenges: Vec<String>,
    body: Vec<u8>,
}

fn request(
    endpoint: &Endpoint,
    action: &str,
    body: &str,
    credentials: Option<&Arc<Credentials>>,
    digest_state: &Arc<Mutex<HashMap<Endpoint, DigestSession>>>,
    timeout: Duration,
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

    let cached = if let Some(creds) = credentials {
        let state = digest_state
            .lock()
            .map_err(|_| "ONVIF authentication state unavailable")?;

        state.get(endpoint).and_then(|entry| {
            digest_authorization(
                &entry.challenge,
                endpoint,
                creds,
                entry.nonce_count.saturating_add(1),
            )
            .ok()
        })
    } else {
        None
    };

    let mut reply = http_post(endpoint, action, &envelope, cached.as_deref(), timeout)?;

    if cached.is_some()
        && reply.code != 401
        && let Ok(mut state) = digest_state.lock()
        && let Some(entry) = state.get_mut(endpoint)
    {
        entry.nonce_count = entry.nonce_count.saturating_add(1);
    }

    let mut retries = 0;

    while reply.code == 401 && retries < 2 {
        retries += 1;

        let creds = credentials.ok_or("ONVIF authentication required — use ONVIF sign-in")?;

        let challenge = select_digest_challenge(&reply.challenges)
            .ok_or("ONVIF device requires unsupported HTTP authentication")?;

        if retries > 1 {
            let params = parse_auth_params(challenge.get(7..).unwrap_or(""));

            if !auth_field(&params, "stale").is_some_and(|v| v.eq_ignore_ascii_case("true")) {
                break;
            }
        }

        let authorization = digest_authorization(&challenge, endpoint, creds, 1)?;

        reply = http_post(endpoint, action, &envelope, Some(&authorization), timeout)?;

        let mut state = digest_state
            .lock()
            .map_err(|_| "ONVIF authentication state unavailable")?;

        state.insert(
            endpoint.clone(),
            DigestSession {
                challenge,
                nonce_count: 1,
            },
        );
    }

    if matches!(reply.code, 401 | 403) {
        return Err("ONVIF authentication required or credentials rejected".into());
    }

    let xml = String::from_utf8(reply.body).map_err(|_| "ONVIF response was not UTF-8")?;

    parse_xml(&xml)?;

    if reply.code != 200 {
        return Err(format!("ONVIF returned HTTP {}", reply.code));
    }

    Ok(xml)
}

fn http_post(
    endpoint: &Endpoint,
    action: &str,
    xml: &str,
    auth: Option<&str>,
    timeout: Duration,
) -> Result<HttpReply, String> {
    let client = endpoint.client(timeout)?;

    let mut req = client
        .post(endpoint.url())
        .header(
            reqwest::header::CONTENT_TYPE,
            format!("application/soap+xml; charset=utf-8; action=\"{action}\""),
        )
        .body(xml.to_owned());

    if let Some(header) = auth {
        req = req.header(reqwest::header::AUTHORIZATION, header);
    }

    let response = req.send().map_err(|err| {
        if err.is_timeout() {
            "ONVIF request timed out".to_owned()
        } else if endpoint.scheme == Scheme::Https {
            "ONVIF HTTPS connection failed (check certificate trust and hostname)".to_owned()
        } else {
            "ONVIF camera could not be reached".to_owned()
        }
    })?;

    let code = response.status().as_u16();

    let challenges = response
        .headers()
        .get_all(reqwest::header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok().map(str::to_owned))
        .collect();

    let mut body = Vec::new();

    response
        .take((MAX_REPLY + 1) as u64)
        .read_to_end(&mut body)
        .map_err(|_| "Could not read ONVIF HTTP response")?;

    if body.len() > MAX_REPLY {
        return Err("ONVIF response exceeds size limit".into());
    }

    Ok(HttpReply {
        code,
        challenges,
        body,
    })
}

fn select_digest_challenge(challenges: &[String]) -> Option<String> {
    let candidates = challenges
        .iter()
        .flat_map(|line| split_challenges(line))
        .filter(|challenge| {
            challenge
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("digest ")
        })
        .collect::<Vec<_>>();

    for algorithm in ["SHA-256", "SHA-256-sess", "MD5", "MD5-sess"] {
        if let Some(value) = candidates.iter().find(|s| {
            let fields = parse_auth_params(s.trim_start().get(7..).unwrap_or(""));
            let qop = auth_field(&fields, "qop").unwrap_or("");

            (qop.is_empty()
                || qop
                    .split(',')
                    .any(|item| item.trim().eq_ignore_ascii_case("auth")))
                && auth_field(&fields, "algorithm")
                    .unwrap_or("MD5")
                    .eq_ignore_ascii_case(algorithm)
        }) {
            return Some((*value).to_string());
        }
    }

    None
}

fn split_challenges(line: &str) -> Vec<&str> {
    let mut chunks = Vec::new();
    let mut in_quotes = false;
    let mut escaped = false;
    let mut start = 0;

    for (i, ch) in line.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }

        if ch == '\\' && in_quotes {
            escaped = true;
            continue;
        }

        if ch == '"' {
            in_quotes = !in_quotes;
            continue;
        }

        if ch == ',' && !in_quotes {
            let rest = line[i + 1..].trim_start();

            if rest.to_ascii_lowercase().starts_with("digest ")
                || rest.to_ascii_lowercase().starts_with("basic ")
            {
                chunks.push(line[start..i].trim());
                start = i + 1;
            }
        }
    }

    chunks.push(line[start..].trim());
    chunks
}

fn digest_authorization(
    challenge: &str,
    endpoint: &Endpoint,
    credentials: &Credentials,
    nc: u32,
) -> Result<String, String> {
    let start = challenge.trim_start();

    let fields = parse_auth_params(
        start
            .get(7..)
            .filter(|_| start[..7].eq_ignore_ascii_case("Digest "))
            .ok_or("Unsupported HTTP Digest challenge")?,
    );

    let realm = auth_field(&fields, "realm").ok_or("HTTP Digest realm missing")?;
    let nonce = auth_field(&fields, "nonce").ok_or("HTTP Digest nonce missing")?;
    let algorithm = auth_field(&fields, "algorithm").unwrap_or("MD5");

    if !["MD5", "MD5-sess", "SHA-256", "SHA-256-sess"]
        .iter()
        .any(|a| algorithm.eq_ignore_ascii_case(a))
    {
        return Err("Unsupported HTTP Digest algorithm".into());
    }

    let qop = auth_field(&fields, "qop").unwrap_or("");
    let has_qop = !qop.is_empty();

    if has_qop
        && !qop
            .split(',')
            .any(|q| q.trim().eq_ignore_ascii_case("auth"))
    {
        return Err("ONVIF HTTP Digest requires unsupported qop".into());
    }

    let is_sess = algorithm.to_ascii_lowercase().ends_with("-sess");

    let base_algorithm = if algorithm.to_ascii_uppercase().starts_with("SHA-256") {
        "SHA-256"
    } else {
        "MD5"
    };

    let mut cnonce_bytes = [0u8; 12];
    rand::fill(&mut cnonce_bytes);

    let cnonce: String = cnonce_bytes.iter().map(|b| format!("{b:02x}")).collect();

    cnonce_bytes.zeroize();

    let response = digest_response(DigestResponseParams {
        algorithm: base_algorithm,
        sess: is_sess,
        username: &credentials.username,
        password: &credentials.password,
        realm,
        nonce,
        method: "POST",
        uri: &endpoint.path,
        nc,
        cnonce: &cnonce,
        qop: has_qop,
    });

    let mut header = format!(
        "Digest username=\"{}\", realm=\"{}\", nonce=\"{}\", uri=\"{}\", response=\"{response}\", algorithm={algorithm}",
        quote_header(&credentials.username),
        quote_header(realm),
        quote_header(nonce),
        quote_header(&endpoint.path),
    );

    if has_qop {
        header.push_str(&format!(", qop=auth, nc={nc:08x}, cnonce=\"{cnonce}\""));
    }

    if is_sess && !has_qop {
        header.push_str(&format!(", cnonce=\"{cnonce}\""));
    }

    if let Some(opaque) = auth_field(&fields, "opaque") {
        header.push_str(&format!(", opaque=\"{}\"", quote_header(opaque)));
    }

    Ok(header)
}

#[derive(Clone, Copy)]
struct DigestResponseParams<'a> {
    algorithm: &'a str,
    sess: bool,
    username: &'a str,
    password: &'a str,
    realm: &'a str,
    nonce: &'a str,
    method: &'a str,
    uri: &'a str,
    nc: u32,
    cnonce: &'a str,
    qop: bool,
}

fn digest_response(params: DigestResponseParams<'_>) -> String {
    let DigestResponseParams {
        algorithm,
        sess,
        username,
        password,
        realm,
        nonce,
        method,
        uri,
        nc,
        cnonce,
        qop,
    } = params;

    let mut a1_input = format!("{username}:{realm}:{password}");
    let mut ha1 = digest_hash(algorithm, &a1_input);

    a1_input.zeroize();

    if sess {
        ha1 = digest_hash(algorithm, &format!("{ha1}:{nonce}:{cnonce}"));
    }

    let ha2 = digest_hash(algorithm, &format!("{method}:{uri}"));

    let response = if qop {
        digest_hash(
            algorithm,
            &format!("{ha1}:{nonce}:{nc:08x}:{cnonce}:auth:{ha2}"),
        )
    } else {
        digest_hash(algorithm, &format!("{ha1}:{nonce}:{ha2}"))
    };

    ha1.zeroize();
    response
}

fn digest_hash(algorithm: &str, input: &str) -> String {
    let bytes = if algorithm == "SHA-256" {
        Sha256::digest(input.as_bytes()).to_vec()
    } else {
        Md5::digest(input.as_bytes()).to_vec()
    };

    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn quote_header(s: &str) -> String {
    s.replace(['\r', '\n'], "")
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
}

fn auth_field<'a>(fields: &'a [(String, String)], name: &str) -> Option<&'a str> {
    fields
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

fn parse_auth_params(text: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    let mut it = text.char_indices().peekable();

    while let Some((_, ch)) = it.peek().copied() {
        if ch == ',' || ch.is_ascii_whitespace() {
            it.next();
        } else {
            break;
        }
    }

    while it.peek().is_some() {
        let mut key = String::new();

        while let Some((_, ch)) = it.peek().copied() {
            if ch == '=' || ch == ',' {
                break;
            }

            key.push(ch);
            it.next();
        }

        let key = key.trim().to_owned();

        if it.next().is_none_or(|(_, ch)| ch != '=') {
            break;
        }

        let value = if it.peek().is_some_and(|(_, ch)| *ch == '"') {
            it.next();

            let mut s = String::new();
            let mut escaped = false;

            for (_, c) in it.by_ref() {
                if escaped {
                    s.push(c);
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    break;
                } else {
                    s.push(c);
                }
            }

            s
        } else {
            let mut s = String::new();

            while let Some((_, ch)) = it.peek().copied() {
                if ch == ',' {
                    break;
                }

                s.push(ch);
                it.next();
            }

            s.trim().to_owned()
        };

        if !key.is_empty() {
            result.push((key, value));
        }

        while let Some((_, ch)) = it.peek().copied() {
            if ch == ',' || ch.is_ascii_whitespace() {
                it.next();
            } else {
                break;
            }
        }
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn endpoint() -> Endpoint {
        Endpoint::same_camera_url(
            Ipv4Addr::new(192, 168, 1, 20),
            "http://192.168.1.20:8899/onvif/device_service",
        )
        .unwrap()
    }

    #[test]
    fn reject_other_host_but_allow_pinned_hostname_and_https() {
        let ip = Ipv4Addr::new(192, 168, 1, 20);

        assert!(Endpoint::same_camera_url(ip, "http://192.168.1.21/onvif").is_err());

        assert!(Endpoint::same_camera_url(ip, "https://camera.home.arpa:8443/onvif").is_ok());

        assert!(
            Endpoint::same_camera_url(ip, "http://camera.home.arpa@192.168.1.20/onvif").is_err()
        );

        assert!(Endpoint::same_camera_url(ip, "http://192.168.1.20/onvif?next=host").is_err());
    }

    #[test]
    fn reject_public_camera() {
        assert!(camera_ip("rtsp://8.8.8.8/live").is_err());
        assert!(camera_ip("rtsp://192.168.1.20/live").is_ok());
    }

    #[test]
    fn parse_movement_limits_and_reject_long_minimum_timeout() {
        let text = "<GetConfigurationOptionsResponse><PTZConfigurationOptions><Spaces>\
             <ContinuousPanTiltVelocitySpace><URI>http://www.onvif.org/ver10/tptz/PanTiltSpaces/VelocityGenericSpace</URI>\
             <XRange><Min>-1</Min><Max>1</Max></XRange><YRange><Min>-1</Min><Max>1</Max></YRange>\
             </ContinuousPanTiltVelocitySpace></Spaces><PTZTimeout><Min>PT0.2S</Min><Max>PT1S</Max></PTZTimeout>\
             </PTZConfigurationOptions></GetConfigurationOptionsResponse>";

        let doc = Document::parse(text).unwrap();

        assert_eq!(
            parse_movement_options(&doc).unwrap().timeout,
            DESIRED_CAMERA_TIMEOUT
        );

        let unsafe_xml = text.replace("PT0.2S", "PT3S");

        assert!(parse_movement_options(&Document::parse(&unsafe_xml).unwrap()).is_err());
    }

    #[test]
    fn digest_rfc_7616_algorithm_and_qop() {
        let credentials = Credentials::new("operator".into(), "secret".into()).unwrap();

        let header = digest_authorization(
            "Digest realm=\"cámara\", nonce=\"n\", qop=\"auth,auth-int\", algorithm=SHA-256",
            &endpoint(),
            &credentials,
            2,
        )
        .unwrap();

        assert!(header.contains("algorithm=SHA-256"));
        assert!(header.contains("nc=00000002"));
        assert!(!header.contains("secret"));

        assert!(
            digest_authorization(
                "Digest realm=\"x\", nonce=\"n\", qop=\"auth-int\"",
                &endpoint(),
                &credentials,
                1,
            )
            .is_err()
        );
    }

    #[test]
    fn digest_matches_rfc_2617_md5_reference_vector() {
        let response = digest_response(DigestResponseParams {
            algorithm: "MD5",
            sess: false,
            username: "Mufasa",
            password: "Circle Of Life",
            realm: "testrealm@host.com",
            nonce: "dcd98b7102dd2f0e8b11d0f600bfb0c093",
            method: "GET",
            uri: "/dir/index.html",
            nc: 1,
            cnonce: "0a4f113b",
            qop: true,
        });

        assert_eq!(response, "6629fae49393a05397450978507c4ef1");
    }

    #[test]
    fn digest_sha256_is_not_the_md5_digest() {
        let params = DigestResponseParams {
            algorithm: "SHA-256",
            sess: false,
            username: "user",
            password: "pass",
            realm: "realm",
            nonce: "nonce",
            method: "POST",
            uri: "/onvif",
            nc: 1,
            cnonce: "c0ffee",
            qop: true,
        };

        let sha = digest_response(params);

        let md5 = digest_response(DigestResponseParams {
            algorithm: "MD5",
            ..params
        });

        assert_eq!(sha.len(), 64);
        assert_eq!(md5.len(), 32);
        assert_ne!(sha, md5);
    }

    #[test]
    fn multiple_challenges_select_sha256() {
        let challenges = [
            "Basic realm=\"x\", Digest realm=\"x\",nonce=\"y\",algorithm=MD5".to_owned(),
            "Digest realm=\"x\",nonce=\"z\",algorithm=SHA-256".to_owned(),
        ];

        assert!(
            select_digest_challenge(&challenges)
                .unwrap()
                .contains("SHA-256")
        );
    }
}
